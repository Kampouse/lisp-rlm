#!/usr/bin/env python3
"""emit_psha.py v4 — pure-lisp SHA-256, let*-free hot path.

Design (every construct individually probe-verified under near-mock):
- NO let* in round/absorb path (let* + user-fn-call bindings miscompile).
- NO bnot in bindings (ch via 4294967295 - x instead).
- State = 8-elem h-list + 16-elem schedule window (OLDEST-first, w[0]=w[i-16]).
- Recursion args <= 6; reads via vec-nth; xor32 = a+b-2(a&b) variant ok in bodies.
- rotR keeps its let* (builtin-only bindings — proven correct).
"""
import subprocess, sys

KK = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
      0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
      0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
      0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
      0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
      0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
      0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
      0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2]

KKL = "(list " + " ".join(str(k) for k in KK) + ")"

PRE = r''';; ═══ pure-lisp SHA-256 v4 (let*-free hot path) ═══
(define (m32 x) (band x 4294967295))
(define (xor32 a b) (m32 (bor (band a (bnot b)) (band (bnot a) b))))
(define (rotR x k)
  (let* ((s (- 32 k))
         (mask (- (shl 1 k) 1))
         (lo (band (shr x k) 4294967295))
         (hi (shl (band x mask) s)))
    (bor lo hi)))
(define (zs n) (if (<= n 0) "" (str-cat (hex-decode "00") (zs (- n 1)))))
(define (bat s i) (if (< i (str-len s)) (byte-at s i) 0))
(define (bword s off j)
  (bor (m32 (shl (bat s (+ off (* j 4))) 24))
       (bor (m32 (shl (bat s (+ off (* j 4) 1)) 16))
            (bor (m32 (shl (bat s (+ off (* j 4) 2)) 8))
                 (bat s (+ off (* j 4) 3))))))
(define (hexc x) (vec-nth (list "0" "1" "2" "3" "4" "5" "6" "7" "8" "9" "a" "b" "c" "d" "e" "f") x))
(define (h1b b) (str-cat (hexc (shr (band b 240) 4)) (hexc (band b 15))))
(define (w2b w) (hex-decode (str-cat (h1b (band (shr w 24) 255))
                                     (str-cat (h1b (band (shr w 16) 255))
                                              (str-cat (h1b (band (shr w 8) 255)) (h1b (band w 255)))))))
(define (lb n s) (if (<= s 0) "" (str-cat (hex-decode (h1b (band (shr n (- s 8)) 255))) (lb n (- s 8)))))
(define (len8 n) (lb n 64))
(define (padded msg)
  (let* ((n (str-len msg))
         (n8 (* 8 n))
         (k (m32 (* 64 (+ 1 (m32 (shr (+ n 8) 6)))))))
    (str-cat msg (hex-decode "80") (zs (- k n 9)) (len8 n8))))
(define (c-kk) ''' + KKL + r''')
;; ── schedule helpers (i >= 16) ──
(define (s0 x) (xor32 (rotR x 7) (xor32 (rotR x 18) (m32 (shr x 3)))))
(define (s1 x) (xor32 (rotR x 17) (xor32 (rotR x 19) (m32 (shr x 10)))))
;; w window oldest-first: w[0]=w[i-16] .. w[15]=w[i-1]
;; FIPS: w[i] = s1(w[i-2]) + w[i-7] + s0(w[i-15]) + w[i-16]
;;   = s1(W[14]) + W[9] + s0(W[1]) + W[0]
(define (wi-of i w)
  (if (< i 16)
      (vec-nth w i)
      (m32 (+ (m32 (+ (vec-nth w 0) (s0 (vec-nth w 1))))
              (m32 (+ (vec-nth w 9) (s1 (vec-nth w 14))))))))
;; next window: i<16 keeps block b; else shift left, append wi at end
(define (nw-of i w wi)
  (if (< i 16)
      w
      (list (vec-nth w 1) (vec-nth w 2) (vec-nth w 3) (vec-nth w 4)
            (vec-nth w 5) (vec-nth w 6) (vec-nth w 7) (vec-nth w 8)
            (vec-nth w 9) (vec-nth w 10) (vec-nth w 11) (vec-nth w 12)
            (vec-nth w 13) (vec-nth w 14) (vec-nth w 15) wi)))
;; next h-list: t1/t2 inlined from h + wi (no let*, ch via subtraction)
(define (ch-of h4 h5 h6)
  (m32 (+ (band h4 h5) (band (- 4294967295 h4) h6))))
(define (S1-of h4) (xor32 (rotR h4 6) (xor32 (rotR h4 11) (rotR h4 25))))
(define (S0-of h0) (xor32 (rotR h0 2) (xor32 (rotR h0 13) (rotR h0 22))))
(define (mj-of h0 h1 h2) (bor (band h0 h1) (bor (band h0 h2) (band h1 h2))))
(define (t1-of kk i h wi)
  (m32 (+ (m32 (+ (m32 (+ (vec-nth h 7) (S1-of (vec-nth h 4))))
                  (ch-of (vec-nth h 4) (vec-nth h 5) (vec-nth h 6))))
          (m32 (+ (vec-nth kk i) wi)))))
(define (t2-of h)
  (m32 (+ (S0-of (vec-nth h 0))
          (mj-of (vec-nth h 0) (vec-nth h 1) (vec-nth h 2)))))
(define (nh-of kk i h wi)
  (list
    (m32 (+ (t1-of kk i h wi) (t2-of h)))
    (vec-nth h 0)
    (vec-nth h 1)
    (vec-nth h 2)
    (m32 (+ (vec-nth h 3) (t1-of kk i h wi)))
    (vec-nth h 4)
    (vec-nth h 5)
    (vec-nth h 6)))
;; round driver: computes wi first (top-level conditional), then recurses
(define (rd kk i h w)
  (if (>= i 64)
      h
      (rd kk (+ i 1)
          (nh-of kk i h (wi-of i w))
          (nw-of i w (wi-of i w)))))
(define (bwords s off)
  (list (bword s off 0) (bword s off 1) (bword s off 2) (bword s off 3)
        (bword s off 4) (bword s off 5) (bword s off 6) (bword s off 7)
        (bword s off 8) (bword s off 9) (bword s off 10) (bword s off 11)
        (bword s off 12) (bword s off 13) (bword s off 14) (bword s off 15)))
(define (hadd2 h wv)
  (list (m32 (+ (vec-nth h 0) (vec-nth wv 0)))
        (m32 (+ (vec-nth h 1) (vec-nth wv 1)))
        (m32 (+ (vec-nth h 2) (vec-nth wv 2)))
        (m32 (+ (vec-nth h 3) (vec-nth wv 3)))
        (m32 (+ (vec-nth h 4) (vec-nth wv 4)))
        (m32 (+ (vec-nth h 5) (vec-nth wv 5)))
        (m32 (+ (vec-nth h 6) (vec-nth wv 6)))
        (m32 (+ (vec-nth h 7) (vec-nth wv 7)))))
(define (blocks p off end h kk)
  (if (>= off end)
      h
      (blocks p (+ off 64) end
              (hadd2 h (rd kk 0 h (bwords (str-substring p off (+ off 64)) 0)))
              kk)))
(define (hadd h)
  (list (m32 (+ (vec-nth h 0) 1779033703))
        (m32 (+ (vec-nth h 1) 3144134277))
        (m32 (+ (vec-nth h 2) 1013904242))
        (m32 (+ (vec-nth h 3) 2773480762))
        (m32 (+ (vec-nth h 4) 1359893119))
        (m32 (+ (vec-nth h 5) 2600822924))
        (m32 (+ (vec-nth h 6) 528734635))
        (m32 (+ (vec-nth h 7) 1541459225))))
(define (H-init) (list 1779033703 3144134277 1013904242 2773480762 1359893119 2600822924 528734635 1541459225))
(define (psha256 msg)
  (blocks (padded msg) 0 (str-len (padded msg)) (H-init) (c-kk)))
(define (digest msg) (str-cat (w2b (vec-nth (psha256 msg) 0))
                        (str-cat (w2b (vec-nth (psha256 msg) 1))
                         (str-cat (w2b (vec-nth (psha256 msg) 2))
                          (str-cat (w2b (vec-nth (psha256 msg) 3))
                           (str-cat (w2b (vec-nth (psha256 msg) 4))
                            (str-cat (w2b (vec-nth (psha256 msg) 5))
                             (str-cat (w2b (vec-nth (psha256 msg) 6)) (w2b (vec-nth (psha256 msg) 7))))))))))
'''


