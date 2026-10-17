#!/usr/bin/env python3
"""lp_pnl_probe2.py — custody-path verification for op "pnl" (final).

Scale note (learned the hard way): the walk's integer dust behavior
dominates at small L — x-side cell capacity ≈ L·Δtick/2^128 needs
L ≳ 4e23 for ONE atom. Use Rust-test scale (D=1e32, span [−1,1]).

Checks vs independent oracles:
  O1 closed-form composition (driver's G/dy/dx math)
  O2 accrual spec: fx/fy = Σ_i ((FG_i − FS_i)·d) >> 64 from state dump
  O3 zero-write: pool storage bit-identical across the pnl call

Scenario: boot → seed escrows → custody mint [−1,1] d=1e32 → Y-sell
1.024e29 (the d94afbc2 params-test pin: FEEY = 1e26 exact) → pnl →
partial custody burn (burnc writes FS) → pnl again (unclaimed since FS).
"""
import base64, json, math, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SRC = os.path.join(ROOT, "lib")
MOCK = os.path.join(ROOT, "target", "debug", "near-mock")
COMPILE = os.path.join(ROOT, "target", "debug", "near-compile")
TWO128, TWO64 = 1 << 128, 1 << 64

def G(i):
    if i >= 0:
        num = pow(10001, i) * TWO128 // pow(10000, i)
    else:
        num = pow(10000, -i) * TWO128 // pow(10001, -i)
    return math.isqrt(num)

def dy(L, S, S2): return (L * (S2 - S)) >> 64
def dx(L, S, S2): return (L * (S2 - S)) // (S * S2)
def reqx(a, b, d): return sum(dx(d, G(i), G(i + 1)) for i in range(a, b + 1))
def reqy(a, b, d): return sum(dy(d, G(i), G(i + 1)) for i in range(a, b + 1))

def build():
    out = {}
    for name, parts in {
        "pool": ("limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"),
        "ft":   ("limb_math.lisp", "clmm_core.lisp", "ft2.lisp"),
    }.items():
        concat, wasm = f"/tmp/lpp2b_{name}.lisp", f"/tmp/lpp2b_{name}.wasm"
        with open(concat, "w") as f:
            for p in parts:
                with open(os.path.join(SRC, p)) as g:
                    f.write(g.read()); f.write("\n")
        r = subprocess.run([COMPILE, concat, wasm], capture_output=True, text=True)
        o = r.stdout + r.stderr
        if r.returncode != 0 or "✅" not in o:
            print(o[-3000:]); sys.exit(f"build {name} failed")
        out[name] = wasm
    return out

class World:
    def __init__(self, wasms):
        self.state = "/tmp/lpp2b_state.bin"
        for p in (self.state, self.state + ".manifest.json"):
            if os.path.exists(p): os.remove(p)
        self.manifest = f"pool={wasms['pool']},tokx={wasms['ft']},toky={wasms['ft']}"

    def cross(self, acct, method, args, signer="alice", pre="600"):
        env = dict(os.environ); env["NEAR_MOCK_SIGNER"] = signer
        r = subprocess.run([MOCK, "cross", self.state, self.manifest, acct, method,
                            json.dumps(args), "--prepaid", pre],
                           capture_output=True, text=True, env=env, timeout=180)
        out = r.stdout + r.stderr
        if r.returncode != 0 or "TRAPPED" in out or "LOG: ERR" in out:
            print(out[-600:]); sys.exit(f"{acct}.{method} failed")
        for line in out.splitlines():
            if "📄" in line:
                return line.split("📄", 1)[1].strip()
        return None

    def pool_op(self, op, args=None, signer="alice"):
        a = dict(args or {}); a["op"] = op
        return self.cross("pool", "_run", a, signer)

    def rows(self):
        r = subprocess.run([MOCK, "state", "dump", self.state], capture_output=True, text=True)
        out = {}
        for row in json.loads(r.stdout):
            k = base64.b64decode(row["key"]).decode(errors="replace")
            v = row.get("value")
            out[f'{row["account"]}|{k}'] = base64.b64decode(v).decode(errors="replace") if v else None
        return out

def accrual(rows, side, a, b, d):
    tot = 0
    for i in range(a, b + 1):
        fg = int(rows.get(f'pool|6|FG{side}:{i}', "0") or 0)
        fs = int(rows.get(f'pool|6|FS{side}:alice:{a}:{b}:{i}', "0") or 0)
        tot += ((fg - fs) * d) >> 64
    return tot

def signed(s): return -int(s[1:]) if s.startswith("-") else int(s)

def check(tag, got, want, tol=0):
    good = abs(got - want) <= tol
    print(f"  {tag:22s} got {got:>34} want {want:>34}  {'✓' if good else '✗'}")
    return good

