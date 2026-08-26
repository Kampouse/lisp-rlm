#!/usr/bin/env python3
"""gen_bip340.py — FINAL. Emits bip340.lisp: BIP-340 Schnorr verify in raw lisp-rlm.
Compiler constraints honored (see git log / session notes):
  tagged Nums < 2^61, comparisons on non-negatives only, no nested loops,
  no zero-arg fns, no array params from sibling call results (limb args instead),
  loop/recur value-args-first, arithmetic xor32, Montgomery with plain-p reduction.
"""
import csv, hashlib

LIMB, NL, M = 30, 9, (1 << 30) - 1
MASK32 = 0xFFFFFFFF

p = 2**256 - 2**32 - 977
n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8
R = 1 << 270
R2 = (R * R) % p
R1 = R % p
seven_m = (7 * R) % p
pp270 = (-pow(p, -1, 1 << 270)) % (1 << 270)
d1 = (p + 1) // 4

def limbs(x, nl=NL):
    return [(x >> (LIMB * i)) & M for i in range(nl)]

def lisp_list(vals):
    return "(list " + " ".join(str(v) for v in vals) + ")"

L = []
def w(s=""):
    L.append(s)

def fe2(fn, a, b):
    return "(%s %s %s)" % (fn,
        " ".join("(vec-nth %s %d)" % (a, i) for i in range(NL)), b)

def fe1(fn, a):
    return "(%s %s)" % (fn, " ".join("(vec-nth %s %d)" % (a, i) for i in range(NL)))

def fe2limbs(fn, a_lims, b):
    """first operand already limb names"""
    return "(%s %s %s)" % (fn, " ".join(a_lims),
        " ".join("(vec-nth %s %d)" % (b, i) for i in range(NL)))

def seqlets(binds, final):
    out = []
    for name, expr in binds:
        out.append("(let ((%s %s))" % (name, expr))
    out.append(final)
    out.append(")" * len(binds))
    return "\n".join(out)

# ═══ constants ═══
w(";; bip340.lisp — BIP-340 Schnorr verification (raw lisp-rlm, generated)")
w("(define (c-p _d) %s)" % lisp_list(limbs(p)))
w("(define (c-n _d) %s)" % lisp_list(limbs(n)))
w("(define (c-r2 _d) %s)" % lisp_list(limbs(R2)))
w("(define (c-onem _d) %s)" % lisp_list(limbs(R1)))
w("(define (c-pp _d) %s)" % lisp_list(limbs(pp270)))
w("(define (c-sevenm _d) %s)" % lisp_list(limbs(seven_m)))
w("(define (c-gx _d) %s)" % lisp_list(limbs((Gx * R) % p)))
w("(define (c-gy _d) %s)" % lisp_list(limbs((Gy * R) % p)))
w("(define (c-d1 _d) %s)" % lisp_list(limbs(d1)))
w("(define (c-zero _d) %s)" % lisp_list([0] * NL))
TAG = hashlib.sha256(b"BIP0340/challenge").digest()
tag_words = [int.from_bytes(TAG[i*4:(i+1)*4], "big") for i in range(8)]
w("(define (c-tagw _d) %s)" % lisp_list(tag_words))
w()

def geq_chain(aget, bget, top):
    expr = "0"
    for k in range(top, -1, -1):
        expr = "(if (> %s %s) 1 (if (< %s %s) 0 %s))" % (aget(k), bget(k), aget(k), bget(k), expr)
    return expr

def geq_chain_msb(aget, bget, top):
    # MSB-first magnitude compare, equal counts as geq (returns 1)
    expr = "1"
    for k in range(0, top + 1):
        expr = "(if (> %s %s) 1 (if (< %s %s) 0 %s))" % (aget(k), bget(k), aget(k), bget(k), expr)
    return expr

