#!/usr/bin/env python3
# gen_bip340_sign.py — BIP340 signing for shamod.lisp via TWO-PHASE protocol.
#
# Runtime finding (2026-09-20): ONE scalar mult per inlayer run works; a SECOND
# mult in the same run traps (reproduced across sc-mul-gp/sc-mul-g2, limbs/lanes).
# Protocol therefore splits: run A computes P = d*G (pk hex + y-parity flip bit),
# run B computes R = k*G and s (k = tagged_hash(pk) is d-independent).
#
# Run A: {"case":"D","dbg":"P","sk":...}                 -> "pkhex|flipchar"
# Run B: {"case":"D","dbg":"*","sk":...,"pk":...,"df":..} -> per-leg or "rhex|shex"
# Sign:  {"case":"S","sk":...,"msg":...}  -> single-run full sign: EXPECTED TRAP
#         (kept deliberately as the standing probe for the two-mult limit).
#
# Conventions learned the hard way:
# - fe-words-be returns BE limb list (element 0 = most significant) => parity bit
#   lives in element 8 (bit 0 of the last limb), NOT 7.
# - words-hex renders the list in order => to emit the NUMBER kflip, put it in
#   the LAST of 9 limbs: (list 0 0 0 0 0 0 0 0 kflip).
# - callees must be defined before callers (compiler rejects forward refs).
import re, subprocess, json, hashlib, sys, os

SRC = '/tmp/nostr_probe/shamod.lisp'
OUT_LISP = '/tmp/nostr_probe/bip340.lisp'
OUT_WASM = '/tmp/nostr_probe/bip340.wasm'
NC = '/Users/asil/dev/lisp-rlm/target/release/near-compile'
B = '/Users/asil/.local/bin/inlayer'
D = '/tmp/nostr_probe'

src = open(SRC).read()
names = re.findall(r'^\(define \(([\w\-?>]+)', src, re.M)
for dep in ['sc-mul-g2', 'sc-from-hex', 'words->limbs', 'fe-words-be', 'words-hex',
            'fe-sqrt', 'c-pm2', 'c-r2', 'c-sevenm', 'fa', 'fs', 'bword', 'bat', 'zs',
            'len8', 'sched16', 'sha-h1', 'sha-h2', 'c-h0', 'sc-negv',
            'sc-addv', 'sc-redv', 'fmn', 'fm', 'hexc', 'w24v',
            'pt-dbl', 'my-add']:
    if dep not in names:
        print(f"missing dep: {dep}"); sys.exit(1)
# band/shr/hex-decode are compiler builtins (not Lisp defs) — used without assert.

def parens_balanced(s):
    d, instr, esc = 0, False, False
    for ch in s:
        if esc:
            esc = False; continue
        if ch == '\\':
            esc = True; continue
        if ch == '"':
            instr = not instr; continue
        if instr:
            continue
        if ch == '(':
            d += 1
        elif ch == ')':
            d -= 1
            if d < 0:
                return False
    return d == 0

def sha_fixed():
    return '''(define (padlen s n)
  (str-cat s (hex-decode "80") (zs (- (- (* 64 (+ 1 (/ (+ n 8) 64))) n) 9)) (len8 (* 8 n))))
(define (sha-n-blocks sp nb b H)
  (let* ((j0 (* b 64))
         (blk (list (bword sp j0 0)
                    (bword sp (+ j0 4) 0)
                    (bword sp (+ j0 8) 0)
                    (bword sp (+ j0 12) 0)
                    (bword sp (+ j0 16) 0)
                    (bword sp (+ j0 20) 0)
                    (bword sp (+ j0 24) 0)
                    (bword sp (+ j0 28) 0)
                    (bword sp (+ j0 32) 0)
                    (bword sp (+ j0 36) 0)
                    (bword sp (+ j0 40) 0)
                    (bword sp (+ j0 44) 0)
                    (bword sp (+ j0 48) 0)
                    (bword sp (+ j0 52) 0)
                    (bword sp (+ j0 56) 0)
                    (bword sp (+ j0 60) 0)))
         (sch (sched16 blk))
         (H1 (sha-h2 sch (sha-h1 sch H) H)))
    (if (= (+ b 1) nb)
        (words-hex H1)
        (sha-n-blocks sp nb (+ b 1) H1))))
(define (sha-fixed s n)
  (sha-n-blocks (padlen s n) (/ (+ n 72) 64) 0 (c-h0 0)))
(define (tag-hash-fixed tg s n)
  (let* ((th (sha-fixed tg (str-len tg)))
         (tb (hex-decode th)))
    (sha-fixed (str-cat tb (str-cat tb s)) (+ 64 n))))'''

