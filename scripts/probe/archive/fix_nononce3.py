#!/usr/bin/env python3
"""fix_nononce3.py — TRUE BIP-340 nonce: t = bytes(d') XOR tag_hash("BIP0340/aux", aux)
k' = int(tag_hash("BIP0340/nonce", t || pk || m)) mod n.  (Per bip-0340/reference.py.)"""
import ast, sys

P = '/tmp/nostr_probe/gen_bip340_sign.py'
s = open(P).read()

def rep(old, new, label):
    global s
    n = s.count(old)
    if n != 1:
        print(f"FAIL ({n}x) :: {label}"); sys.exit(1)
    s = s.replace(old, new)
    print(f"ok: {label}")

# 1) BIP template phase-B (duse precedes auxb here — direct)
rep('''         (t0 (hex-decode (tag-hash-fixed "BIP0340/nonce" pkb 32)))
         (tb (xorb-str t0 auxb))''',
    '''         (hab (hex-decode (tag-hash-fixed "BIP0340/aux" auxb 32)))
         (db (hex-decode (words-hex (fe-words-be duse))))
         (tb (xorb-str db hab))''',
    "BIP phase-B: t = d' XOR tag_aux(aux)")

# 2) b_binds: d/duse must precede the nonce block; move them up
rep('''        '         (auxb (let ((ah (json-get-str "aux" input))) (if (= (str-len ah) 64) (hex-decode ah) (c-zb 0))))\\n'
        '         (t0 (hex-decode (tag-hash-fixed "BIP0340/nonce" (hex-decode pkhex) 32)))\\n'
        '         (tb (xorb-str t0 auxb))\\n''',
    '''        '         (d (sc-from-hex (json-get-str "sk" input)))\\n'
        '         (duse (if (= pflip 1) (sc-negv d) d))\\n'
        '         (auxb (let ((ah (json-get-str "aux" input))) (if (= (str-len ah) 64) (hex-decode ah) (c-zb 0))))\\n'
        '         (hab (hex-decode (tag-hash-fixed "BIP0340/aux" auxb 32)))\\n'
        '         (db (hex-decode (words-hex (fe-words-be duse))))\\n'
        '         (tb (xorb-str db hab))\\n''',
    "b_binds: d/duse hoisted + t = d' XOR tag_aux(aux)")

# 3) remove the now-duplicate late d/duse in b_binds
rep('''        '         (d (sc-from-hex (json-get-str "sk" input)))\\n'
        '         (duse (if (= pflip 1) (sc-negv d) d))\\n'
        '         (eM (sc-from-hex ch))\\n''',
    '''        '         (eM (sc-from-hex ch))\\n''',
    "b_binds: drop late d/duse dup")

# 4) python judge mirrors true spec
rep('''tn = tagged_hash("BIP0340/nonce", pk_b)
t = bytes(a ^ b for a, b in zip(tn, aux_hex))''',
    '''tn = tagged_hash("BIP0340/aux", aux_hex)
t = bytes(a ^ b for a, b in zip(du.to_bytes(32, 'big'), tn))''',
    "judge: t = d' XOR tag_aux(aux)")

ast.parse(s)
open(P, 'w').write(s)
print("generator now on true spec nonce")
