#!/usr/bin/env python3
"""gen_sign_driver.py v3 — BIP-340 sign driver on top of sign_lib.lisp.

Core signature convention: (fe-mul a0..a8 b) = 9 limbs + ONE vector.
Wrappers take vector A + vector b, unpack only A.
Output: /tmp/nostr_probe/sign_driver.lisp
"""
import hashlib

LIMB, NL, M = 30, 9, (1 << 30) - 1
p = 2**256 - 2**32 - 977

def limbs(x):
    return [(x >> (LIMB * i)) & M for i in range(NL)]

def lisp_list(vals):
    return "(list " + " ".join(str(v) for v in vals) + ")"

L = []
def w(s=""):
    L.append(s)

# ── vector wrappers: (f A b), A vector via vec-nth, b passed straight through ──
for name, core in [("fm", "fe-mul"), ("fa", "fe-add"), ("fs", "fe-sub")]:
    w("(define (%s A b) (%s %s b))" % (name, core,
        " ".join("(vec-nth A %d)" % i for i in range(NL))))
w("(define (fz A) (fe-zero? %s))" % " ".join("(vec-nth A %d)" % i for i in range(NL)))
w("(define (fmn A b) (fe-muln %s b))" % " ".join("(vec-nth A %d)" % i for i in range(NL)))
w()

# ── constants ──
w("(define (c-pm2 _d) %s)" % lisp_list(limbs(p - 2)))
w()

# ── words / hex helpers ──
w("(define (word-at s j)")
w("  (let* ((b0 (byte-at s (* 4 j)))")
w("         (b1 (byte-at s (+ (* 4 j) 1)))")
w("         (b2 (byte-at s (+ (* 4 j) 2)))")
w("         (b3 (byte-at s (+ (* 4 j) 3))))")
w("    (+ (+ (+ (* b0 16777216) (* b1 65536)) (* b2 256)) b3)))")
w()
w('(define (nib i) (str-substring "0123456789abcdef" i (+ i 1)))')
w("(define (hex8 x)")
w("  (str-cat (nib (band (shr x 28) 15)) (nib (band (shr x 24) 15))")
w("           (nib (band (shr x 20) 15)) (nib (band (shr x 16) 15))")
w("           (nib (band (shr x 12) 15)) (nib (band (shr x 8) 15))")
w("           (nib (band (shr x 4) 15)) (nib (band x 15))))")
w("(define (words-hex wv)")
w("  (str-cat (hex8 (vec-nth wv 0)) (hex8 (vec-nth wv 1)) (hex8 (vec-nth wv 2)) (hex8 (vec-nth wv 3))")
w("           (hex8 (vec-nth wv 4)) (hex8 (vec-nth wv 5)) (hex8 (vec-nth wv 6)) (hex8 (vec-nth wv 7))))")
w()

# words->limbs: BE words -> LE 30-bit limbs (explicit per-limb emission)
def limb_terms(k):
    terms = []
    for wi in range(8):
        lo = 32 * (7 - wi)
        if lo < 30 * k + LIMB and lo + 32 > 30 * k:
            shift = 30 * k - lo
            if shift == 0:
                terms.append("(band w%d %d)" % (wi, M))
            elif 0 < shift:
                # right-shift: word's high bits land at limb bottom; mask to LIMB
                # (handles shift>=30 too — e.g. w7's top 2 bits into limb 1)
                terms.append("(band (shr w%d %d) %d)" % (wi, shift, M))
            else:
                # left-shift: pre-mask kept bits (<=2^(LIMB-off)), then positive
                # count (<30) — compiled wasm traps on i64 overflow & masks
                # negative counts mod 64
                off = -shift
                terms.append("(shl (band w%d %d) %d)" % (wi, (1 << (LIMB - off)) - 1, off))
    return terms

w("(define (words->limbs w0 w1 w2 w3 w4 w5 w6 w7)")
w("  (list %s))" % " ".join(
    "(+ " + " ".join(limb_terms(k)) + ")" if len(limb_terms(k)) > 1 else limb_terms(k)[0]
    for k in range(NL)))
w()

