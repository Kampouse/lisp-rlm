#!/usr/bin/env python3
"""Pure-stdlib BIP-340 reference signer — ground truth for debugging the
Rust sign path in ../../../schnorr/. Usage:
  bip340_ref.py <sk_hex> <msg_hex> [aux_hex]   -> pk, sig, self-verify
"""
import hashlib

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8

def inv(a, m): return pow(a, -1, m)
def padd(p, q):
    if p is None: return q
    if q is None: return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0: return None
    if p == q: l = 3 * p[0] * p[0] * inv(2 * p[1], P) % P
    else: l = (q[1] - p[1]) * inv(q[0] - p[0], P) % P
    x = (l * l - p[0] - q[0]) % P
    return (x, (l * (p[0] - x) - p[1]) % P)
def pmul(k, p=(Gx, Gy)):
    r = None
    while k:
        if k & 1: r = padd(r, p)
        p = padd(p, p); k >>= 1
    return r
def tag(msg, tagname):
    t = hashlib.sha256(tagname).digest()
    return hashlib.sha256(t + t + msg).digest()
def sign(sk, msg, aux):
    d = sk % N
    px, py = pmul(d)
    dp = d if py % 2 == 0 else N - d
    t = bytes(a ^ b for a, b in zip(dp.to_bytes(32, 'big'), tag(aux, b"BIP0340/aux")))
    rand = tag(t + px.to_bytes(32, 'big') + msg, b"BIP0340/nonce")
    k = int.from_bytes(rand, 'big') % N
    rx, ry = pmul(k)
    k = k if ry % 2 == 0 else N - k
    e = int.from_bytes(tag(rx.to_bytes(32, 'big') + px.to_bytes(32, 'big') + msg, b"BIP0340/challenge"), 'big') % N
    s = (k + e * dp) % N
    return rx.to_bytes(32, 'big') + s.to_bytes(32, 'big')

if __name__ == "__main__":
    import sys
    sk = int(sys.argv[1], 16)
    msg = bytes.fromhex(sys.argv[2])
    aux = bytes.fromhex(sys.argv[3]) if len(sys.argv) > 3 else b"\x00" * 32
    sig = sign(sk, msg, aux)
    pk = pmul(sk)[0].to_bytes(32, 'big')
    print("pk: ", pk.hex())
    print("sig:", sig.hex())