def cond_sub_p_stmts(aget, dst):
    stmts = ["(set! g %s)" % geq_chain(aget, lambda k: "(vec-nth (c-p 0) %d)" % k, 8)]  # top=8: 9 limbs (0..8) — 9 reads OOB!
    stmts.append("(if (= g 1)")
    stmts.append("  (begin")
    stmts.append("    (set! %sbr 0)" % dst)
    for k in range(10):
        stmts.append("    (set! %sv (+ (- (- %s (vec-nth (c-p 0) %d)) %sbr) 1073741824))" % (dst, aget(k), k, dst))
        stmts.append("    (set! %shi (shr %sv 30))" % (dst, dst))
        stmts.append("    (set! %sbr (if (= %shi 0) 1 0))" % (dst, dst))
        stmts.append("    (set! %s%d (if (= %sbr 1) %sv (- %sv 1073741824)))" % (dst, k, dst, dst, dst))
    stmts.append("    0)")
    stmts.append("  0)")
    for k in range(10):
        stmts.append("(set! %s%d (if (= g 1) %s%d %s))" % (dst, k, dst, k, aget(k)))
    return stmts

# ═══ fe-mul (REDC) ═══
w(";; fe-mul — Montgomery REDC (x as limbs, y as array)")
lines = ["(define (fe-mul x0 x1 x2 x3 x4 x5 x6 x7 x8 y)"]
params = ["(t%d 0)" % i for i in range(19)] + ["(m%d 0)" % i for i in range(NL)]
params += ["(v 0)", "(c 0)", "(g 0)", "(dbr 0)", "(dhi 0)", "(dv 0)"] + \
          ["(d%d 0)" % k for k in range(10)]
lines.append("  (let (" + " ".join(params) + ")")
lines.append("    (begin")
body = []
for i in range(NL):
    body.append("(set! c 0)")
    for j in range(NL):
        body.append("(set! v (+ (+ t%d (wrap-mul x%d (vec-nth y %d))) c))" % (i + j, i, j))
        body.append("(set! t%d (band v %d))" % (i + j, M))
        body.append("(set! c (shr v %d))" % LIMB)
    body.append("(if (!= c 0) (begin (set! v (+ t%d c)) (set! t%d (band v %d)) 0) 0)" % (i + NL, i + NL, M))
for i in range(NL):
    body.append("(set! c 0)")
    for j in range(NL - i):
        body.append("(set! v (+ (+ m%d (wrap-mul t%d (vec-nth (c-pp 0) %d))) c))" % (i + j, i, j))
        body.append("(set! m%d (band v %d))" % (i + j, M))
        body.append("(set! c (shr v %d))" % LIMB)
for i in range(NL):
    body.append("(set! c 0)")
    for j in range(NL):
        body.append("(set! v (+ (+ t%d (wrap-mul m%d (vec-nth (c-p 0) %d))) c))" % (i + j, i, j))
        body.append("(set! t%d (band v %d))" % (i + j, M))
        body.append("(set! c (shr v %d))" % LIMB)
    body.append("(if (!= c 0) (begin (set! v (+ t%d c)) (set! t%d (band v %d)) 0) 0)" % (i + NL, i + NL, M))
for rnd in range(2):
    aget = (lambda k: ("t%d" % (9 + k))) if rnd == 0 else (lambda k: "d%d" % k)
    body.extend(cond_sub_p_stmts(aget, "d"))
body.append("(list d0 d1 d2 d3 d4 d5 d6 d7 d8)")
lines.extend("    " + b for b in body)
lines.append(")))")
w("\n".join(lines))
w()

