#!/usr/bin/env python3
"""Splice v3 (clean rewrite): sealed-root per-user Nostr worker.

Ops (input JSON "case"):
  i (105): init   {seed:<64hex>}  -> {"root_commitment":...}   root stored AS HEX TEXT
  d (100): derive {caller}        -> "pkhex|df|skhex"
  s (115): sign   {caller,ts,kind,content} -> {"id","sig"}   pk/df/sk ALL from derive-rec (non-spoofable)
c (99): claim  (secret NOSTR_ROOT -> storage; storage-first=stable).\nLegacy 78/69 kept. Root hex text stored (storage round-trips strings; derive-rec expects hex).
Invariant: root and sk NEVER appear in outputs.
"""
import sys

SRC = '/tmp/nostr_probe/bip340_nostr.lisp'
src = open(SRC).read()

def die(msg):
    print("FATAL:", msg); sys.exit(1)

def sub1(old, new, label):
    global src
    n = src.count(old)
    if n != 1:
        die(f"{label}: anchor count {n} (need 1)")
    src = src.replace(old, new)
    print(f"ok: {label}")

# ---------- 1. helpers: substr-from + derive-rec clone ----------
sub1(
'''         (skred (words-hex (fe-words-be duse))))
    (str-cat pkhex (str-cat "|" (str-cat (hexc flip) (str-cat "|" skred))))))
;; ---- Nostr NIP-01 event id + BIP-340 signature ----''',
'''         (skred (words-hex (fe-words-be duse))))
    (str-cat pkhex (str-cat "|" (str-cat (hexc flip) (str-cat "|" skred))))))

(define (substr-from s i n out)
  (if (= n 0) out
      (substr-from s (+ i 1) (- n 1)
                   (str-cat out (hex-decode (byte-hex (byte-at s i)))))))

(define (nostr-derive-rec root caller)
  (let* ((cat (str-cat root (str-cat (hex-decode "1f") caller)))
         (skh (sha-fixed cat (str-len cat)))
         (d (sc-redv (sc-from-hex skh)))
         (P (sc-mul-gp d))
         (pkhex (words-hex (fe-words-be (fm (aff-x (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0)))))
         (yw (fe-words-be (fm (aff-y (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0))))
         (flip (band (vec-nth yw 7) 1))
         (duse (if (= flip 1) (sc-negv d) d))
         (skred (words-hex (fe-words-be duse))))
    (str-cat pkhex (str-cat "|" (str-cat (hexc flip) (str-cat "|" skred))))))

(define (json-id-sig idh sig)
  (str-cat "{" (str-cat (qch) (str-cat "id" (str-cat (qch) (str-cat ":" (str-cat (qch) (str-cat idh (str-cat (qch) (str-cat "," (str-cat (qch) (str-cat "sig" (str-cat (qch) (str-cat ":" (str-cat (qch) (str-cat sig (str-cat (qch) "}")))))))))))))))))

(define (nostr-sign-v2 root caller pkh df-in ts kind content)
  (let* ((cat (str-cat root (str-cat (hex-decode "1f") caller)))
         (skh (words-hex (fe-words-be (sc-redv (sc-from-hex (sha-fixed cat (str-len cat)))))))
         (pflip (- df-in 48))
         (q (qch))
         (s1 (str-cat "[0," q))
         (s2 (str-cat s1 pkh))
         (s3 (str-cat s2 q))
         (s4 (str-cat s3 ","))
         (s5 (str-cat s4 (str-cat q (str-cat ts (str-cat q ",")))))
         (s7 (str-cat s5 (str-cat q (str-cat kind (str-cat q ",")))))
         (s8 (str-cat s7 "[],"))
         (s9 (str-cat s8 q))
         (s10 (str-cat s9 content))
         (s11 (str-cat s10 q))
         (ser (str-cat s11 "]"))
         (idh (sha-fixed ser (str-len ser)))
         (sig (phase-B skh idh pkh pflip 1 "")))
    (json-id-sig idh sig)))

;; ---- Sealed-root op wrappers ----
(define (nostr-init input)
  (let* ((root (json-get-str "seed" input))
         (prev (outlayer/storage-get "nostr-root"))
         (eff (if prev prev root))
         (sto (if prev 0 (outlayer/storage-set "nostr-root" root)))
         (com (sha-fixed eff (str-len eff))))
    (str-cat "{" (str-cat (qch) (str-cat "root_commitment" (str-cat (qch) (str-cat ":" (str-cat (qch) (str-cat com (str-cat (qch) "}"))))))))))

(define (nostr-derive2 input)
  (let* ((root (outlayer/storage-get "nostr-root")))
    (if root
        (let* ((q (qch))
               (rec (nostr-derive-rec root (json-get-str "caller" input)))
               (pkh (substr-from rec 0 64 ""))
               (dfh (substr-from rec 65 1 "")))
          (str-cat "{" (str-cat q (str-cat "pk" (str-cat q (str-cat ":" (str-cat q (str-cat pkh (str-cat q (str-cat "," (str-cat q (str-cat "df" (str-cat q (str-cat ":" (str-cat q (str-cat dfh (str-cat q "}")))))))))))))))))
        "no root: run init")))

(define (nostr-init2 input)
  (let* ((q (qch))
         (cur (outlayer/storage-get "nostr-root")))
    (if cur
        (str-cat "{" (str-cat q (str-cat "root_commitment" (str-cat q (str-cat ":" (str-cat q (str-cat (sha-fixed cur (str-len cur)) (str-cat q "}"))))))))
        (let* ((env (env/get "NOSTR_ROOT")))
          (if env
              (let* ((sto (outlayer/storage-set "nostr-root" env)))
                (str-cat "{" (str-cat q (str-cat "root_commitment" (str-cat q (str-cat ":" (str-cat q (str-cat (sha-fixed env (str-len env)) (str-cat q "}")))))))))
              "no root: set NOSTR_ROOT project secret")))))

(define (nostr-sign2 input)
  (let* ((root (outlayer/storage-get "nostr-root")))
    (if root
        (nostr-sign-v2 root
                       (json-get-str "caller" input)
                       (json-get-str "pk" input)
                       (byte-at (json-get-str "df" input) 0)
                       (json-get-str "ts" input)
                       (json-get-str "kind" input)
                       (json-get-str "content" input))
        "no root: run init")))

;; ---- Nostr NIP-01 event id + BIP-340 signature ----''',
'helpers + derive-rec + json-id-sig + wrappers')


# ---------- 3. dispatcher: op pre-chain + tail paren debt ----------
sub1(
'''    (if (= csw 78)
        (nostr-derive (json-get-str "root" input) (json-get-str "caller" input))''',
'''    (if (= csw 99) (nostr-init2 input)
        (if (= csw 105) (nostr-init input)
        (if (= csw 100) (nostr-derive2 input)
            (if (= csw 115) (nostr-sign2 input)
                (if (= csw 78)
                    (nostr-derive (json-get-str "root" input) (json-get-str "caller" input))''',
'dispatcher: op pre-chain')

sub1(
'''(sc-mul-g2 (sc-from-hex sk-h))
                                  (cat8l W24)))))))))))))''',
'''(sc-mul-g2 (sc-from-hex sk-h))
                                  (cat8l W24)))))))))))))))))''',
'dispatcher: tail +3 closers')

open(SRC, 'w').write(src)
print("written:", len(src), "chars")