# mul returning the Jacobian triple (file sc-mul-g ladder shape; triple kept).
MULGP = '''(define (sc-mul-gp k)
  (loop ((X (c-zero 0)) (Y (c-zero 0)) (Z (c-zero 0)) (i 255))
    (let* ((dbl (pt-dbl X Y Z))
           (bit (band (shr (vec-nth k (/ i 30)) (mod i 30)) 1))
           (sum (if (= bit 1)
                    (my-add (vec-nth dbl 0) (vec-nth dbl 1) (vec-nth dbl 2))
                    dbl)))
      (if (< i 1)
          sum
          (recur (vec-nth sum 0) (vec-nth sum 1) (vec-nth sum 2) (- i 1))))))'''

AFF = '''(define (aff-x X Y Z)
  (let ((Zi (fe-pow Z (c-pm2 0))))
    (fm X (fm Zi Zi))))
(define (aff-y X Y Z)
  (let ((Zi (fe-pow Z (c-pm2 0))))
    (fm Y (fm Zi (fm Zi Zi)))))'''

# BUG FIX (2026-09-20, root-caused via limb-exact Python port of sc-addmod):
# shamod's sc-addmod comparator g walks limbs LSB-FIRST (d0>n0 alone fires
# "greater"), so a sum already < n gets n subtracted anyway -> s + 2^270 - n,
# rendered (256-bit fe-words-be) as s + 2^128 - n_lo128. sc-reduce compares
# MSB-first (correct). Fix: plain 9-limb carry add (no conditional subtract)
# + sc-redv. Domain note: kuse, ed < n < 2^256 => sum < 2^257 fits 9x30-bit
# limbs; top limb stays < 2^18, no overflow.
MYADD = '''(define (sc-add-raw a b)
  (let ((v 0) (c 0) (r0 0) (r1 0) (r2 0) (r3 0) (r4 0) (r5 0) (r6 0) (r7 0) (r8 0))
    (begin
    (set! v (+ (vec-nth a 0) (vec-nth b 0)))
    (set! r0 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 1) (vec-nth b 1)) c))
    (set! r1 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 2) (vec-nth b 2)) c))
    (set! r2 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 3) (vec-nth b 3)) c))
    (set! r3 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 4) (vec-nth b 4)) c))
    (set! r4 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 5) (vec-nth b 5)) c))
    (set! r5 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 6) (vec-nth b 6)) c))
    (set! r6 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 7) (vec-nth b 7)) c))
    (set! r7 (band v 1073741823))
    (set! c (shr v 30))
    (set! v (+ (+ (vec-nth a 8) (vec-nth b 8)) c))
    (set! r8 (band v 1073741823))
    (list r0 r1 r2 r3 r4 r5 r6 r7 r8))))
'''

# BUG #2 (shamod, 2026-09-20): fe-muln is value-dependently WRONG (low ~128
# bits corrupt on some operands; high bits exact; throwaway-msg e fine, nostr
# event-id e corrupt; fmn(e, c-none) itself corrupts). Evidence: runN/runO
# legs eM/fmn1/ed. Route-around: shift-and-add mod-n multiply using ONLY
# proven primitives (sc-add-raw, sc-redv, band/shr) — same ladder shape as
# the proven sc-mul-gp. 256 iterations, i=255..0 covers all 256 bits of e.
MULMODN = '''(define (sc-mulmod-n a b)
  (loop ((acc (list 0 0 0 0 0 0 0 0 0)) (i 255))
    (let* ((acc2 (sc-redv (sc-add-raw acc acc)))
           (bit (band (shr (vec-nth b (/ i 30)) (mod i 30)) 1))
           (acc3 (if (= bit 1) (sc-redv (sc-add-raw acc2 a)) acc2)))
      (if (= i 0) acc3 (recur acc3 (- i 1))))))'''