# ═══ fe-add / fe-sub ═══
def emit_addsub(name, is_add):
    lines = ["(define (%s a0 a1 a2 a3 a4 a5 a6 a7 a8 b)" % name]
    params = ["(v 0)", "(c 0)", "(br 0)", "(hi 0)", "(g 0)", "(dbr 0)", "(dhi 0)", "(dv 0)"]
    if not is_add:
        params += ["(t%d 0)" % i for i in range(10)]
    params += ["(d%d 0)" % k for k in range(10)]
    lines.append("  (let (" + " ".join(params) + ")")
    lines.append("    (begin")
    lines.append("    (set! c 0)")
    for i in range(NL):
        src_b = "(vec-nth b %d)" % i if is_add else "(vec-nth (c-p 0) %d)" % i
        lines.append("    (set! v (+ (+ a%d %s) c))" % (i, src_b))
        lines.append("    (set! d%d (band v %d))" % (i, M))
        lines.append("    (set! c (shr v %d))" % LIMB)
    if is_add:
        lines.append("    (set! d9 c)")
    else:
        lines.append("    (set! t9 c)")
        lines.append("    (set! br 0)")
        for i in range(NL):
            lines.append("    (set! v (+ (- (- t%d (vec-nth b %d)) br) 1073741824))" % (i, i))
            lines.append("    (set! hi (shr v 30))")
            lines.append("    (set! br (if (= hi 0) 1 0))")
            lines.append("    (set! d%d (if (= br 1) v (- v 1073741824)))" % i)
        lines.append("    (set! d9 (- t9 br))")
    lines.extend("    " + s for s in cond_sub_p_stmts(lambda k: "d%d" % k, "d"))
    lines.append("    (list d0 d1 d2 d3 d4 d5 d6 d7 d8)")
    lines.append(")))")
    w("\n".join(lines))
    w()

w(";; fe-add / fe-sub mod p")
emit_addsub("fe-add", True)
emit_addsub("fe-sub", False)

# ═══ fe-eq / fe-zero? ═══
expr = "(= a8 b8)"
for i in range(NL - 2, -1, -1):
    expr = "(and (= a%d b%d) %s)" % (i, i, expr)
bexpr = expr
for i in range(NL):
    bexpr = bexpr.replace("(= a%d b%d)" % (i, i), "(= a%d (vec-nth b %d))" % (i, i))
w("(define (fe-eq a0 a1 a2 a3 a4 a5 a6 a7 a8 b)")
w("  (if %s 1 0))" % bexpr)
zexpr = "(= a8 0)"
for i in range(NL - 2, -1, -1):
    zexpr = "(and (= a%d 0) %s)" % (i, zexpr)
w("(define (fe-zero? a0 a1 a2 a3 a4 a5 a6 a7 a8)")
w("  (if %s 1 0))" % zexpr)
w()

# ═══ scalar ops ═══
def emit_geq_fn(name, cfn):
    w("(define (%s a0 a1 a2 a3 a4 a5 a6 a7 a8)" % name)
    w("  %s)" % geq_chain_msb(lambda k: "a%d" % k, lambda k: "(vec-nth (%s 0) %d)" % (cfn, k), 8))
    w()
emit_geq_fn("sc-geq-n", "c-n")
emit_geq_fn("sc-geq-p", "c-p")

def emit_scop(name, mode):
    lines = ["(define (%s a0 a1 a2 a3 a4 a5 a6 a7 a8)" % name]
    params = ["(v 0)", "(hi 0)", "(br 0)", "(g 0)"] + ["(d%d 0)" % i for i in range(NL)]
    lines.append("  (let (" + " ".join(params) + ")")
    lines.append("    (begin")
    if mode == "reduce":
        lines.append("    (set! g %s)" % geq_chain_msb(lambda k: "a%d" % k, lambda k: "(vec-nth (c-n 0) %d)" % k, 8))
        lines.append("    (if (= g 1)")
        lines.append("      (begin")
    lines.append("      (set! br 0)")
    for i in range(NL):
        A = "a%d" % i
        B = "(vec-nth (c-n 0) %d)" % i
        if mode == "reduce":
            srcA, srcB = A, B
        else:  # n - a
            srcA, srcB = B, A
        lines.append("      (set! v (+ (- (- %s %s) br) 1073741824))" % (srcA, srcB))
        lines.append("      (set! hi (shr v 30))")
        lines.append("      (set! br (if (= hi 0) 1 0))")
        lines.append("      (set! d%d (if (= br 1) v (- v 1073741824)))" % i)
    if mode == "reduce":
        lines.append("      0)")
        lines.append("      0)")
        for i in range(NL):
            lines.append("    (set! d%d (if (= g 1) d%d a%d))" % (i, i, i))
    lines.append("    (list d0 d1 d2 d3 d4 d5 d6 d7 d8)")
    lines.append(")))")
    w("\n".join(lines))
    w()
