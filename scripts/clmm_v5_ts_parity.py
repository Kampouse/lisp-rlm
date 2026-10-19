#!/usr/bin/env python3
"""CLMM v5 byte-parity harness: real wasm (near-mock) vs TS frontend twin.

Runs identical op sequences through (a) the compiled lisp trio in near-mock
and (b) fixtures/clmm_v5.ts under tsx, then byte-diffs the RET:/STATE: lines.
Gate: zero diffs. Any diff is a semantics drift between the contract and
the frontend representation.

Usage: python3 scripts/clmm_v5_ts_parity.py
Wasm: rebuilt from lib/ each run (trio concat, same as the Rust suites).
"""
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOCK = os.path.join(ROOT, "target", "release", "near-mock")
TSX = "/Users/asil/.hermes/hermes-agent/node_modules/.bin/tsx"
D = str(10**26)
S0 = str(2**64)
SEED = str(5 * 10**25)
FUND = str(10**30)
SWAP = str(10**21)
SWAP_BIG = str(10**23)
SKIM = str(10**21 - (10**21 >> 8))          # swap amt minus fee — quote(skim) law
SKIM2 = str(10**22 - (10**22 >> 8))

# scenarios: name → ops. rx=None ⇒ harness resolves via position_requirements
# on the mock first, then feeds the resolved amounts to both engines.
SCENARIOS = {
    "live_pool3": [
        {"op": "init", "toka": "tokx.tst", "tokb": "toky.tst", "fee_shift": "8", "s0": S0},
        {"op": "seed", "tok": "x", "amt": SEED},
        {"op": "seed", "tok": "y", "amt": SEED},
        {"op": "req", "d": D, "lo": -8, "hi": 8},
        {"op": "mint", "d": D, "lo": -8, "hi": 8, "rx": None, "ry": None},
        {"op": "quote", "amt": SKIM},
        {"op": "swap", "tok": "x", "amt": SWAP},
        {"op": "req", "d": D, "lo": 0, "hi": 8},
    ],
    "narrow_span": [
        {"op": "init", "toka": "tokx.tst", "tokb": "toky.tst", "fee_shift": "8", "s0": S0},
        {"op": "seed", "tok": "x", "amt": SEED},
        {"op": "seed", "tok": "y", "amt": SEED},
        {"op": "req", "d": D, "lo": 0, "hi": 8},
        {"op": "mint", "d": D, "lo": 0, "hi": 8, "rx": None, "ry": None},
        {"op": "swap", "tok": "x", "amt": SWAP},
    ],
    "swap_down": [
        {"op": "init", "toka": "tokx.tst", "tokb": "toky.tst", "fee_shift": "8", "s0": S0},
        {"op": "seed", "tok": "x", "amt": SEED},
        {"op": "seed", "tok": "y", "amt": SEED},
        {"op": "mint", "d": D, "lo": -8, "hi": 8, "rx": None, "ry": None},
        {"op": "quote", "amt": "-" + SKIM},
        {"op": "swap", "tok": "y", "amt": SWAP},
        {"op": "quote", "amt": "-" + SKIM2},
        {"op": "swap", "tok": "y", "amt": str(10**22)},
        {"op": "quote", "amt": SKIM},
        {"op": "swap", "tok": "x", "amt": SWAP},
    ],
    "overshoot": [
        {"op": "init", "toka": "tokx.tst", "tokb": "toky.tst", "fee_shift": "8", "s0": S0},
        {"op": "seed", "tok": "x", "amt": SEED},
        {"op": "seed", "tok": "y", "amt": SEED},
        {"op": "mint", "d": D, "lo": -4, "hi": 4, "rx": None, "ry": None},
        {"op": "swap", "tok": "x", "amt": SWAP_BIG},
        {"op": "swap", "tok": "x", "amt": SWAP_BIG},
        {"op": "swap", "tok": "y", "amt": SWAP_BIG},
    ],
    # mo guard: swap 1 aborts (mo = exact out + 1 ⇒ ERR_MO, full refund incl.
    # fee); swap 2 with mo == exact out succeeds — proving the quote-law
    # value is achievable and the abort left zero state damage.
    "min_out": [
        {"op": "init", "toka": "tokx.tst", "tokb": "toky.tst", "fee_shift": "8", "s0": S0},
        {"op": "seed", "tok": "x", "amt": SEED},
        {"op": "seed", "tok": "y", "amt": SEED},
        {"op": "mint", "d": D, "lo": -8, "hi": 8, "rx": None, "ry": None},
        {"op": "quote", "amt": SKIM},
        {"op": "swap", "tok": "x", "amt": SWAP, "mo": "Q+1"},
        {"op": "swap", "tok": "x", "amt": SWAP, "mo": "Q"},
    ],
}


