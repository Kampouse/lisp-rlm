#!/usr/bin/env python3
"""bip340-runner — local end-to-end Schnorr pipeline over inlayer.

sign  = 2 inlayer runs (A: pk derive, B: R + s) on bip340.wasm
verify = 1 inlayer run on bip340_verify.wasm
Usage: bip340-runner.py sign <sk_hex> <msg_hex> [aux_hex]
       bip340-runner.py verify <pk_hex> <sig_hex> <msg_hex>
       bip340-runner.py roundtrip <sk_hex> <msg_hex>
"""
import json, subprocess, sys

INLAYER = "/Users/asil/.local/bin/inlayer"
SIGN = "/tmp/nostr_probe/bip340.wasm"
VERIFY = "/tmp/nostr_probe/bip340_verify.wasm"


def run(wasm, payload):
    r = subprocess.run([INLAYER, "run", wasm, "-i", payload],
                       capture_output=True, text=True, timeout=300)
    out = None
    for line in (r.stdout or "").splitlines():
        if line.startswith("📤 Output:"):
            out = line.split("📤 Output:", 1)[1].strip()
    if out is None:
        raise RuntimeError(f"inlayer failed: {(r.stderr or r.stdout)[-400:]}")
    return out


def hexpad(s, n):
    return s.rjust(n, "0")


def sign(sk_hex, msg_hex, aux_hex=None):
    sk_hex = hexpad(sk_hex, 64)
    msg_hex = hexpad(msg_hex, 64)
    aux_hex = hexpad(aux_hex or "00", 64)
    # Run A: pk derive -> 64-hex pk + 1 parity-flip char (d'=n-d when odd-y)
    out_a = run(SIGN, json.dumps({"case": "D", "dbg": "P", "sk": sk_hex}))
    pkhex, flip = out_a[:64], out_a[64:65]
    # Run B: R + s (k = nonce, spec construction, aux-bound)
    out_b = run(SIGN, json.dumps({
        "case": "D", "dbg": "*", "sk": sk_hex, "msg": msg_hex,
        "pk": pkhex, "df": flip, "aux": aux_hex}))
    rhex, shex = out_b[:64], out_b[64:128]
    return {"pk": pkhex.lower(), "sig": (rhex + shex).lower()}


def verify(pk_hex, sig_hex, msg_hex):
    # dispatcher reads pk/r/s/msg (no aux -- verify never uses it)
    out = run(VERIFY, json.dumps({
        "pk": hexpad(pk_hex, 64), "msg": hexpad(msg_hex, 64),
        "r": hexpad(sig_hex[:64], 64), "s": hexpad(sig_hex[64:128], 64)}))
    return out in ("1", "\x01")


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "sign":
        sig = sign(*sys.argv[2:4], *(sys.argv[4:5] or []))
        print(json.dumps(sig, indent=2))
    elif cmd == "verify":
        print("VALID" if verify(*sys.argv[2:5]) else "INVALID")
    elif cmd == "roundtrip":
        res = sign(sys.argv[2], sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else None)
        ok = verify(res["pk"], res["sig"], sys.argv[3].rjust(64, "0"))
        print(f"pk:  {res['pk']}")
        print(f"sig: {res['sig']}")
        print(f"verify: {'VALID ✅' if ok else 'INVALID ❌'}")
    else:
        print(__doc__)