BIP = '''(define (c-zb _d) (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))\n(define (byte-hex x) (str-cat (hexc (shr (band x 255) 4)) (hexc (band x 15))))\n(define (xorb-str a b) (hex-decode (str-cat (byte-hex (xor32 (byte-at a 0) (byte-at b 0))) (byte-hex (xor32 (byte-at a 1) (byte-at b 1))) (byte-hex (xor32 (byte-at a 2) (byte-at b 2))) (byte-hex (xor32 (byte-at a 3) (byte-at b 3))) (byte-hex (xor32 (byte-at a 4) (byte-at b 4))) (byte-hex (xor32 (byte-at a 5) (byte-at b 5))) (byte-hex (xor32 (byte-at a 6) (byte-at b 6))) (byte-hex (xor32 (byte-at a 7) (byte-at b 7))) (byte-hex (xor32 (byte-at a 8) (byte-at b 8))) (byte-hex (xor32 (byte-at a 9) (byte-at b 9))) (byte-hex (xor32 (byte-at a 10) (byte-at b 10))) (byte-hex (xor32 (byte-at a 11) (byte-at b 11))) (byte-hex (xor32 (byte-at a 12) (byte-at b 12))) (byte-hex (xor32 (byte-at a 13) (byte-at b 13))) (byte-hex (xor32 (byte-at a 14) (byte-at b 14))) (byte-hex (xor32 (byte-at a 15) (byte-at b 15))) (byte-hex (xor32 (byte-at a 16) (byte-at b 16))) (byte-hex (xor32 (byte-at a 17) (byte-at b 17))) (byte-hex (xor32 (byte-at a 18) (byte-at b 18))) (byte-hex (xor32 (byte-at a 19) (byte-at b 19))) (byte-hex (xor32 (byte-at a 20) (byte-at b 20))) (byte-hex (xor32 (byte-at a 21) (byte-at b 21))) (byte-hex (xor32 (byte-at a 22) (byte-at b 22))) (byte-hex (xor32 (byte-at a 23) (byte-at b 23))) (byte-hex (xor32 (byte-at a 24) (byte-at b 24))) (byte-hex (xor32 (byte-at a 25) (byte-at b 25))) (byte-hex (xor32 (byte-at a 26) (byte-at b 26))) (byte-hex (xor32 (byte-at a 27) (byte-at b 27))) (byte-hex (xor32 (byte-at a 28) (byte-at b 28))) (byte-hex (xor32 (byte-at a 29) (byte-at b 29))) (byte-hex (xor32 (byte-at a 30) (byte-at b 30))) (byte-hex (xor32 (byte-at a 31) (byte-at b 31))))))\n(define (phase-A sk-h)
  (let* ((d (sc-from-hex sk-h))
         (P (sc-mul-gp d))
         (pkhex (words-hex (fe-words-be (fm (aff-x (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0)))))
         (yw (fe-words-be (fm (aff-y (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0))))
         (flip (band (vec-nth yw 7) 1)))
    (str-cat pkhex (hexc flip))))
(define (phase-B sk-h mhex pkhex pflip do-sign aux-hex)
  (let* ((d (sc-from-hex sk-h))
         (duse (if (= pflip 1) (sc-negv d) d))
         (pkb (hex-decode pkhex))
         (mm (hex-decode mhex))
         (auxb (if (= (str-len aux-hex) 64) (hex-decode aux-hex) (c-zb 0)))
         (hab (hex-decode (tag-hash-fixed "BIP0340/aux" auxb 32)))
         (db (hex-decode (words-hex (fe-words-be duse))))
         (tb (xorb-str db hab))
         (tn (tag-hash-fixed "BIP0340/nonce" (str-cat tb (str-cat pkb mm)) 96))
         (kraw (sc-from-hex tn))
         (knum (sc-reduce (vec-nth kraw 0) (vec-nth kraw 1) (vec-nth kraw 2) (vec-nth kraw 3) (vec-nth kraw 4) (vec-nth kraw 5) (vec-nth kraw 6) (vec-nth kraw 7) (vec-nth kraw 8)))
         (KP (sc-mul-gp knum))
         (rhex (words-hex (fe-words-be (fm (aff-x (vec-nth KP 0) (vec-nth KP 1) (vec-nth KP 2)) (c-onep 0)))))
         (ryw (fe-words-be (fm (aff-y (vec-nth KP 0) (vec-nth KP 1) (vec-nth KP 2)) (c-onep 0))))
         (kflip (band (vec-nth ryw 7) 1))
         (kuse (if (= kflip 1) (sc-negv knum) knum))
         (rXb (hex-decode rhex))
         (ch (tag-hash-fixed "BIP0340/challenge" (str-cat rXb (str-cat pkb mm)) 96))
         (eM (sc-from-hex ch))
         (ed (sc-mulmod-n duse eM))
         (ssum (sc-add-raw ed kuse))
         (shex (words-hex (fe-words-be (sc-redv ssum)))))
    (if (= do-sign 1)
        (str-cat rhex shex)
        shex)))'''