emit_scop("sc-reduce", "reduce")
emit_scop("sc-n-minus", "nminus")

# ═══ fe-sqrt ═══
w("(define (fe-sqrt x0 x1 x2 x3 x4 x5 x6 x7 x8)")
w("  (let ((x (list x0 x1 x2 x3 x4 x5 x6 x7 x8)))")
w("    (loop ((acc (c-onem 0)) (i 254))")
w("      (let ((a2 %s))" % fe2("fe-mul", "acc", "acc"))
w("        (let ((a3 (if (= (band (shr (vec-nth (c-d1 0) (/ i 30)) (mod i 30)) 1) 1)")
w("                      %s a2)))" % fe2("fe-mul", "a2", "x"))
w("          (if (= i 0) a3 (recur a3 (- i 1))))))))")
w()

# ═══ conversions ═══
def word_expr(k, prefix):
    parts = []
    for i in range(NL):
        shift = 32 * k - 30 * i
        if shift == 0:
            parts.append("(band %s%d 4294967295)" % (prefix, i))
        elif 0 < shift < 30:
            parts.append("(shr %s%d %d)" % (prefix, i, shift))
        elif -32 < shift < 0:
            parts.append("(band (shl %s%d %d) 4294967295)" % (prefix, i, -shift))
    return parts[0] if len(parts) == 1 else "(band (bor %s) 4294967295)" % " ".join(parts)

w("(define (fe-words-be a)")
w("  (let (%s)" % " ".join("(a%d (vec-nth a %d))" % (i, i) for i in range(NL)))
w("    (list %s)))" % " ".join(word_expr(k, "a") for k in range(7, -1, -1)))
w()

def limb_from_words(k):
    parts = []
    lk = 30 * k
    for wi in range(8):
        lo = 32 * (7 - wi)
        if lo < lk + LIMB and lo + 32 > lk:
            shift = lk - lo
            if shift == 0:
                parts.append("(band w%d %d)" % (wi, M))
            elif shift > 0:
                parts.append("(shr w%d %d)" % (wi, shift))
            else:
                parts.append("(band (shl w%d %d) %d)" % (wi, -shift, M))
    return parts[0] if len(parts) == 1 else "(bor %s)" % " ".join(parts)

w("(define (words-limbs w0 w1 w2 w3 w4 w5 w6 w7)")
w("  (list %s))" % " ".join(limb_from_words(k) for k in range(NL)))
w()

# ═══ SHA-256 flat ═══
K = [
0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2]
H0 = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]

def rotr(e, k):
    return "(band (bor (shr %s %d) (shl %s %d)) 4294967295)" % (e, k, e, 32 - k)
def xor32(a, b):
    return "(band (- (+ %s %s) (wrap-mul (band %s %s) 2)) 4294967295)" % (a, b, a, b)
def sig0(x): return xor32(xor32(rotr(x, 7), rotr(x, 18)), "(shr %s 3)" % x)
def sig1(x): return xor32(xor32(rotr(x, 17), rotr(x, 19)), "(shr %s 10)" % x)
def bigS0(x): return xor32(xor32(rotr(x, 2), rotr(x, 13)), rotr(x, 22))
def bigS1(x): return xor32(xor32(rotr(x, 6), rotr(x, 11)), rotr(x, 25))

w("(define (sha-k _d) %s)" % lisp_list(K))
w()