# ── mod-p exponentiation (montgomery in/out) ──
w("(define (fe-pow x ewords)")
w("  (loop ((acc (c-onem 0)) (i 255))")
w("    (let* ((a2 (fm acc acc))")
w("           (bit (band (shr (vec-nth ewords (/ i 30)) (mod i 30)) 1))")
w("           (with (fm a2 x)))")
w("      (if (< i 1)")
w("          (if (= bit 1) with a2)")
w("          (recur (if (= bit 1) with a2) (- i 1))))))")
w()

# ── point double (Jacobian, a=0) — VERIFIED vs python EC (dbl-2009-l):
# A=X², B=Y², C=B², t=X+B, D=2(t²-A-C)=4XY², E=3A, F=E²,
# X3=F-2D, Y3=E(D-X3)-8C, Z3=2YZ  ──
w("(define (pt-dbl X Y Z)")
w("  (let* ((A (fm X X))")
w("         (B (fm Y Y))")
w("         (C (fm B B))")
w("         (T (fa X B))")
w("         (D1 (fs (fs (fm T T) A) C))")
w("         (D (fa D1 D1))")
w("         (E (fa A (fa A A)))")
w("         (F (fm E E))")
w("         (C2 (fa C C))")
w("         (C4 (fa C2 C2))")
w("         (C8 (fa C4 C4))")
w("         (X3 (fs F (fa D D)))")
w("         (YZ (fm Y Z))")
w("         (Y3 (fs (fm E (fs D X3)) C8))")
w("         (Z3 (fa YZ YZ)))")
w("    (list X3 Y3 Z3)))")
w()

# ── mixed add: Jacobian + affine (G); acc=inf -> G; H=0 -> inf ──
w("(define (pt-add-aff X1 Y1 Z1)")
w("  (if (= (fz Z1) 1)")
w("      (list (c-gxm 0) (c-gym 0) (c-onem 0))")
w("      (let* ((ZZ (fm Z1 Z1))")
w("             (U2 (fm (c-gxm 0) ZZ))")
w("             (S2 (fm (c-gym 0) (fm ZZ Z1)))")
w("             (H (fs U2 X1))")
w("             (R1 (fs S2 Y1)))")
w("        (if (= (fz H) 1)")
w("            (list (c-zero 0) (c-zero 0) (c-zero 0))")
w("            (let* ((HH (fm H H))")
w("                   (HHH (fm HH H))")
w("                   (V (fm X1 HH))")
w("                   (RR (fa R1 R1))")
w("                   (X3 (fs (fs (fm RR RR) HHH) (fa V V)))")
w("                   (Y3 (fs (fm RR (fs V X3)) (fm S2 HHH)))")
w("                   (Z3 (fm Z1 H)))")
w("              (list X3 Y3 Z3))))))")
w()

# ── scalar mult k*G ──
w("(define (sc-mul-g k)")
w("  (loop ((X (c-zero 0)) (Y (c-zero 0)) (Z (c-zero 0)) (i 255))")
w("    (let* ((dbl (pt-dbl X Y Z))")
w("           (bit (band (shr (vec-nth k (/ i 30)) (mod i 30)) 1))")
w("           (sum (if (= bit 1)")
w("                    (pt-add-aff (vec-nth dbl 0) (vec-nth dbl 1) (vec-nth dbl 2))")
w("                    dbl)))")
w("      (if (< i 1)")
w("          sum")
w("          (recur (vec-nth sum 0) (vec-nth sum 1) (vec-nth sum 2) (- i 1))))))")
w()
w("(define (aff-x X Y Z) (let ((Zi (fe-pow Z (c-pm2 0)))) (fm X (fm Zi Zi))))")
w()
# fe-ser: serialize a Montgomery-domain field element to 32 BE bytes.
# multiply by PLAIN 1 (limbs of 1, NOT c-onem which is R) strips the R factor:
# fe-mul(v*R, 1) = v. Then fe-words-be emits the true 32 BE bytes.
w("(define (c-onep _d) (list 1 0 0 0 0 0 0 0 0))")
w("(define (fe-ser v) (words-hex (fe-words-be (fm v (c-onep 0)))))")
w()

open("/tmp/nostr_probe/sign_driver.lisp", "w").write("\n".join(x for x in L if x) + "\n")
print("driver v3 emitted OK")