def make_dbg():
    # Nested-if dispatchers assembled programmatically: parens by construction.
    a_binds = (
        '  (let* ((d (sc-from-hex (json-get-str "sk" input)))\n'
        '         (P (sc-mul-gp d))\n'
        '         (pkhex (words-hex (fe-words-be (fm (aff-x (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0)))))\n'
        '         (yw (fe-words-be (fm (aff-y (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0))))\n'
        '         (flip (band (vec-nth yw 7) 1))\n'
        '         (duse (if (= flip 1) (sc-negv d) d)))'
    )
    e = '(str-cat pkhex (hexc flip))'
    for code, body in reversed([(89, '(words-hex (fe-words-be duse))'),
                                (84, '(cat8l (fe-words-pure (fm (aff-y (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0))))')]):
        e = f'(if (= case {code}) {body} {e})'
    A = f'(define (phase-A-legs case input)\n{a_binds}\n    {e}))\n'

    b_binds = (
        '  (let* ((pflip (- dfb 48))\n'
        '         (mm (hex-decode (json-get-str "msg" input)))\n'
        '         (d (sc-from-hex (json-get-str "sk" input)))\n'
        '         (duse (if (= pflip 1) (sc-negv d) d))\n'
        '         (auxb (let ((ah (json-get-str "aux" input))) (if (= (str-len ah) 64) (hex-decode ah) (c-zb 0))))\n'
        '         (hab (hex-decode (tag-hash-fixed "BIP0340/aux" auxb 32)))\n'
        '         (db (hex-decode (words-hex (fe-words-be duse))))\n'
        '         (tb (xorb-str db hab))\n'
        '         (tn (tag-hash-fixed "BIP0340/nonce" (str-cat tb (str-cat (hex-decode pkhex) mm)) 96))\n'
        '         (kraw (sc-from-hex tn))\n'
        '         (knum (sc-reduce (vec-nth kraw 0) (vec-nth kraw 1) (vec-nth kraw 2) (vec-nth kraw 3) (vec-nth kraw 4) (vec-nth kraw 5) (vec-nth kraw 6) (vec-nth kraw 7) (vec-nth kraw 8)))\n'
        '         (KP (sc-mul-gp knum))\n'
        '         (rhex (words-hex (fe-words-be (fm (aff-x (vec-nth KP 0) (vec-nth KP 1) (vec-nth KP 2)) (c-onep 0)))))\n'
        '         (ryw (fe-words-be (fm (aff-y (vec-nth KP 0) (vec-nth KP 1) (vec-nth KP 2)) (c-onep 0))))\n'
        '         (kflip (band (vec-nth ryw 7) 1))\n'
        '         (kuse (if (= kflip 1) (sc-negv knum) knum))\n'
        '         (rXb (hex-decode rhex))\n'
        '         (ch (tag-hash-fixed "BIP0340/challenge" (str-cat rXb (str-cat (hex-decode pkhex) mm)) 96))\n'
        '         (eM (sc-from-hex ch))\n'
        '         (ed (sc-mulmod-n duse eM))\n'
        '         (ssum (sc-add-raw ed kuse))\n'
        '         (shex (words-hex (fe-words-be (sc-redv ssum)))))'
    )
    b_cases = [(82, 'rhex'),
               (87, '(words-hex (list 0 0 0 0 0 0 0 0 kflip))'),
               (81, '(cat8l (fe-words-pure (fm (aff-y (vec-nth KP 0) (vec-nth KP 1) (vec-nth KP 2)) (c-onep 0))))'),
               (73, '(words-hex (fe-words-be ed))'),
               (71, '(words-hex (fe-words-be ssum))'),
               (78, 'tn'),
               (75, '(words-hex (fe-words-be knum))'),
               (72, 'ch'),
               (67, '(words-hex (fe-words-be kuse))')]
    e = '(str-cat rhex shex)'
    for code, body in reversed(b_cases):
        e = f'(if (= case {code}) {body} {e})'
    Bdef = f'(define (phase-B-legs case input pkhex dfb)\n{b_binds}\n    {e}))\n'

    G = ('(define (dbg case input)\n'
         '  (let* ((pkx-in (json-get-str "pk" input)))\n'
         '    (if (= (str-len pkx-in) 0)\n'
         '        (phase-A-legs case input)\n'
         '        (phase-B-legs case input pkx-in (byte-at (json-get-str "df" input) 0)))))\n')
    return A + Bdef + G

