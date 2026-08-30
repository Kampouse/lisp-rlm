#!/usr/bin/env python3
"""diff-phase2.py — full Phase-1+2 lifecycle differential: lisp twin vs TS port.
Drives BOTH wasms through identical vm-run sequences (fresh state each):
wallet create (with deposit), set_approvers, propose, approve x2, execute,
and view reads — comparing returns, LOG traces, and abort codes.
"""
import json, os, subprocess, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "tests"))
from bip340 import sign, sha

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
VM = os.path.join(ROOT, "near-vm-run", "target", "release", "near-vm-run")
LISP = os.path.join(ROOT, "projects/nostr-gov-lisp/target/nostr-gov-lisp.wasm")
TS = "/tmp/gov-ts.wasm"
STATE = "/tmp/near-vm-run-state.bin"
CONTRACT = "escrow.test.near"
SK = bytes([0xAA] * 32)
from bip340 import i2b, mul, b2i
PK = i2b(mul(b2i(SK))[0]).hex()
ASK1 = bytes([0x01] * 32)
ASK2 = bytes([0x02] * 32)
APK1 = i2b(mul(b2i(ASK1))[0]).hex()
APK2 = i2b(mul(b2i(ASK2))[0]).hex()

NOW = 1787000000000000000  # pinned; every call runs with --ts NOW
EXP = NOW + 3600 * 10**9

def osig(action, nonce):
    m = f"expires {EXP}.000000000: {action} | nonce: {nonce} | contract: {CONTRACT}".encode()
    return sign(SK, sha(m)).hex()

def asig(sk, name, pid, ix):
    m = f"expires {EXP}.000000000: approve:{name}:{pid}:{ix} | contract: {CONTRACT}".encode()
    return sign(sk, sha(m)).hex()

def base(name=None):
    a = {"expires_at": str(EXP)}
    if name: a["name"] = name
    return a

# (method, args-json, view?, deposit?, label)
def steps():
    S = [
        ("init", {"npub": PK}, 0, 0, "init"),
        ("create_wallet", {**base("tgt1"), "signature": osig("create_wallet:tgt1", 0), "nonce": "0"}, 0, "2", "create"),
        ("get_wallet", {"name": "tgt1"}, 1, 0, "wallet"),
        ("set_approvers", {**base("tgt1"), "pks": f"{APK1},{APK2}", "thr": "2",
                           "signature": osig("set_approvers:tgt1", 1), "nonce": "1"}, 0, 0, "approvers"),
        ("propose", {**base("tgt1"), "pexp": str(NOW + 86400 * 10**9),
                     "am": "1000000", "rc": "rc.test.near",
                     "signature": osig("propose:tgt1:0", 2), "nonce": "2"}, 0, 0, "propose"),
        ("get_proposal", {"name": "tgt1", "id": "0"}, 1, 0, "p0"),
        ("approve", {"name": "tgt1", "id": "0", "ix": "0", "pubkey_hex": APK1,
                     "signature": asig(ASK1, "tgt1", "0", "0"),
                     "expires_at": str(EXP)}, 0, 0, "approve0"),
        # duplicate approval must fail identically
        ("approve", {"name": "tgt1", "id": "0", "ix": "0", "pubkey_hex": APK1,
                     "signature": asig(ASK1, "tgt1", "0", "0"),
                     "expires_at": str(EXP)}, 0, 0, "dup-approve0"),
        ("approve", {"name": "tgt1", "id": "0", "ix": "1", "pubkey_hex": APK2,
                     "signature": asig(ASK2, "tgt1", "0", "1"),
                     "expires_at": str(EXP)}, 0, 0, "approve1"),
        ("get_proposal", {"name": "tgt1", "id": "0"}, 1, 0, "p0-approved"),
        ("execute", {**base("tgt1"), "id": "0", "signature": osig("execute:tgt1:0", 3), "nonce": "3"}, 0, 0, "execute"),
        ("get_proposal", {"name": "tgt1", "id": "0"}, 1, 0, "p0-executed"),
        ("get_owner_nonce", {}, 1, 0, "nonce"),
        # second wallet via reused nonce must fail
        ("create_wallet", {**base("w2"), "signature": osig("create_wallet:w2", 3), "nonce": "3"}, 0, "2", "reuse-nonce"),
        ("create_wallet", {**base("w2"), "signature": osig("create_wallet:w2", 4), "nonce": "4"}, 0, "2", "wallet2"),
        ("get_wallet", {"name": "w2"}, 1, 0, "wallet2-view"),
    ]
    return S

def run(wasm):
    if os.path.exists(STATE): os.remove(STATE)
    out = []
    for m, a, view, dep, label in steps():
        cmd = [VM, wasm, m, json.dumps(a, separators=(",", ":")), "--ts", str(NOW)]
        if view: cmd.append("--view")
        if dep: cmd += ["--deposit", dep]
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        txt = r.stdout + r.stderr
        err = None
        for line in txt.splitlines():
            if "ERR_" in line:
                err = "ERR_" + line.split("ERR_")[1].split()[0]
        ret = ""
        for line in txt.splitlines():
            if line.startswith("📄"): ret = line[2:].strip()
        logs = [l for l in txt.splitlines() if "LOG:" in l]
        out.append((label, err, ret, logs))
    return out

la = run(LISP)
ta = run(TS)
bad = 0
for (l1, e1, r1, g1), (l2, e2, r2, g2) in zip(la, ta):
    if (e1, r1, g1) != (e2, r2, g2):
        bad += 1
        print(f"❌ {l1}: lisp=({e1 or r1!r}) ts=({e2 or r2!r})")
        if g1 != g2:
            print("   lisp logs:", g1)
            print("   ts logs:  ", g2)
    else:
        v = e1 if e1 else r1
        print(f"✅ {l1}: {v}")
print(f"── {len(la)-bad}/{len(la)} identical" + (" — TRACE-EQUIVALENT" if bad == 0 else ""))
sys.exit(1 if bad else 0)
