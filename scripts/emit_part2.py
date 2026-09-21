#!/usr/bin/env python3
"""emit_part2.py — writes the sign-flow + run() lisp (driver part 2) and assembles the full program."""

PART2 = r'''
;; ── hex/byte-string to scalar limbs ──
(define (sc-from-hex h)
  (let ((raw (hex-decode h)))
    (words->limbs (word-at raw 0) (word-at raw 1) (word-at raw 2) (word-at raw 3)
                  (word-at raw 4) (word-at raw 5) (word-at raw 6) (word-at raw 7))))

;; words of sha256 over raw string
(define (hash-words s)
  (let ((raw (sha-raw s)))
    (words->limbs (word-at raw 0) (word-at raw 1) (word-at raw 2) (word-at raw 3)
                  (word-at raw 4) (word-at raw 5) (word-at raw 6) (word-at raw 7))))

;; tagged hash: sha256(taghash || taghash || msg-raw)
(define (th-sha tagname msg-raw)
  (let ((th (sha-raw tagname)))
    (sha-raw (str-cat th th msg-raw))))

;; t = a + b mod n  (vector in, limb args out via sc-addmod)
(define (sc-addv av bv)
  (sc-addmod (vec-nth av 0) (vec-nth av 1) (vec-nth av 2) (vec-nth av 3) (vec-nth av 4)
             (vec-nth av 5) (vec-nth av 6) (vec-nth av 7) (vec-nth av 8) bv))

(define (sc-redv v)
  (sc-reduce (vec-nth v 0) (vec-nth v 1) (vec-nth v 2) (vec-nth v 3) (vec-nth v 4)
             (vec-nth v 5) (vec-nth v 6) (vec-nth v 7) (vec-nth v 8)))

(define (sc-negv v)
  (sc-n-minus (vec-nth v 0) (vec-nth v 1) (vec-nth v 2) (vec-nth v 3) (vec-nth v 4)
              (vec-nth v 5) (vec-nth v 6) (vec-nth v 7) (vec-nth v 8)))

(define (to-montn v) (fmn v (c-none 0)))

;; ── BIP-340 sign, aux=0 ──
;; NOTE (FP_GLOBAL): json-get-str results share one buffer. The caller must
;; fully consume the sk string (sc-from-hex + hex-decode) BEFORE reading msg.
;; This fn therefore takes pre-extracted values, never raw json strings.
(define (bip340-sign d-raw t-d m-raw)
  (let* ((pk-x (aff-x (vec-nth (sc-mul-g d-raw) 0) (vec-nth (sc-mul-g d-raw) 1) (vec-nth (sc-mul-g d-raw) 2)))
         (pkhex (fe-ser pk-x))
         (pk-raw (hex-decode pkhex))
         ;; t = bytes(d) passed in as t-d; rand = tagged("BIP0340/nonce", t || pk || m)
         (rand (th-sha "BIP0340/nonce" (str-cat t-d pk-raw m-raw)))
         (k0 (sc-redv (hash-words rand)))
         (K (sc-mul-g k0))
         (r-x (aff-x (vec-nth K 0) (vec-nth K 1) (vec-nth K 2)))
         (rhex (fe-ser r-x))
         ;; e = int(tagged("BIP0340/challenge", r || pk || m)) mod n
         (ehex (th-sha "BIP0340/challenge" (str-cat (hex-decode rhex) pk-raw m-raw)))
         (e-raw (hash-words ehex))
         (e-mont (to-montn e-raw))
         (d-mont (fmn d-raw (c-none 0)))
         (ed (fmn e-mont d-mont))
         (k-mont (to-montn k0))
         (s-sum (sc-addv k-mont ed))
         (s-mont (to-montn s-sum))
         (s-neg (sc-negv s-mont))
         (rhex-lo (str-cat (hex8 0) (hex8 0)))
)
    (str-cat pkhex " " rhex " " (fe-ser s-mont))))

(define (find-sp s i)
  (if (>= i (str-len s)) (- 0 1)
      (if (= (byte-at s i) 32) i (find-sp s (+ i 1)))))

(define (run input)
  (let* ((skh (json-get-str "sk" input))
         ;; consume sk FULLY before any second json-get-str (FP_GLOBAL rule)
         (d-raw (sc-from-hex skh))
         (t-d (hex-decode skh))
         (mh (json-get-str "msg" input))
         (m-raw (hex-decode mh))
         ;; tags/content must survive as strings: copy each into fresh
         ;; fn-result storage immediately after its read
         (tags1 (json-get-str "tags" input))
         (tags (str-cat tags1 ""))
         (content1 (json-get-str "content" input))
         (content (str-cat content1 ""))
         (mode (json-get "mode" input))
         (ts (json-get "ts" input))
         (signed (bip340-sign d-raw t-d m-raw)))
    (if (str= (str-substring signed 0 7) "SIGFAIL")
        signed
        (if (= mode 1)
            (let* ((sp1 (find-sp signed 0))
                   (pk (str-substring signed 0 sp1))
                   (rest1 (str-substring signed (+ sp1 1) (str-len signed)))
                   (sp2 (find-sp rest1 0))
                   (rr (str-substring rest1 0 sp2))
                   (ss (str-substring rest1 (+ sp2 1) (str-len rest1)))
                   (kind 1)
                   (url "https://mas-levitra-resulting-sound.trycloudflare.com/publish")
                   (preimage (str-cat "[0,\"" pk "\"," (to-string ts) "," (to-string kind) "," tags ",\"" content "\"]"))
                   (eid (hex-encode (sha-raw preimage)))
                   (ev (str-cat "{\"id\":\"" eid "\",\"pubkey\":\"" pk "\",\"created_at\":" (to-string ts)
                                ",\"kind\":" (to-string kind) ",\"tags\":" tags ",\"" "content\":\"" content
                                "\",\"sig\":\"" rr ss "\"}"))
                   (resp (http-post url ev)))
              (str-cat signed " " eid "|" (str-substring resp 0 80)))
            signed))))
'''

lib = open('/tmp/nostr_probe/sign_lib.lisp').read()
psha = open('/tmp/nostr_probe/psha.lisp').read()
psha_core = psha[:psha.rfind('(define (run')]  # SHA core sans its run/export
d1 = open('/tmp/nostr_probe/sign_driver.lisp').read()
# pure-lisp sha shim: same contract as the broken-in-P2 sha256-hash builtin
# (raw byte string in -> raw 32-byte string out)
shim = '(define (sha-raw s) (hex-decode (words-hex (psha256 s))))\n'
full = lib + "\n" + psha_core + "\n" + d1 + "\n" + shim + "\n" + PART2 + '\n(export "run" run)\n'
open('/tmp/nostr_probe/nostr_sign.lisp', 'w').write(full)
print("assembled nostr_sign.lisp:", full.count(chr(10)), "lines")
print("sha-raw refs:", full.count('(sha-raw '))
print("psha256 core present:", '(define (psha256' in full)