RUN = '''(define (run input)
  (let* ((cs (json-get-str "case" input))
         (csw (byte-at cs 0))
         (sk-h (json-get-str "sk" input))
         (msh (json-get-str "msg" input))
         (pkx-in (json-get-str "pk" input)))
    (if (= csw 83)
        (let* ((d (sc-from-hex sk-h))
               (P (sc-mul-gp d))
               (pkhex (words-hex (fe-words-be (fm (aff-x (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0)))))
               (yw (fe-words-be (fm (aff-y (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0))))
               (flip (band (vec-nth yw 7) 1)))
          (str-cat pkhex "|" (phase-B sk-h msh pkhex flip 1 (json-get-str "aux" input))))
        (if (= csw 68)
            (dbg (byte-at (json-get-str "dbg" input) 0) input)
            (let* ((skh (hex-decode sk-h))
                   (m (hex-decode msh))
                   (W24 (w24v skh m (hex-decode pkx-in))))
              (if (= csw 100)
                  (let* ((PD (pt-dbl (c-gxm 0) (c-gym 0) (c-onem 0))))
                    (ptaa-dbg (vec-nth PD 0) (vec-nth PD 1) (vec-nth PD 2)))
                  (if (= csw 115)
                      (sha-96 W24)
                      (if (= csw 103)
                          (let* ((PD (pt-dbl (c-gxm 0) (c-gym 0) (c-onem 0)))
                                 (PA (pt-add-aff (vec-nth PD 0) (vec-nth PD 1) (vec-nth PD 2))))
                            (cat8l (fe-words-pure (fm (vec-nth PA 0) (c-onep 0)))))
                          (if (= csw 120)
                              (let* ((PD (pt-dbl (c-gxm 0) (c-gym 0) (c-onem 0))))
                                (str-cat (str-cat (cat8l (fe-words-pure (fm (vec-nth PD 0) (c-onep 0))))
                                                  (cat8l (fe-words-pure (fm (vec-nth PD 1) (c-onep 0)))))
                                         (cat8l (fe-words-pure (fm (vec-nth PD 2) (c-onep 0))))))
                              (if (= csw 107)
                                  (sc-mul-g2 (sc-from-hex sk-h))
                                  (cat8l W24)))))))))))'''

_chunks = [('sha_fixed', sha_fixed()), ('MULGP', MULGP), ('AFF', AFF),
           ('BIP', BIP), ('dbg', make_dbg()), ('RUN', RUN)]
for nm, ch in _chunks:
    if not parens_balanced(ch):
        print(f"chunk {nm} unbalanced"); sys.exit(1)

NEW = (sha_fixed() + '\n' + MULGP + '\n' + AFF + '\n' + MYADD + '\n' + MULMODN + '\n' + BIP + '\n'
       + make_dbg() + '\n' + RUN + '\n')
full = src + NEW
assert parens_balanced(NEW), "NEW unbalanced"
assert full.count('(') - full.count(')') == 0, "unbalanced parens"
open(OUT_LISP, 'w').write(full)
print(f"bip340.lisp: {len(full)} bytes")

r = subprocess.run(f"{NC} {OUT_LISP} --target=outlayer-p2 -o {OUT_WASM} 2>&1", shell=True,
                   capture_output=True, text=True, timeout=560)
log = r.stdout + r.stderr
if '❌' in log or 'error' in log.lower():
    print("compile rc:", r.returncode)
    for line in log.splitlines():
        if '❌' in line or 'error' in line.lower():
            print(line[:300])
    sys.exit(1)