def build(srcs: list, tmp: str, name: str) -> str:
    src = ""
    for f in srcs:
        with open(os.path.join(ROOT, f)) as fh:
            src += fh.read() + "\n"
    concat = os.path.join(tmp, name + ".lisp")
    wasm = os.path.join(tmp, name + ".wasm")
    with open(concat, "w") as fh:
        fh.write(src)
    r = subprocess.run([os.path.join(ROOT, "target/release/near-compile"), concat, wasm],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"compile {name} failed: " + r.stderr[-500:])
    return wasm


def mock_lines(wasm: str, ft: str, tmp: str, ops: list) -> list:
    state = os.path.join(tmp, "state.bin")
    for f in (state, state + ".manifest.json"):
        if os.path.exists(f):
            os.remove(f)
    manifest = f"pool.tst={wasm},tokx.tst={ft},toky.tst={ft}"
    env = dict(os.environ, NEAR_MOCK_SIGNER="owner")
    lines = []

    def cross(acct, method, args):
        r = subprocess.run([MOCK, "cross", state, manifest, acct, method, json.dumps(args)],
                           capture_output=True, text=True, env=env)
        m = re.search(r"📄 ([^\s(]+)", r.stdout)
        return m.group(1).strip() if m else "<trap>"

    # bootstrap: ft2 new (owner becomes owner) + fund the trader
    for tok in ("tokx.tst", "toky.tst"):
        cross(tok, "new", {})
        cross(tok, "mint", {"to": "owner", "amt": FUND})
    for o in ops:
        if o["op"] == "init":
            ret = cross("pool.tst", "init", {"toka": o["toka"], "tokb": o["tokb"],
                                             "fee_shift": o["fee_shift"], "s0": o["s0"]})
        elif o["op"] == "seed":
            tok = "tokx.tst" if o["tok"] == "x" else "toky.tst"
            ret = cross(tok, "ft_transfer_call",
                        {"receiver_id": "pool.tst", "amount": o["amt"], "msg": '{"a":"seed"}'})
        elif o["op"] == "mint":
            rx, ry = o["rx"], o["ry"]
            if rx is None:
                req = cross("pool.tst", "position_requirements",
                            {"d": o["d"], "a": str(o["lo"]), "b": str(o["hi"])})
                rx = req.split(":")[1]
                ry = req.split(":")[3]
                o["rx"], o["ry"] = rx, ry
            mm = json.dumps({"a": "mint", "d": o["d"], "lo": str(o["lo"]), "hi": str(o["hi"])})
            r1 = cross("tokx.tst", "ft_transfer_call",
                       {"receiver_id": "pool.tst", "amount": rx, "msg": mm})
            r2 = cross("toky.tst", "ft_transfer_call",
                       {"receiver_id": "pool.tst", "amount": ry, "msg": mm})
            ret = r1 + "," + r2
        elif o["op"] == "swap":
            tok = "tokx.tst" if o["tok"] == "x" else "toky.tst"
            msg = '{"a":"swap"}'
            if o.get("mo") is not None:
                if o["mo"] in ("Q", "Q+1"):
                    # resolve the exact-out floor from a probe quote on the
                    # fee-skimmed walk input (same law the quote leg pins)
                    amt = int(o["amt"])
                    v = str(amt - (amt >> 8))
                    qret = cross("pool.tst", "quote",
                                 {"v": v if o["tok"] == "x" else "-" + v})
                    if ":" not in qret:
                        sys.exit(f"mo probe: quote returned {qret!r}")
                    out = qret.split(":")[1]
                    o["mo"] = str(int(out) + 1) if o["mo"] == "Q+1" else out
                msg = '{"a":"swap","mo":"' + o["mo"] + '"}'
            ret = cross(tok, "ft_transfer_call",
                        {"receiver_id": "pool.tst", "amount": o["amt"], "msg": msg})
        elif o["op"] == "req":
            ret = cross("pool.tst", "position_requirements",
                        {"d": o["d"], "a": str(o["lo"]), "b": str(o["hi"])})
        elif o["op"] == "quote":
            ret = cross("pool.tst", "quote", {"v": o["amt"]})
        lines.append("RET:" + ret)
        st = cross("pool.tst", "get_state", {})
        st = re.sub(r":TOKX:[^:]*:TOKY:[^:]*:", ":TOKX:T:TOKY:T:", st)
        lines.append("STATE:" + st)
    return lines


