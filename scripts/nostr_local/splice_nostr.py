#!/usr/bin/env python3
"""Splice Nostr cases (N=derive, E=event-sign) into bip340.lisp -> bip340_nostr.lisp.

N (csw 78): sk = SHA256(root || 0x1f || caller) reduced mod n -> "pkhex|flip|skhex"
E (csw 69): ser=[0,"pk",ts,kind,[],"content"] -> id=SHA256(ser) -> sig via phase-B
            -> "id|sig"
"""
import sys

SRC = '/tmp/nostr_probe/bip340.lisp'
DST = '/tmp/nostr_probe/bip340_nostr.lisp'

s = open(SRC).read()

HELPERS = r'''
;; ---- Nostr identity derivation (per-caller key from root secret) ----
(define (qch) (hex-decode "22"))
(define (nostr-derive root caller)
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
;; ---- Nostr NIP-01 event id + BIP-340 signature ----
(define (nostr-sign pkh skh ts kind content df-in)
  (let* ((pflip (- df-in 48))
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
    (str-cat "{" (str-cat (qch) (str-cat "id" (str-cat (qch) (str-cat ":" (str-cat (qch) (str-cat idh (str-cat (qch) (str-cat "," (str-cat (qch) (str-cat "sig" (str-cat (qch) (str-cat ":" (str-cat (qch) (str-cat sig (str-cat (qch) "}"))))))))))))))))))
'''

# 1) helpers before the LAST (real) dispatcher definition
anchor_run = '(define (run input)\n  (let* ((cs (json-get-str "case" input))'
assert s.count(anchor_run) == 1, f"run anchor count={s.count(anchor_run)}"
i = s.rindex(anchor_run)
s = s[:i] + HELPERS + '\n' + s[i:]

# 2) wrap the if-chain with the two new cases (inserted above csw=83)
old_if = '(pkx-in (json-get-str "pk" input)))\n    (if (= csw 83)'
assert s.count(old_if) == 1, f"if-chain anchor count={s.count(old_if)}"
new_if = ('(pkx-in (json-get-str "pk" input)))\n'
          '    (if (= csw 78)\n'
          '        (nostr-derive (json-get-str "root" input) (json-get-str "caller" input))\n'
          '        (if (= csw 69)\n'
          '            (nostr-sign (json-get-str "pk" input) (json-get-str "sk" input)\n'
          '                        (json-get-str "ts" input) (json-get-str "kind" input)\n'
          '                        (json-get-str "content" input) (byte-at (json-get-str "df" input) 0))\n'
          '    (if (= csw 83)')
s = s.replace(old_if, new_if)

# 3) two extra closing parens for the two new ifs (unique terminal of the chain)
old_end = '(cat8l W24)))))))))))'
assert s.count(old_end) == 1, f"chain-end anchor count={s.count(old_end)}"
s = s.replace(old_end, '(cat8l W24)))))))))))))')

# 4) sanity: whole-file paren balance unchanged net of helpers (both must be 0)
def balance(t):
    return t.count('(') - t.count(')')
assert balance(s) == balance(open(SRC).read()) == 0, \
    f"balance drifted: {balance(s)} vs {balance(open(SRC).read())}"

open(DST, 'w').write(s)
print(f"OK wrote {DST} ({len(s)} bytes, balance={balance(s)})")