print("compile rc: 0 | wasm:", os.path.getsize(OUT_WASM))

# ---------- python judge ----------
p = 2**256 - 2**32 - 977
nn = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8

sk_hex = hashlib.sha256(b"throwaway sk").hexdigest()
msg_hex = hashlib.sha256(b"throwaway msg").hexdigest()
d = int(sk_hex, 16)
mm = bytes.fromhex(msg_hex)

def pt_add(Pv, Qv):
    if Pv is None: return Qv
    if Qv is None: return Pv
    if Pv[0] == Qv[0] and (Pv[1] + Qv[1]) % p == 0: return None
    if Pv == Qv:
        l = (3 * Pv[0] * Pv[0]) * pow(2 * Pv[1], -1, p) % p
    else:
        l = (Qv[1] - Pv[1]) * pow(Qv[0] - Pv[0], -1, p) % p
    x = (l * l - Pv[0] - Qv[0]) % p
    return (x, (l * (Pv[0] - x) - Pv[1]) % p)

def mul(kk, base=None):
    Rv = None
    A = base if base is not None else (Gx, Gy)
    while kk:
        if kk & 1: Rv = pt_add(Rv, A)
        A = pt_add(A, A)
        kk >>= 1
    return Rv

def tagged_hash(tag, msg):
    t = hashlib.sha256(tag.encode()).digest()
    return hashlib.sha256(t + t + msg).digest()

P = mul(d)
pk_b = P[0].to_bytes(32, 'big')
du = d if P[1] % 2 == 0 else nn - d
aux_hex = bytes(32)  # harness passes no "aux" key => zeros on both sides
tn = tagged_hash("BIP0340/aux", aux_hex)
t = bytes(a ^ b for a, b in zip(du.to_bytes(32, 'big'), tn))
kfull = tagged_hash("BIP0340/nonce", t + pk_b + mm).hex()
kn = int(kfull, 16) % nn
RP = mul(kn)
rX = RP[0].to_bytes(32, 'big')
kuse = kn if RP[1] % 2 == 0 else nn - kn
e = int.from_bytes(tagged_hash("BIP0340/challenge", rX + pk_b + mm), 'big') % nn
s = (kuse + e * du) % nn
sig = rX + s.to_bytes(32, 'big')
pk_flip = P[1] % 2  # 1 => odd y => flip

def run_case(payload):
    r2 = subprocess.run(f"{B} run {D}/bip340.wasm run '{payload}'", shell=True,
                        capture_output=True, text=True, timeout=1700)
    outp = None
    for l in (r2.stdout or '').splitlines():
        if l.startswith('📤 Output:'):
            outp = l.split('📤 Output:', 1)[1].strip()
    return outp, r2

def leg(code, want, label, with_pk=False):
    pl = {"case": "D", "dbg": chr(code), "sk": sk_hex, "msg": msg_hex}
    if with_pk:
        pl["pk"] = pk_b.hex(); pl["df"] = str(pk_flip)
    payload = json.dumps(pl, separators=(',', ':'))
    assert all(32 < ord(c) < 127 for c in payload)
    outp, r2 = run_case(payload)
    if outp is None:
        print(f"leg {label:12}: TRAP"); return None
    ok = outp == want
    print(f"leg {label:12}: {'OK' if ok else 'WRONG'}")
    if not ok:
        print(f"   got : {outp[:100]}")
        print(f"   want: {str(want)[:100]}")
    return outp

# Phase A (one mult per run)
a_out = leg(80, pk_b.hex() + str(pk_flip), "A:pk|flip")
leg(89, f"{du:064x}", "A:duse")
# Phase B (one mult per run; pk+df passed in from A/driver)
leg(78, kfull, "B:nonce_kfull", with_pk=True)
leg(82, rX.hex(), "B:r", with_pk=True)
leg(87, f"{RP[1] % 2:064x}", "B:kflip", with_pk=True)
leg(81, f"{RP[1]:d}", "B:KP.y", with_pk=True)
leg(84, f"{P[1]:d}", "A:P.y")
leg(73, f"{du * e % nn:064x}", "B:ed", with_pk=True)
leg(71, f"{(kuse + du * e) % nn:064x}", "B:ssum", with_pk=True)
leg(72, f"{e:064x}", "B:chal", with_pk=True)
leg(67, f"{kuse:064x}", "B:kuse", with_pk=True)