# ── run block: generated, parens balanced by construction ──
# FP_GLOBAL pitfall: json-get-str results share one buffer — NEVER hold two.
# Each branch compares mode FIRST (consumed immediately), reads msg fresh after.
MSG = '(json-get-str "msg" (near/input))'
PDX = '''(bat (padded (json-get-str "msg" (near/input))) (json-get "ix" (near/input)))'''
MODES = [
    ("pb",  PDX),
    ("pad", f'(str-len (padded {MSG}))'),
    ("w",   f'(vec-nth (bwords (padded {MSG}) 0) 0)'),
    ("w2",  f'(vec-nth (bwords (padded {MSG}) 64) 0)'),
    ("w3",  f'(vec-nth (bwords (padded {MSG}) 128) 0)'),
    ("w3l", f'(vec-nth (bwords (padded {MSG}) 128) 15)'),
    ("b1",  f'(vec-nth (rd (c-kk) 0 (H-init) (bwords (padded {MSG}) 0)) 0)'),
    ("h",   f'(vec-nth (psha256 {MSG}) 0)'),
    ("p2d", f'(http-post "https://httpbin.org/post" (digest {MSG}))'),
    ("d",   f'(digest {MSG})'),
]
expr = MODES[-1][1]
for name, e in reversed(MODES[:-1]):
    expr = f'(if (= (json-get-str "mode" (near/input)) "{name}") {e} {expr})'
RUN = '(define (run input)\n    ' + expr + ')\n'

# paren balance gate (advisory; compiler is authority)
LISP = PRE + RUN
depth = 0; instr = False; incomment = False
for ch in LISP:
    if incomment:
        if ch == "\n": incomment = False
        continue
    if instr:
        if ch == '"': instr = False
        continue
    if ch == '"': instr = True
    elif ch == ';': incomment = True
    elif ch == '(': depth += 1
    elif ch == ')': depth -= 1
assert depth == 0, f"generator emitted unbalanced parens: {depth}"
LISP += '\n(export "run" run)\n'
open('/tmp/nostr_probe/psha.lisp', 'w').write(LISP)
print("psha.lisp written:", len(LISP), "bytes, balanced")

