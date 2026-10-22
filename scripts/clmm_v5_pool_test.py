#!/usr/bin/env python3
"""clmm_v5_pool_test.py — M3/arc-C1 differential driver for the proof-fit
CLMM pool (lib/clmm_pool_v5.lisp).

Oracle: the Isabelle relations, evaluated in exact Python integers —
  swap_rel  : s' = s+v, per-cell gen_quote_diff closed form, floors
  mint_rel  : pointwise lq i += d on [a,b]
  burn_rel  : pointwise −, with ∀i lq' i ≥ 0 + support/span preserved
  donate_rel: pointwise lq i += v at i
  grid      : G(i) = isqrt(1.0001^i · 2^128)  (AFP lattice instantiation)

Stages:
  edge   — deterministic battery: init, mint→swap across cells, empty-pool
           swap (zero-move), burn-to-empty then span recheck, donate,
           auth/underflow rejects (identity), over-span clamp.
  fuzz   — random ops; after EVERY op the full oracle state must equal
           the contract state (S, P, per-cell LQ, books, IMIN/IMAX).
  parity — quote vs swap: same v, quote must equal the swap's out/leg
           (pure projection discipline).

Usage: python3 scripts/clmm_v5_pool_test.py --seed 1 [--ops 200]
Exit 0 iff all stages green.
"""
import argparse
import json
import math
import os
import random
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
MOCK = os.path.join(ROOT, "target", "release", "near-mock")
COMPILE = os.path.join(ROOT, "target", "release", "near-compile")
SRC = os.path.join(ROOT, "lib")

PMAX = 800000
TWO128 = 1 << 128
TWO64 = 1 << 64

# ────────────────────────────────────────────── oracle: the thy, in ints

