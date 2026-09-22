#!/usr/bin/env python3
"""Splice v3 (clean rewrite): sealed-root per-user Nostr worker.

Ops (input JSON "case"):
  i (105): init   {seed:<64hex>}  -> {"root_commitment":...}   root stored AS HEX TEXT
  d (100): derive {caller}        -> "pkhex|df|skhex"
  s (115): sign   {caller,ts,kind,content} -> {"id","sig"}   pk/df/sk ALL from derive-rec (non-spoofable)
Legacy 78/69 kept. Root hex text stored (storage round-trips strings; derive-rec expects hex).
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

(define (nostr-sign-v2 root caller ts kind content)
  (let* ((rec (nostr-derive-rec root caller))
         (pkh (substr-from rec 0 64 ""))
         (skh (hex-decode (substr-from rec 66 64 "")))
         (pflip (- (byte-at rec 65) 48))
         (q (qch))
         (s1 (str-cat "[0," q))
         (s2 (str-cat s1 pkh))
         (s3 (str-cat s2 q))
         (s4 (str-cat s3 ","))
         (s5 (str-cat s4 ts))
         (s6 (str-cat s5 ","))
         (s7 (str-cat s6 kind))
         (s8 (str-cat s7 ",[],"))
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
        (nostr-derive-rec root (json-get-str "caller" input))
        "no root: run init")))

(define (nostr-sign2 input)
  (let* ((root (outlayer/storage-get "nostr-root")))
    (if root
        (nostr-sign-v2 root
                       (json-get-str "caller" input)
                       (json-get-str "ts" input)
                       (json-get-str "kind" input)
                       (json-get-str "content" input))
        "no root: run init")))

;; ---- Nostr NIP-01 event id + BIP-340 signature ----''',
'helpers + derive-rec + json-id-sig + wrappers')


# ---------- 3. dispatcher ----------
sub1(
'''(define (run input)
  (let* ((cs (json-get-str "case" input))
         (csw (byte-at cs 0))''',
'''(define (run input)
  (let* ((cs (json-get-str "case" input))
         (csw (byte-at cs 0))
         (ri (if (= csw 105) (nostr-init input) 0))
         (rd (if (= csw 100) (nostr-derive2 input) 0))
         (rs (if (= csw 115) (nostr-sign2 input) 0))''',
'dispatcher: i/d/s ops added')

open(SRC, 'w').write(src)
print("written:", len(src), "chars")