def emit_block(b):
    base = 16 * b
    lines = ["(define (sha-blk%d W h0 h1 h2 h3 h4 h5 h6 h7)" % b]
    binds = " ".join("(%s h%d)" % (v, i) for i, v in enumerate("abcdefgh"))
    lines.append("  (let ((%s) (t1 0) (t2 0) (Wx (list)))" % binds)
    lines.append("    (begin")
    lines.append("    (set! Wx (list %s))" % " ".join("(vec-nth W %d)" % (base + i) for i in range(16)))
    for r in range(64):
        if r >= 16:
            wnext = "(band (+ (+ %s (vec-nth Wx %d)) (+ %s (vec-nth Wx %d))) 4294967295)" % (
                sig1("(vec-nth Wx %d)" % (r - 2)), r - 7,
                sig0("(vec-nth Wx %d)" % (r - 15)), r - 16)
            lines.append("    (set! Wx (vec-push Wx %s))" % wnext)
        chf = xor32("(band e f)", "(- g (band e g))")
        maj = xor32(xor32("(band a b)", "(band a c)"), "(band b c)")
        lines.append("    (set! t1 (band (+ (+ (+ (+ h %s) %s) %d) (vec-nth Wx %d)) 4294967295))"
                      % (bigS1("e"), chf, K[r], r))
        lines.append("    (set! t2 (band (+ %s %s) 4294967295))" % (bigS0("a"), maj))
        for frm, to in [("h","g"),("g","f"),("f","e"),("e","d"),("d","c"),("c","b"),("b","a")]:
            if to == "e":
                lines.append("    (set! e (band (+ d t1) 4294967295))")
            elif to == "a":
                lines.append("    (set! a (band (+ a t2) 4294967295))")
            else:
                lines.append("    (set! %s %s)" % (to, frm))
    lines.append("    (list %s))" % " ".join(
        "(band (+ h%d %s) 4294967295)" % (i, v) for i, v in enumerate("abcdefgh")))
    lines.append("))")
    w("\n".join(lines))
    w()

for b in range(6):
    emit_block(b)

# driver
dl = ["(define (sha256-words msg nw tail tlen)"]
dl.append("  (let ((padw (if (= tlen 0) 2147483648 (bor tail (shl 128 (* 8 tlen)))))")
dl.append("        (bitlen (+ (* 32 nw) (* 8 tlen))))")
dl.append("    (let ((W1 (vec-push msg padw)))")
dl.append("      (let ((Wp (loop ((Wv W1))")
dl.append("                  (if (= (mod (vec-length Wv) 16) 14) Wv")
dl.append("                      (recur (vec-push Wv 0))))")
dl.append("        (let ((W (vec-push (vec-push Wp 0) bitlen)))")
dl.append("          (let ((r1 (sha-blk0 W %d %d %d %d %d %d %d %d)))" % tuple(H0))
dl.append("            (let ((r2 (if (>= (/ (vec-length W) 16) 2)")
dl.append("                        (sha-blk1 W %s) r1)))" % " ".join("(vec-nth r1 %d)" % i for i in range(8)))
dl.append("            (let ((r3 (if (>= (/ (vec-length W) 16) 3)")
dl.append("                        (sha-blk2 W %s) r2)))" % " ".join("(vec-nth r2 %d)" % i for i in range(8)))
dl.append("            (let ((r4 (if (>= (/ (vec-length W) 16) 4)")
dl.append("                        (sha-blk3 W %s) r3)))" % " ".join("(vec-nth r3 %d)" % i for i in range(8)))
dl.append("            (let ((r5 (if (>= (/ (vec-length W) 16) 5)")
dl.append("                        (sha-blk4 W %s) r4)))" % " ".join("(vec-nth r4 %d)" % i for i in range(8)))
dl.append("            (let ((r6 (if (>= (/ (vec-length W) 16) 6)")
dl.append("                        (sha-blk5 W %s) r5)))" % " ".join("(vec-nth r5 %d)" % i for i in range(8)))
dl.append("              r6)))))))))))")

open("/tmp/bip340_core.lisp", "w").write("\n".join(L) + "\n")
print("CORE emitted:", len(L), "lines")
