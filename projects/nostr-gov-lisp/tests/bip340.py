#!/usr/bin/env python3
"""bip340.py — pure-python BIP-340 sign/verify for lisp-rlm differential tests.

Signs exactly like nostr-gov's Rust path: verification is
schnorr_verify(pk32, sig64, SHA256(message)). Self-checked against
BIP-340 spec vectors AND nostr-gov's embedded k256 pair.
"""
import hashlib

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)

def sha(b): return hashlib.sha256(b).digest()

def th(tag, msg):
    t = sha(tag.encode())
    return sha(t + t + msg)

def add(p, q):
    if p is None: return q
    if q is None: return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0: return None
    if p == q: lam = (3 * p[0] * p[0]) * pow(2 * p[1], -1, P) % P
    else: lam = (q[1] - p[1]) * pow(q[0] - p[0], -1, P) % P
    x = (lam * lam - p[0] - q[0]) % P
    return (x, (lam * (p[0] - x) - p[1]) % P)

def mul(k, p=G):
    r = None
    while k:
        if k & 1: r = add(r, p)
        p = add(p, p)
        k >>= 1
    return r

def i2b(x): return x.to_bytes(32, 'big')
def b2i(b): return int.from_bytes(b, 'big')

def sign(sk, msg32, aux=bytes(32)):
    d = b2i(sk) % N
    if d == 0: raise ValueError("zero sk")
    Pp = mul(d)
    dp = d if Pp[1] % 2 == 0 else N - d
    t = i2b(dp ^ b2i(th("BIP0340/aux", aux)))
    rand = th("BIP0340/nonce", t + i2b(Pp[0]) + msg32)
    kp = b2i(rand) % N
    if kp == 0: raise ValueError("bad nonce")
    R = mul(kp)
    k = kp if R[1] % 2 == 0 else N - kp
    e = b2i(th("BIP0340/challenge", i2b(R[0]) + i2b(Pp[0]) + msg32)) % N
    return i2b(R[0]) + i2b((k + e * dp) % N)

def verify(pk, sig, msg32):
    px = b2i(pk)
    if px >= P: return False
    r, s = b2i(sig[:32]), b2i(sig[32:])
    if r >= P or s >= N: return False
    e = b2i(th("BIP0340/challenge", sig[:32] + pk + msg32)) % N
    R = add(mul(s), mul(N - e, (px, sqrt_y(px))))
    if R is None or R[1] % 2 != 0 or R[0] != r: return False
    return True

def sqrt_y(x):
    y = pow(x**3 + 7, (P + 1) // 4, P)
    assert y * y % P == (x**3 + 7) % P
    if y % 2: y = P - y  # BIP-340: even y
    return y

if __name__ == "__main__":
    # ── self-check 1: BIP-340 spec vector 0 ──
    sk = bytes.fromhex("0000000000000000000000000000000000000000000000000000000000000003")
    msg = bytes.fromhex("0000000000000000000000000000000000000000000000000000000000000000")
    sig = sign(sk, msg)
    pk = i2b(mul(b2i(sk))[0])
    assert pk.hex().upper() == "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9", pk.hex()
    assert verify(pk, sig, msg), "self-verify failed"
    print("spec-vector pk ok, self-verify ok")
    print("sig:", sig.hex())

    # ── self-check 2: nostr-gov embedded pair (sk=[0xaa]*32, msg "test") ──
    sk2 = bytes([0xaa] * 32)
    m2 = sha(b"test")  # nostr-gov: msg_hash = SHA256(message)
    pk2 = i2b(mul(b2i(sk2))[0])
    assert pk2.hex() == "6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3", pk2.hex()
    print("nostr-gov pk match ok")


# ── NIP-01 governance events (kind 37500) ──────────────────────────
import json as _json

GOV_KIND = 37500

def canonical_event(pubkey, created_at, kind, tags, content):
    """Exact nostr serialization: [0,pk,created_at,kind,tags,content]"""
    return _json.dumps([0, pubkey, created_at, kind, tags, content],
                       separators=(",", ":"))

def event_id(pubkey, created_at, kind, tags, content):
    return sha(canonical_event(pubkey, created_at, kind, tags, content).encode()).hex()

def sign_event(sk, pubkey, created_at, kind, tags, content):
    """BIP-340 sig over the event id (32 raw bytes)"""
    eid = sha(canonical_event(pubkey, created_at, kind, tags, content).encode())
    return sign(sk, eid).hex()

def gov_event(sk, pubkey, action, nonce, expires_ns, contract, created_at=1,
              content="nostr-gov owner action", kind=GOV_KIND,
              action_override=None, contract_override=None):
    """Build a signed governance event arg-dict for the contract."""
    tags = [
        ["t", "nostr-gov"],
        ["action", action_override if action_override is not None else action],
        ["nonce", str(nonce)],
        ["expires", str(expires_ns)],
        ["contract", contract_override if contract_override is not None else contract],
    ]
    sig = sign_event(sk, pubkey, created_at, kind, tags, content)
    return {
        "pk": pubkey,
        "ev": event_id(pubkey, created_at, kind, tags, content),
        "event_id_hex": event_id(pubkey, created_at, kind, tags, content),
        "ev": event_id(pubkey, created_at, kind, tags, content),
        "cat": str(created_at),
        "kind": str(kind),
        "tags": _json.dumps(tags, separators=(",", ":")),
        "ct": content,
        "sig": sig,
    }