def twin_lines(ops: list) -> list:
    r = subprocess.run([TSX, os.path.join(ROOT, "fixtures/clmm_v5_driver.ts")],
                       input=json.dumps({"ops": ops}).encode(),
                       capture_output=True)
    if r.returncode != 0:
        sys.exit("tsx failed: " + r.stderr.decode()[-800:])
    return [re.sub(r":TOKX:[^:]*:TOKY:[^:]*:", ":TOKX:T:TOKY:T:", ln)
            for ln in r.stdout.decode().strip().splitlines()]


def main() -> None:
    tmp = tempfile.mkdtemp(prefix="ts-parity-")
    # CLMM_POOL_WASM overrides the contract artifact — used to gate the
    # size-optimized release build (shipped == tested law).
    wasm = os.environ.get("CLMM_POOL_WASM") or build(
        ["lib/limb_math.lisp", "lib/clmm_v5_kernel.lisp", "lib/clmm_pool_v5.lisp"],
        tmp, "pool")
    print(f"pool wasm: {wasm}")
    ft = build(["lib/limb_math.lisp", "lib/clmm_core.lisp", "lib/ft2.lisp"], tmp, "ft2")
    total = 0
    fails = []
    for name, ops in SCENARIOS.items():
        ml = mock_lines(wasm, ft, tmp, ops)
        tl = twin_lines(ops)
        if len(ml) != len(tl):
            fails.append(f"{name}: line count {len(ml)} vs {len(tl)}")
            continue
        diffs = [(i, a, b) for i, (a, b) in enumerate(zip(ml, tl)) if a != b]
        if diffs:
            i, a, b = diffs[0]
            fails.append(f"{name}: first diff at line {i}\n  mock: {a[:220]}\n  twin: {b[:220]}")
            continue
        # law: quote(skim(amt)) out == executed swap's LAST out — per engine
        for eng, ls in (("mock", ml), ("twin", tl)):
            for i in range(len(ls) - 2):
                if ls[i].startswith("RET:q:") and ls[i + 2].startswith("STATE:") \
                        and "LAST:swap-out:" in ls[i + 2]:
                    qout = ls[i][6:].split(":rm:")[0]
                    sout = ls[i + 2].split("LAST:swap-out:")[1].split(":rem:")[0]
                    if qout != sout:
                        fails.append(f"{name} [{eng}]: quote/swap law broken: {qout} != {sout}")
        total += len(ml)
        print(f"{name}: {len(ml)} lines byte-exact OK")
    print()
    if fails:
        for f in fails:
            print("FAIL " + f)
        sys.exit(1)
    print(f"ALL BYTE-EXACT: {total} lines across {len(SCENARIOS)} scenarios")


if __name__ == "__main__":
    main()