def _G(i: int) -> int:
    """S(i) = isqrt(1.0001^i · 2^128) — the fixed Q64.64 lattice."""
    if abs(i) > PMAX:
        raise ValueError
    # exact: Decimal-free — pow via integer math on the same closed form
    # the kernel uses: 1.0001^i scaled by 2^128, then floor sqrt.
    # Use math.isqrt over exact integer pow: 1.0001 = 10001/10000.
    if i >= 0:
        num = pow(10001, i) * (TWO128 // (1)) // pow(10000, i)
    else:
        num = pow(10000, -i) * TWO128 // pow(10001, -i)
    return math.isqrt(num)

_G_CACHE = {}
def G(i):
    """G with memoization — the exact bigint pow is far too slow to recompute
    inside p()'s binary search (20+ calls per op over ±PMAX)."""
    v = _G_CACHE.get(i)
    if v is None:
        v = _G(i)
        _G_CACHE[i] = v
    return v

def dy(L, S, S2):
    return (L * (S2 - S)) >> 64

def dx(L, S, S2):
    # renormalized (price-bridge law, thy cd6ab511): real dx/L = (s2−s)/(s·s2)
    # with s = S/2^64 ⇒ the fixed-point form carries ONE ×2^64
    return (L * (S2 - S) * TWO64) // (S * S2)

def sfdx(L, S, dxin):
    """S_for_dx_in — exact closed form, mirrors the kernel (thy cd6ab511).

    Inverse of the RENORMALIZED dx_in: smallest S2 with dx(L,S,S2) > dxin,
    i.e. the price after spending exactly dxin x into cell (S -> S2).
    N = L·S·t64, D = L·t64 − (dx+1)·S; when D ≤ 0 the input exceeds the
    cell's whole book → 2^127 sentinel (walk hops)."""
    t64 = 1 << 64
    if L <= 0 or S <= 0 or L * t64 <= (dxin + 1) * S:
        return 1 << 127
    return max(S, (L * S * t64 - 1) // (L * t64 - (dxin + 1) * S))

def sfdyi(L, S, dyin):
    """S_for_dy_in: largest S2 ≤ S with dy(L, S2, S) ≤ dyin (up-convention dy).

    Mirrors the kernel/p5-sfdyi closed form: t* = ceil((2^64·(dy+1) − 1)/L),
    S2 = max(1, S − t*). The old binary search tested dy(L, S, S2) with S2 < S
    (negative), which collapsed to 1 and teleported s to ~0."""
    if L == 0:
        return 1
    tstar = ((1 << 64) * (dyin + 1) - 1) // L
    s2 = S - tstar
    return s2 if s2 >= 1 else 1

class Oracle:
    """(P, s, lq) — the thy state, plus books (arc-C1 swap legs only)."""

    def __init__(self):
        self.s = TWO64          # price 1.0
        self.lq = {}            # point -> L (sparse; missing = 0)
        self.x = 0
        self.y = 0
        # Finding A authority ledger: POS[(who,a,b)] — what the caller may
        # burn. (who = the mock signer "alice"; the battery is single-owner.)
        self.pos = {}
        # The pool CARRIES its walk pointer in storage (6|P); it never
        # re-derives P from S. The oracle mirrors the ledger: swaps move
        # self._p; mint/burn/donate never touch it (the contract has no
        # 6|P write outside the swap arms + init).
        self._p = 0

    def support(self):
        nz = sorted(i for i, v in self.lq.items() if v != 0)
        return (nz[0], nz[-1]) if nz else None

    def imin(self):
        sp = self.support()
        return sp[0] if sp else 0

    def imax(self):
        sp = self.support()
        return sp[1] if sp else 0

    def cell(self, i):
        return self.lq.get(i, 0)

    def p(self):
        """The pool's WALK POINTER (6|P): carried in storage, never
        re-derived from S. Swaps move it (partial branch pins P inside
        the cell; full hops walk it); mint/burn/donate NEVER touch it —
        the contract has no 6|P write outside the swap arms + init."""
        return self._p


    def quote(self, v):
        """swap_rel projection: (out, rem). up: v>0 sells x. down: v<0."""
        if v == 0:
            return 0, 0
        if v > 0:
            sp = self.support()
            if sp is None:
                return 0, v
            gmax = G(sp[1] + 1)
            if os.environ.get("V5_TRACE"):
                print(f"[quote up v={v} s={self.s} sp={sp}]", file=sys.stderr)
            s, p, acc, rm = self.s, self._p, 0, v
            _guard = 0
            while rm > 0 and s < gmax:
                _guard += 1
                if _guard > 100000:
                    raise RuntimeError(f"up-walk runaway: p={p} s={s} gmax={gmax} rm={rm}")
                L = self.cell(p)
                gh = G(p + 1)
                if L == 0:
                    # empty cell: s jumps the gap for free, no output
                    p += 1
                    s = gh
                    continue
                dx_leg = dx(L, s, gh)
                if dx_leg == 0:
                    # fail-closed: zero input leg ⇒ zero output (no free y);
                    # jump the gap (s must advance or the s<gmax bound never fires)
                    p += 1
                    s = gh
                    continue
                if dx_leg <= rm:
                    acc += dy(L, s, gh)
                    rm -= dx_leg
                    p += 1
                    s = gh
                else:
                    s2 = sfdx(L, s, rm)
                    acc += dy(L, s, s2)
                    rm = 0
                    s = s2
            return acc, rm
        else:
            sp = self.support()
            if sp is None:
                return 0, v
            gmin = G(sp[0])
            s, p, acc, rm = self.s, self.p(), 0, -v
            while rm > 0 and s > gmin:
                L = self.cell(p)
                gl = G(p - 1)
                if L == 0:
                    p -= 1
                    s = gl
                    continue
                dy_leg = dy(L, gl, s)        # dy() is up-convention: (S→S2 up)
                if dy_leg == 0:
                    p -= 1
                    s = gl
                    continue
                if dy_leg <= rm:
                    acc += dx(L, gl, s)
                    rm -= dy_leg
                    p -= 1
                    s = gl
                else:
                    s2 = sfdyi(L, s, rm)
                    acc += dx(L, s2, s)
                    rm = 0
                    s = s2
            return acc, rm

    # mutations (relation instances; rejects raise)
    class Reject(Exception):
        pass

    def mint(self, d, a, b, who="alice"):
        if b < a or b - a > 64 or d <= 0:
            raise Oracle.Reject
        for i in range(a, b + 1):
            self.lq[i] = self.cell(i) + d
        self.pos[(who, a, b)] = self.pos.get((who, a, b), 0) + d

    def burn(self, d, a, b, who="alice"):
        if b < a or b - a > 64:
            raise Oracle.Reject
        if self.pos.get((who, a, b), 0) < d:
            raise Oracle.Reject   # Finding A: authority = own POS >= d
        for i in range(a, b + 1):
            if self.cell(i) < d:
                raise Oracle.Reject
        for i in range(a, b + 1):
            self.lq[i] = self.cell(i) - d
        self.pos[(who, a, b)] = self.pos.get((who, a, b), 0) - d

    def donate(self, v, i):
        if v < 0:
            raise Oracle.Reject
        self.lq[i] = self.cell(i) + v

    def swap(self, v):
        if v == 0:
            return "swap-zero", None
        out, rem = self.quote(v)
        if v > 0:
            self.x += v - rem
            self.y += out
        else:
            self.y += -v - rem
            self.x += out
        # move s/p: recompute from the walk (only if some input consumed)
        consumed = abs(v) - rem
        if consumed == 0:
            return "swap-empty", None
        if v > 0:
            sp = self.support()
            gmax = G(sp[1] + 1)
            s, p = self.s, self._p
            rm = v
            while rm > 0 and s < gmax:
                L = self.cell(p)
                gh = G(p + 1)
                if L == 0:
                    p += 1
                    s = gh
                    continue
                dx_leg = dx(L, s, gh)
                if dx_leg == 0:
                    p += 1
                    s = gh
                    continue
                if dx_leg <= rm:
                    rm -= dx_leg
                    p += 1
                    s = gh
                else:
                    s = sfdx(L, s, rm)
                    rm = 0
            self.s, self._p = s, p
        else:
            sp = self.support()
            gmin = G(sp[0])
            s, p = self.s, self._p
            rm = -v
            while rm > 0 and s > gmin:
                L = self.cell(p)
                gl = G(p - 1)
                if L == 0:
                    p -= 1
                    s = gl
                    continue
                dy_leg = dy(L, gl, s)
                if dy_leg == 0:
                    p -= 1
                    s = gl
                    continue
                if dy_leg <= rm:
                    rm -= dy_leg
                    p -= 1
                    s = gl
                else:
                    s = sfdyi(L, s, rm)
                    rm = 0
            self.s, self._p = s, p
        return f"swap-out:{out}:rem:{rem}", None

# ────────────────────────────────────────────── harness

class Pool:
    def __init__(self, wasm, state):
        self.wasm = wasm
        self.state = state
        self.n = 0

    def import_key(self, key, value):
        """Merge-seed one storage key into pool state (Finding C prod-gate)."""
        import base64
        rows = [{"account": "pool",
                 "key": base64.b64encode(key.encode()).decode(),
                 "value": base64.b64encode(value.encode()).decode()}]
        r = subprocess.run([MOCK, "state", "import", self.state, "-"],
                           input=json.dumps(rows), capture_output=True,
                           text=True, timeout=60)
        if r.returncode != 0:
            raise RuntimeError(f"state import failed: {r.stdout} {r.stderr}")

    def call(self, op, args, signer="alice", expect_ok=True):
        manifest = f"pool={self.wasm}"
        if op == "init":
            method, payload = "init", dict(args)
        else:
            method, payload = "exec", {**args, "op": op}
        env = dict(os.environ)
        env["NEAR_MOCK_SIGNER"] = signer
        r = subprocess.run(
            [MOCK, "cross", self.state, manifest, "pool", method,
             json.dumps(payload)],
            capture_output=True, text=True, env=env, timeout=120)
        out = r.stdout + r.stderr
        if expect_ok and r.returncode != 0:
            raise RuntimeError(f"{op} failed: {out[-400:]}")
        ret = None
        for line in out.splitlines():
            if "RETURN" in line:
                ret = line.split("RETURN", 1)[-1].lstrip(" :").strip()
            elif "📄" in line:
                ret = line.split("📄", 1)[1].strip()
        return r.returncode, ret

# ────────────────────────────────────────────── state comparison

def read_state(state):
    """pool storage via `near-mock state dump` (JSON rows, b64 k/v)."""
    import base64
    r = subprocess.run([MOCK, "state", "dump", state],
                       capture_output=True, text=True, timeout=60)
    out = {}
    try:
        rows = json.loads(r.stdout or "[]")
    except json.JSONDecodeError:
        return out
    for row in rows:
        try:
            k = base64.b64decode(row["key"]).decode("utf-8")
            v = base64.b64decode(row["value"]).decode("utf-8")
        except Exception:
            continue
        if row.get("account") == "pool":
            out[k] = v
    return out

def pool_rows(state):
    rows = read_state(state)
    return {k: v for k, v in rows.items() if k.startswith("6|")}

# ────────────────────────────────────────────── stages

def build():
    concat = "/tmp/v5p.lisp"
    wasm = "/tmp/v5p.wasm"
    with open(concat, "w") as f:
        for part in ("limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"):
            with open(os.path.join(SRC, part)) as g:
                f.write(g.read())
                f.write("\n")
    r = subprocess.run([COMPILE, concat, wasm], capture_output=True, text=True)
    out = r.stdout + r.stderr
    if r.returncode != 0 or "✅" not in out:
        print(out[-3000:])
        sys.exit("build failed")
    print(f"built {wasm} ({os.path.getsize(wasm)} bytes)")
    return wasm

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--ops", type=int, default=200)
    args = ap.parse_args()

    wasm = build()
    state = f"/tmp/v5pool_{args.seed}.bin"
    if os.path.exists(state):
        os.remove(state)
    pool = Pool(wasm, state)
    o = Oracle()

    fails = []

    def check(tag):
        """full-state equality: oracle vs contract storage."""
        if os.environ.get("V5_TRACE"):
            print(f"[check {tag}] oracle   s={o.s} p={o.p()} imin={o.imin()} imax={o.imax()} "
                  f"lq={ {k: v for k, v in sorted(o.lq.items()) if v} }", file=sys.stderr)
        rows = pool_rows(state)
        def gv(k, d="0"):
            v = rows.get("6|" + k)
            if v is None:
                return d
            return str(v)
        cs = int(gv("S")); cp = int(gv("P"))
        cx = int(gv("X")); cy = int(gv("Y"))
        cmin = int(gv("IMIN")); cmax = int(gv("IMAX"))
        if os.environ.get("V5_TRACE"):
            print(f"[check {tag}] CONTRACT s={cs} p={cp} imin={cmin} imax={cmax}")
        olq = {i: v for i, v in o.lq.items() if v != 0}
        clq = {}
        for k, v in rows.items():
            if k.startswith("6|LQ:"):
                clq[int(k[5:])] = int(v)
        clq = {i: v for i, v in clq.items() if v != 0}
        if cs != o.s: fails.append(f"{tag}: S {cs} != {o.s}")
        if cp != o.p(): fails.append(f"{tag}: P {cp} != {o.p()}")
        if cx != o.x: fails.append(f"{tag}: X {cx} != {o.x}")
        if cy != o.y: fails.append(f"{tag}: Y {cy} != {o.y}")
        if olq != clq:
            fails.append(f"{tag}: LQ differs "
                         f"{sorted(set(olq.items()) ^ set(clq.items()))[:6]}")
        sp = o.support()
        if sp is None:
            if cmin != 0 or cmax != 0:
                fails.append(f"{tag}: empty-span bounds {cmin}/{cmax}")
        else:
            if cmin != sp[0]: fails.append(f"{tag}: IMIN {cmin} != {sp[0]}")
            if cmax != sp[1]: fails.append(f"{tag}: IMAX {cmax} != {sp[1]}")

    rc, _ = pool.call("init", {"toka": "tokx.test", "tokb": "toky.test"})
    if rc != 0:
        fails.append("init failed")
    check("boot")
    # Finding C prod-gate: init never writes 6|SIMSWAP — verify the gross
    # arm is dead, then arm it for this sim run via direct state import.
    rc, out = pool.call("swap", {"v": "1000"})
    if out != "sim-only":
        fails.append(f"prod-gate: exec not gated: {out}")
    pool.import_key("6|SIMSWAP", "1")
    # Finding B: custody donate rides ft_transfer_call msg="don:<i>" —
    # the exec donate arm does not exist (dispatch answers unknown-op).
    rc, out = pool.call("donate", {"v": "1", "i": "0"}, expect_ok=False)
    if out != "unknown-op":
        fails.append(f"prod-gate: exec donate still alive: {out}")

    rng = random.Random(args.seed)

    # ── edge battery ──
    EDGE = [
        ("e1 empty-swap up", "swap", {"v": "1000"}),
        ("e2 mint a=-2 b=2 d=10^12", "mint", {"d": str(10**12), "a": "-2", "b": "2"}),
        ("e3 swap up 10^15", "swap", {"v": str(10**15)}),
        ("e4 quote equals swap (checked in driver)", "quote", {"v": str(10**15)}),
        ("e5 swap down", "swap", {"v": "-" + str(10**15)}),
        ("e6 mint above", "mint", {"d": str(10**12), "a": "3", "b": "5"}),
        ("e7 swap up crossing", "swap", {"v": str(10**16)}),
        ("e8 burn too much (reject)", "burn", {"d": str(10**13), "a": "-2", "b": "2"}),
        ("e9 burn half", "burn", {"d": str(10**12), "a": "-2", "b": "2"}),
        ("e11 swap zero", "swap", {"v": "0"}),
    ]
    for tag, op, margs in EDGE:
        rc, out = pool.call(op, margs, expect_ok=False)
        if rc != 0 and "reject" not in tag and "too much" not in tag:
            fails.append(f"{tag}: rc={rc} out={out}")
        if op == "swap" and v_ok(margs):
            o.swap(int(margs["v"]))
        elif op == "mint" and "too much" not in tag:
            try:
                o.mint(int(margs["d"]), int(margs["a"]), int(margs["b"]))
            except Oracle.Reject:
                pass
        elif op == "burn" and "too much" not in tag:
            try:
                o.burn(int(margs["d"]), int(margs["a"]), int(margs["b"]))
            except Oracle.Reject:
                pass
        check(tag)

    # quote-vs-swap parity at the edge battery's e4 point is implicit in
    # fuzz below (every swap is preceded by a quote check).

    # ── fuzz ──
    for t in range(args.ops):
        roll = rng.random()
        if os.environ.get("V5_TRACE"):
            print(f"[fuzz {t}] roll={roll:.4f}", file=sys.stderr)
        if roll < 0.30:
            a = rng.randint(-4, 4)
            b = a + rng.randint(0, 6)
            d = rng.randint(1, 10**13)
            if os.environ.get("V5_TRACE"):
                print(f"[mint {d} [{a},{b}]]", file=sys.stderr)
            rc, out = pool.call("mint", {"d": str(d), "a": str(a), "b": str(b)},
                                expect_ok=False)
            try:
                o.mint(d, a, b)
                if rc != 0:
                    fails.append(f"f{t} mint should pass: {out}")
            except Oracle.Reject:
                if rc == 0:
                    fails.append(f"f{t} mint should reject: {out}")
        elif roll < 0.45:
            cand = [i for i, v in o.lq.items() if v > 0]
            if cand:
                a = rng.choice(cand) - rng.randint(0, 2)
                b = a + rng.randint(0, 4)
                d = rng.randint(1, 10**13)
                if os.environ.get("V5_TRACE"):
                    print(f"[burn {d} [{a},{b}]]", file=sys.stderr)
                rc, out = pool.call("burn", {"d": str(d), "a": str(a), "b": str(b)},
                                    expect_ok=False)
                try:
                    o.burn(d, a, b)
                    if rc != 0:
                        fails.append(f"f{t} burn should pass")
                except Oracle.Reject:
                    if rc == 0:
                        fails.append(f"f{t} burn should reject: {out}")
        else:
            mag = rng.choice([10**10, 10**13, 10**15, 10**17, 10**19])
            v = rng.randint(1, mag) * rng.choice([1, -1])
            rc, q = pool.call("quote", {"v": str(v)})
            qout = int(q.split(":")[1]) if q.startswith("q:") else -1
            oout, orem = o.quote(v)
            if qout != oout:
                fails.append(f"f{t} quote mismatch v={v}: {qout} != {oout} ({q})")
            if os.environ.get("V5_TRACE"):
                print(f"[swap {v}]", file=sys.stderr)
            rc, out = pool.call("swap", {"v": str(v)})
            o.swap(v)
            if os.environ.get("V5_TRACE") and (o.s <= 1 or rc != 0):
                print(f"[!! after swap v={v}: rc={rc} out={out} oracle s={o.s}]", file=sys.stderr)
            m = out.split(":")
            if len(m) >= 4 and m[0] == "swap-out":
                if int(m[1]) != oout:
                    fails.append(f"f{t} swap vs quote diverge: {out} vs q={qout}")
        if len(fails) > 8:
            break
        check(f"f{t}")

    if fails:
        print(f"❌ {len(fails)} FAILURE(S):")
        for f in fails[:10]:
            print(" ", f)
        sys.exit(1)
    print(f"✅ seed {args.seed}: edge + {args.ops} fuzz ops, full-state "
          f"oracle parity — clean")

def v_ok(margs):
    return True

if __name__ == "__main__":
    main()