def main():
    wasms = build()
    w = World(wasms)

    w.cross("tokx", "new", {}, "owner")
    w.cross("toky", "new", {}, "owner")
    assert w.pool_op("init", {"toka": "tokx", "tokb": "toky"}, signer="owner") == "init-ok"
    FUND = 250 * 10**27
    for tok in ("tokx", "toky"):
        w.cross(tok, "mint", {"to": "alice", "amt": str(FUND)}, "owner")
        w.cross(tok, "ft_transfer_call", {"receiver_id": "pool", "amount": "1000000000000000000", "msg": "seed"})

    A1, B1 = -1, 1
    D = 100 * 10**30   # 1e32 — Rust params-test scale
    req = w.pool_op("req", {"d": str(D), "a": A1, "b": B1})
    rx_s, ry_s = req.removeprefix("rx:").split(":ry:")
    w.cross("tokx", "ft_transfer_call", {"receiver_id": "pool", "amount": rx_s, "msg": f"m:{D}:{A1}:{B1}"})
    w.cross("toky", "ft_transfer_call", {"receiver_id": "pool", "amount": ry_s, "msg": f"m:{D}:{A1}:{B1}"})
    assert w.rows()[f"pool|6|POS:alice:{A1}:{B1}"] == str(D), "mint legs failed"

    # custody Y-sell: the params-test pin (FEEY = 1e26 exact under shift 8?
    # default shift 8 → skim = 1.024e29>>8 = 4e26 — assert against actual
    # state FEEY key instead of the shift-10 pin)
    w.cross("toky", "ft_transfer_call", {"receiver_id": "pool",
             "amount": "102400000000000000000000000000", "msg": "sw"})
    rows = w.rows()
    S = int(rows["pool|6|S"])
    assert S != TWO64, "swap did not move S"
    feey = int(rows.get("pool|6|FEEY", "0") or 0)
    assert feey == 102400000000000000000000000000 >> 8, f"FEEY pin: {feey}"

    ok = True
    before = rows
    r = w.pool_op("pnl", {"who": "alice", "a": A1, "b": B1})
    after = w.rows()
    assert before == after, "ZERO-WRITE violated"
    print("zero-write ✓  (post fee-swap)")
    f = r.split(":")
    assert f[0] == "pnl" and len(f) == 10, f"shape: {r}"
    d, x, y, fx, fy, V, hodl, il, pnl = f[1:]

    ga, gb = G(A1), G(B1)
    sc = min(max(S, ga), gb)
    P = (S * S) >> 64
    ox, oy = dx(D, sc, gb), dy(D, ga, sc)
    oV = oy + ((ox * P) >> 64)
    ohodl = reqy(A1, B1, D) + ((reqx(A1, B1, D) * P) >> 64)
    ofy = accrual(rows, "Y", A1, B1, D)
    ofx = accrual(rows, "X", A1, B1, D)
    print(f"A: post Y-sell (S={S}, FEEY={feey})")
    ok &= check("d", int(d), D)
    ok &= check("x", int(x), ox)
    ok &= check("y", int(y), oy)
    ok &= check("fx (spec)", int(fx), ofx)
    ok &= check("fy (spec)", int(fy), ofy)
    ok &= check("fy > 0", 1 if int(fy) > 0 else 0, 1)
    ok &= check("V", int(V), oV, tol=2)
    ok &= check("HODL", int(hodl), ohodl, tol=2)
    tot = oV + ((ofx * P) >> 64) + ofy
    ok &= check("pnl", signed(pnl), tot - ohodl, tol=4)

    # B: partial custody burn — burnc accrues fees into owed + writes FS;
    # fresh swap grows FG beyond FS → pnl shows the DELTA as unclaimed.
    w.pool_op("burnc", {"d": str(D // 2), "a": A1, "b": B1})
    w.cross("tokx", "ft_transfer_call", {"receiver_id": "pool", "amount": "10240000000000000000000000000", "msg": "sw"})
    rows = w.rows()
    S2 = int(rows["pool|6|S"])
    Dr = D - D // 2
    r = w.pool_op("pnl", {"who": "alice", "a": A1, "b": B1})
    f = r.split(":")
    d, x, y, fx, fy, V, hodl, il, pnl = f[1:]
    ofx = accrual(rows, "X", A1, B1, Dr)
    ofy = accrual(rows, "Y", A1, B1, Dr)
    sc2 = min(max(S2, ga), gb)
    print("B: post-burnc + X-sell (unclaimed = growth since FS)")
    ok &= check("d (post-burn)", int(d), Dr)
    ok &= check("x", int(x), dx(Dr, sc2, gb))
    ok &= check("y", int(y), dy(Dr, ga, sc2))
    ok &= check("fx (spec+FS)", int(fx), ofx)
    ok &= check("fy (spec+FS)", int(fy), ofy)

    print("ALL ✓" if ok else "MISMATCH")
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
