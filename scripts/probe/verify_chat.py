#!/usr/bin/env python3
"""Verify a chat-agent v4 event: recompute the NIP-01 id and check the BIP-340
sig using the primitives from scripts/probe/bip340_ref.py (pure stdlib).
usage: verify_chat.py <pk> <id> <sig> <ts> <room> <nonce> <content>
"""
import sys, hashlib, importlib.util

spec = importlib.util.spec_from_file_location("bip340", "scripts/probe/bip340_ref.py")
bip340 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bip340)

P, N, Gx, Gy = bip340.P, bip340.N, bip340.Gx, bip340.Gy
padd, pmul, tag = bip340.padd, bip340.pmul, bip340.tag

pk, eid, sig, ts, room, nonce, content = sys.argv[1:8]
ser = '[0,"%s",%s,1,[["t","%s"],["nonce","%s"]],"%s"]' % (pk, ts, room, nonce, content)
cid = hashlib.sha256(ser.encode()).hexdigest()
print("id recomputed:", cid)
print("id match     :", cid == eid)

# lift x-only pk onto the curve
px = int(pk, 16)
y_sq = (pow(px, 3, P) + 7) % P
ty = pow(y_sq, (P + 1) // 4, P)
assert ty * ty % P == y_sq, "pk x not on curve"
py = ty if ty % 2 == 0 else P - ty

r = int(sig[:64], 16); s = int(sig[64:], 16)
e = int.from_bytes(tag(bytes.fromhex(sig[:64]) + bytes.fromhex(pk) + bytes.fromhex(cid), b"BIP0340/challenge"), 'big') % N
# s·G − e·P must equal (r, even y)
sG = pmul(s)
neP = pmul(e, (px, py))
neP = (neP[0], (P - neP[1]) % P)
Rv = padd(sG, neP)
ok = Rv[0] == r and Rv[1] % 2 == 0
print("sig valid    :", ok)
sys.exit(0 if (cid == eid and ok) else 1)
