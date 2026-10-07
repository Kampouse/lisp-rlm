#!/usr/bin/env python3
"""NTT-on-NEAR demo harness — drives ntt.wasm in near-mock.

Simulates the Solana-side peer + Wormhole guardians in python, exactly as
the real system would: the peer observes the burn, guardians attestate the
observation (secp256k1 over keccak256(body)), NEAR verifies + mints.
"""
import json, subprocess, sys, time
import ecdsa
from ecdsa.util import sigencode_string_canonize
from Crypto.Hash import keccak

MOCK = "../target/release/near-mock"
WASM = "ntt.wasm"
subprocess.run([MOCK, WASM, "reset"], capture_output=True, text=True)

def call(method, args=None, view=False, advance=0, expect_fail=False):
    cmd = [MOCK, WASM, method, json.dumps(args or {}), "--once", "--now", "1762300000"]
    if view: cmd.append("--view")
    if advance: cmd += ["--now", "1762300000", "--advance", str(advance)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    out = r.stdout
    ret = None
    for line in out.splitlines():
        if line.startswith("📄"): ret = line[1:].strip()
        if "caused by" in line: ret = ("ERR:" + line.split("caused by:")[1].strip()[:60])
    print(f"  {method}({json.dumps(args or {})[:70]}{'…' if len(json.dumps(args or {}))>70 else ''}) -> {ret}")
    return ret

# ── guardian keys + addresses ──────────────────────────────────
def kec(b): return keccak.new(digest_bits=256).update(b).digest()
def addr_of(sk): return kec(sk.get_verifying_key().to_string())[-20:].hex()  # eth addr = keccak(pk64)[12:]

G = [ecdsa.SigningKey.from_string(bytes([0x11]*32), curve=ecdsa.SECP256k1),
     ecdsa.SigningKey.from_string(bytes([0x22]*32), curve=ecdsa.SECP256k1),
     ecdsa.SigningKey.from_string(bytes([0x33]*32), curve=ecdsa.SECP256k1)]
ADDRS = [addr_of(g) for g in G]
EMITTER = kec(b"fake-solana-ntt-manager").hex()   # 32B peer emitter addr
n = ecdsa.SECP256k1.order; Gp = ecdsa.SECP256k1.generator
curve_p = ecdsa.SECP256k1.curve
CURVE_P = ecdsa.SECP256k1.curve.p()

def sign_with_recid(sk, digest):
    import ecdsa.numbertheory as nt
    sig = sk.sign_digest_deterministic(digest, sigencode=sigencode_string_canonize)
    r = int.from_bytes(sig[:32],'big'); s_ = int.from_bytes(sig[32:],'big')
    z = int.from_bytes(digest,'big')
    order = ecdsa.SECP256k1.order
    Gp = ecdsa.SECP256k1.generator
    P = ecdsa.SECP256k1.curve.p(); A = ecdsa.SECP256k1.curve.a(); B = ecdsa.SECP256k1.curve.b()
    true_pk = b'\x04' + sk.get_verifying_key().to_string()
    for recid in range(4):
        x = r + (order if recid >= 2 else 0)
        if x >= P: continue
        alpha = (pow(x,3,P) + A*x + B) % P
        y = nt.square_root_mod_prime(alpha, P)
        if y % 2 != recid % 2: y = P - y
        R = ecdsa.ellipticcurve.Point(ecdsa.SECP256k1.curve, x, y, order)
        Q = nt.inverse_mod(r, order) * (s_*R + (-z % order)*Gp)
        if b'\x04' + ecdsa.VerifyingKey.from_public_point(Q, curve=ecdsa.SECP256k1).to_string() == true_pk:
            return sig, recid
    raise RuntimeError("no recid")

def build_vaa(payload: bytes, signer_idx=(0,1), gs_idx=0, chain=1, emitter=EMITTER, seq=1, ts=1762300000):
    body = (ts.to_bytes(4,'big') + (0).to_bytes(4,'big') + chain.to_bytes(2,'big')
            + bytes.fromhex(emitter) + seq.to_bytes(8,'big') + b'\x05' + payload)
    digest = kec(body)
    sigs = b''
    present = [i for i in signer_idx]
    for i in range(3):
        if i in present:
            sig, recid = sign_with_recid(G[i], digest)
            sigs += sig + bytes([recid])
        else:
            sigs += b'\x00'*65
    vaa = bytes([1]) + gs_idx.to_bytes(4,'big') + bytes([3]) + sigs + body
    return vaa.hex()

def ntt_payload(amount: int, recipient: str) -> bytes:
    r = recipient.encode()
    return bytes([1]) + amount.to_bytes(32,'big') + bytes([len(r)]) + r

# ── the demo ───────────────────────────────────────────────────
print("== init (3 guardians, threshold 2, emitter chain 1, in_cap 1000)")
call("init", {"g0": ADDRS[0], "g1": ADDRS[1], "g2": ADDRS[2],
              "threshold": "2", "em1": EMITTER, "in_cap": "1000"})

print("== faucet mint 1000 -> alice")
ALICE = "owner.test.near"  # default mock predecessor
call("mint_to", {"account_id": ALICE, "amount": "1000"})

print("== outbound: alice burns 100 (peer observes payload)")
r = call("transfer_out", {"amount": "100", "recipient": "bob"})
assert r and "SEQ=1" in r, r
print("  payload:", r.split("PAYLOAD=")[1])

print("== INBOUND happy path: 2-of-3 quorum VAA mints bob += 100")
vaa = build_vaa(ntt_payload(100, "bob"), signer_idx=(0,1), seq=1)
r = call("receive_vaa", {"vaa": vaa}); assert r and "MINTED" in r, r
call("ft_balance_of", {"account_id": "bob"}, view=True)

print("== replay attack: same VAA again -> must fail")
r = call("receive_vaa", {"vaa": vaa}); assert r and "ERR_REPLAY" in r, r

print("== quorum attack: only guardian 2 signs (1-of-3 < 2)")
vaa_badq = build_vaa(ntt_payload(50, "mallory"), signer_idx=(2,), seq=2)
r = call("receive_vaa", {"vaa": vaa_badq}); assert r and "ERR_QUORUM" in r, r

print("== emitter spoof: right quorum, wrong emitter address")
vaa_bade = build_vaa(ntt_payload(50, "mallory"), signer_idx=(0,1), seq=3,
                     emitter=kec(b"impostor").hex())
r = call("receive_vaa", {"vaa": vaa_bade}); assert r and "ERR_EMITTER" in r, r

print("== rate limit: 600 more (used=100+600=700 <= 1000) ok, then 500 -> QUEUED")
r = call("receive_vaa", {"vaa": build_vaa(ntt_payload(600, "carol"), signer_idx=(0,1), seq=4)})
assert r and "MINTED" in r, r
r = call("receive_vaa", {"vaa": build_vaa(ntt_payload(500, "carol"), signer_idx=(0,1), seq=5)})
assert r and "QUEUED" in r, r
call("get_status", view=True)

print("== release after window (+70s): queued 500 mints")
r = call("release_inbound", {}, advance=70)
assert r and "MINTED" in r, r
call("ft_balance_of", {"account_id": "carol"}, view=True)
call("ft_balance_of", {"account_id": ALICE}, view=True)
call("get_status", view=True)
print("\nALL NTT SCENARIOS PASSED ✅")
