#!/usr/bin/env python3
"""lp_pnl_probe.py — functional probe for the op "pnl" LP view (branch
lp-pnl-view).

Validates, against the exact-integer oracle (same G/dy/dx closed forms
as clmm_v5_pool_test.py):
  1. composition x,y matches Sc-clamped kernel forms at current S
  2. HODL matches reqx/reqy closed forms marked at P = (S·S)>>64
  3. V = y + (x·P)>>64  (quote-grade; tolerance ≤ 2 units of >>64 dust)
  4. il_bp == (V·10⁴)//HODL − 10⁴  (floor, signed)
  5. pnl == V + fx·P + fy − HODL  (parity path: fx = fy = 0)
  6. ZERO-WRITE: pool storage rows bit-identical before/after the pnl call
"""
import json, math, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SRC = os.path.join(ROOT, "lib")
MOCK = os.path.join(ROOT, "target", "debug", "near-mock")
COMPILE = os.path.join(ROOT, "target", "debug", "near-compile")

TWO128 = 1 << 128
TWO64 = 1 << 64

def _G(i):
    if i >= 0:
        num = pow(10001, i) * TWO128 // pow(10000, i)
    else:
        num = pow(10000, -i) * TWO128 // pow(10001, -i)
    return math.isqrt(num)

def G(i):
    return _G(i)

def dy(L, S, S2): return (L * (S2 - S)) >> 64
def dx(L, S, S2): return (L * (S2 - S)) // (S * S2)

def reqx(a, b, d):  # sum of dx(d, G(i), G(i+1)) over cells a..b INCLUSIVE
    return sum(dx(d, G(i), G(i + 1)) for i in range(a, b + 1))
def reqy(a, b, d):
    return sum(dy(d, G(i), G(i + 1)) for i in range(a, b + 1))

def call(state, wasm, op, args, signer="alice"):
    manifest = f"pool={wasm}"
    a = dict(args); a["op"] = op
    env = dict(os.environ); env["NEAR_MOCK_SIGNER"] = signer
    r = subprocess.run([MOCK, "cross", state, manifest, "pool", "_run", json.dumps(a)],
                       capture_output=True, text=True, env=env, timeout=120)
    out = r.stdout + r.stderr
    if r.returncode != 0:
        sys.exit(f"{op} failed: {out[-500:]}")
    ret = None
    for line in out.splitlines():
        if "RETURN" in line:
            ret = line.split("RETURN", 1)[-1].lstrip(" :").strip()
        elif "📄" in line:
            ret = line.split("📄", 1)[1].strip()
    return ret

def pool_rows(state):
    r = subprocess.run([MOCK, "state", "dump", state], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"state dump failed: {r.stderr[-300:]}")
    rows = json.loads(r.stdout)
    out = {}
    import base64
    for row in rows:
        if row.get("account") == "pool":
            k = base64.b64decode(row["key"]).decode(errors="replace")
            v = row.get("value")
            if v is not None:
                v = base64.b64decode(v).decode(errors="replace")
            out[k] = v
    return out

def build():
    concat, wasm = "/tmp/lppnl.lisp", "/tmp/lppnl.wasm"
    with open(concat, "w") as f:
        for part in ("limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"):
            with open(os.path.join(SRC, part)) as g:
                f.write(g.read()); f.write("\n")
    r = subprocess.run([COMPILE, concat, wasm], capture_output=True, text=True)
    out = r.stdout + r.stderr
    if r.returncode != 0 or "✅" not in out:
        print(out[-3000:]); sys.exit("build failed")
    print(f"built {wasm}")
    return wasm

def signed(s):
    return -int(s[1:]) if s.startswith("-") else int(s)

def main():
    wasm = build()
    state = "/tmp/lppnl_state.bin"
    for p in (state, state + ".manifest.json"):
        if os.path.exists(p): os.remove(p)

    A, B, D = -2, 3, 10**16   # small span, sane liquidity
    call(state, wasm, "init", {})
    call(state, wasm, "mint", {"d": str(D), "a": A, "b": B})
    # walk the price UP through part of the range (parity gross path)
    call(state, wasm, "swap", {"neg": "0", "v": str(10**15)})

    before = pool_rows(state)
    ret = call(state, wasm, "pnl", {"who": "alice", "a": A, "b": B})
    after = pool_rows(state)
    print("pnl view:", ret)
    assert before == after, "ZERO-WRITE violated: storage changed"
    print("zero-write ✓")

    f = ret.split(":")
    assert f[0] == "pnl" and len(f) == 10, f"bad shape: {ret}"
    d, x, y, fx, fy, V, hodl, il, pnl = f[1:]

    # oracle
    S = int(before["6|S"])
    ga, gb = G(A), G(B)
    sc = min(max(S, ga), gb)
    ox = dx(D, sc, gb)
    oy = dy(D, ga, sc)
    P = (S * S) >> 64
    oV = oy + ((ox * P) >> 64)
    orx, ory = reqx(A, B, D), reqy(A, B, D)
    ohodl = ory + ((orx * P) >> 64)
    oil = (oV * 10000) // ohodl - 10000
    opnl = oV - ohodl  # parity path: no fees

    checks = [
        ("d", int(d), D),
        ("x", int(x), ox),
        ("y", int(y), oy),
        ("fx", int(fx), 0),   # parity mint: no FS markers
        ("fy", int(fy), 0),
        ("V", int(V), oV),
        ("HODL", int(hodl), ohodl),
        ("il_bp", signed(il), oil),
        ("pnl", signed(pnl), opnl),
    ]
    ok = True
    for name, got, want in checks:
        tol = 2 if name in ("V", "HODL", "pnl") else 0
        good = abs(got - want) <= tol
        ok &= good
        print(f"  {name:5s} got {got:>25} want {want:>25}  {'✓' if good else '✗'}")
    print("ALL ✓" if ok else "MISMATCH")
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
