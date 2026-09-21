#!/usr/bin/env python3
"""fix_affy.py (v3) — aff-y -> Y/Z^3; ALL parity indexes 7 -> 8 (5 sites)."""
import ast

G = '/tmp/nostr_probe/gen_bip340_sign.py'
s = open(G).read()

AFFY_OLD = "(define (aff-y X Y Z)\n  (let ((Zi (fe-pow Z (c-pm2 0))))\n    (fm Y (fm Zi Zi))))"
AFFY_NEW = "(define (aff-y X Y Z)\n  (let ((Zi (fe-pow Z (c-pm2 0))))\n    (fm Y (fm Zi (fm Zi Zi)))))"
assert s.count(AFFY_OLD) == 1, "aff-y not found"
s = s.replace(AFFY_OLD, AFFY_NEW)

n1 = s.count("(vec-nth yw 7)")
n2 = s.count("(vec-nth ryw 7)")
assert (n1, n2) == (3, 2), f"unexpected parity site counts {(n1, n2)}"
s = s.replace("(vec-nth yw 7)", "(vec-nth yw 8)")
s = s.replace("(vec-nth ryw 7)", "(vec-nth ryw 8)")
assert s.count("(vec-nth yw 7)") == 0 and s.count("(vec-nth ryw 7)") == 0
assert s.count("(vec-nth yw 8)") == 3 and s.count("(vec-nth ryw 8)") == 2

ast.parse(s)
open(G, 'w').write(s)
print("aff-y -> Z^3; parity 7->8 at 5 sites (yw x3, ryw x2); ast clean")