# Case S: full sign in ONE run = two mults = the standing trap probe.
pl = {"case": "S", "sk": sk_hex, "msg": msg_hex}
payload = json.dumps(pl, separators=(',', ':'))
outp, r2 = run_case(payload)
print("case S (single-run full sign):", "TRAP (two-mult runtime limit, as expected)" if outp is None else outp[:64])

# Two-phase sign. sc-addmod comparator bug FIXED in-generated (sc-add-raw +
# sc-redv) — Lisp now produces the FULL signature. Case '*' (phase-B do-sign=1)
# returns "rhex|shex".
pl = {"case": "D", "dbg": "*", "sk": sk_hex, "msg": msg_hex, "pk": pk_b.hex(), "df": str(pk_flip)}
payload = json.dumps(pl, separators=(',', ':'))
wasm_sig, r2 = run_case(payload)
assert wasm_sig is not None, f"TRAP on sign run: {r2.stdout[-200:] if r2.stdout else r2.stderr[-200:]}"
print("python sig:", sig.hex())
print("wasm   sig:", wasm_sig, "(FULL sig from Lisp: r, s both wasm-computed)")
print("MATCH" if wasm_sig == sig.hex() else "MISMATCH")

# independent BIP340 verification of the wasm signature
def lift_x(bx_):
    x = int.from_bytes(bx_, 'big')
    if x >= p: return None
    y_sq = (pow(x, 3, p) + 7) % p
    y = pow(y_sq, (p + 1) // 4, p)
    if pow(y, 2, p) != y_sq: return None
    return (x, y if y % 2 == 0 else p - y)  # BIP340: even-y lift (x-only verify)
sigb = bytes.fromhex(wasm_sig)
Rv = lift_x(sigb[:32])
Pk = lift_x(pk_b)
ev = int.from_bytes(tagged_hash("BIP0340/challenge", sigb[:32] + pk_b + mm), 'big') % nn
lhs = mul(int.from_bytes(sigb[32:], 'big'))
rhs = pt_add(Rv, mul(ev, Pk))
print("VERIFICATION s*G == R + e*PK:", lhs is not None and rhs is not None and (lhs[0] - rhs[0]) % p == 0)

# ---------- real Nostr event, throwaway key ----------
print("\n--- Nostr event (NIP-01) ---")
content = "gm from pure-Lisp BIP340 (shamod.lisp on inlayer)"
evd = {
    "pubkey": pk_b.hex(), "created_at": 1758380000,
    "kind": 1, "tags": [], "content": content,
}
ser0 = json.dumps([0, evd["pubkey"], evd["created_at"], evd["kind"], evd["tags"], evd["content"]],
                  separators=(',', ':'), ensure_ascii=False).encode()
eid = hashlib.sha256(ser0).hexdigest()
print("event id:", eid)
em = bytes.fromhex(eid)
# FULL signature from Lisp for the real event id (run B hashes pk|r|msg; msg = eid).
pl = {"case": "D", "dbg": "*", "sk": sk_hex, "msg": eid, "pk": pk_b.hex(), "df": str(pk_flip)}
sig_n, r2 = run_case(json.dumps(pl, separators=(',', ':')))
assert sig_n is not None and len(sig_n) == 128, f"TRAP/bad on nostr sign: {sig_n}"
r_n = sig_n[:64]
# independent verify with the REAL event id
Rn = lift_x(bytes.fromhex(r_n))
evn = int.from_bytes(tagged_hash("BIP0340/challenge", bytes.fromhex(r_n) + pk_b + em), 'big') % nn
lhsn = mul(int.from_bytes(bytes.fromhex(sig_n)[32:], 'big'))
rhsn = pt_add(Rn, mul(evn, Pk))
okn = lhsn is not None and rhsn is not None and (lhsn[0] - rhsn[0]) % p == 0
print("nostr sig:", sig_n)
print("VERIFICATION (real event id):", okn)
evd["id"] = eid; evd["sig"] = sig_n
open(f"{D}/nostr_event.json", "w").write(json.dumps(evd, separators=(',', ':'), ensure_ascii=False))
print(f"event written: {D}/nostr_event.json")
