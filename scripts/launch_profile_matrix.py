#!/usr/bin/env python3
"""Launch-v2 profile matrix (docs/launch_v2_unified_pool_spec.md).

Tests the unified-pool launch model: creator places SUP as ask-side
segments; buyer QT becomes the bid stack; scripted buy/sell walks. One
case per liquidity profile, identical walk, compared on raise, price,
spread cost, clamp behavior.

POLARITY (measured, 2026-10-21): the pool's P walks DOWN on QT-in. So the
launched token is TOKY (asks at NEGATIVE ticks, Y-only ranges), QT = TOKX;
a QT-in buy walks P down into the asks and the QT it leaves behind becomes
the bid stack. Economic price (QT per token) rises as P falls.

Boot straddle: a two-sided seed at P0 is MANDATORY — with P in a liquidity
gap every swap refunds (nothing to walk against).

Profiles: flat | front (dense bottom) | back (dense top) | step (3 bands).
Real ft_transfer_call custody swaps only — no SIMSWAP.

Run: python3 scripts/launch_profile_matrix.py
"""
import base64
import json
import os
import subprocess
import sys

REPO = os.path.expanduser("~/dev/lisp-rlm")
MOCK = os.path.join(REPO, "target/release/near-mock")
DIR = "/tmp/launchprof"
STATE = os.path.join(DIR, "state.bin")
MANIFEST = ("tokx.tst=/tmp/lf-mock/ft.wasm,toky.tst=/tmp/lf-mock/ft.wasm,"
            "pool.tst=/tmp/lf-mock/pool.wasm")
OWNER = "owner.test.near"
CREATOR = "alice.tst"
TRADER = "bob.tst"
TOKEN = "toky.tst"      # launched token (asks, negative ticks)
QT = "tokx.tst"         # quote token (bid stack)

SUP = 10 ** 24          # launched token total, placed as asks
QT_BUDGET = 10 ** 24    # trader QT budget (8 buy tranches of 1/8)
BOOT_D = 10 ** 21       # boot-straddle d at P0 (two-sided seed — without it
                        # P sits in a gap and no swap can start); both legs
                        # over-attach 1e18, excess refunds
PROBE_D = 10 ** 18      # d used to probe per-segment req
LO, HI = -42, -2        # ask range; right lip must stay BELOW P0 — a range
                        # whose lip touches P reads two-sided and a one-sided
                        # mint into it dies ERR_MISMATCH


def sh(*args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def run_steps(cid, steps, phase):
    p = os.path.join(DIR, f"{cid}.{phase}.json")
    with open(p, "w") as f:
        json.dump({"name": cid, "manifest": MANIFEST,
                   "state": STATE, "steps": steps}, f)
    r = sh(MOCK, "scenario", p)
    out = r.stdout + r.stderr
    line = [l for l in out.splitlines() if "pass /" in l]
    ok = bool(line) and " / 0 fail" in line[-1]
    return ok, out


def dump_state():
    rows = json.loads(sh(MOCK, "state", "dump", STATE).stdout)
    st = {}
    for row in rows:
        k = base64.b64decode(row["key"]).decode(errors="replace")
        v = base64.b64decode(row["value"]).decode(errors="replace")
        st[(row["account"], k)] = v
    return st


def g(st, acct, key):
    return st.get((acct, key))


def num(v):
    try:
        return int((v or "0").replace(",", ""))
    except ValueError:
        return 0


# ── profiles (integer weights only — floats leak sci-notation amounts) ───
def segments(name):
    """[(lo, hi, weight_int)] — weights in thousandths. Bands stay ≤5
    cells: req walks + mint legs blow the gas caps past ~10 cells."""
    if name == "flat":
        return [((LO + 5 * i), (LO + 5 * (i + 1)), 1000) for i in range(8)]
    if name in ("front", "back"):
        # 10 bands of 4 cells; front = dense near P0 (cheap early entry)
        r = 0.82 if name == "front" else 1.22
        return [((LO + 4 * i), (LO + 4 * (i + 1)), int(1000 * r ** i))
                for i in range(10)]
    if name == "step":
        # 50/30/20 across three zones below P0, ≤5-cell legs
        return [(-42, -37, 2500), (-37, -32, 2500),
                (-32, -27, 1000), (-27, -22, 1000), (-22, -17, 1000),
                (-17, -12, 667), (-12, -7, 667), (-7, -2, 667)]
    raise ValueError(name)


def setup_steps():
    return [
        {"method": "new", "contract": "tokx.tst", "args": "{}",
         "as": OWNER, "expect": "ok"},
        {"method": "new", "contract": "toky.tst", "args": "{}",
         "as": OWNER, "expect": "ok"},
        {"method": "init", "contract": "pool.tst",
         "args": '{"toka":"tokx.tst","tokb":"toky.tst"}',
         "as": OWNER, "expect": "init-ok"},
        {"method": "register", "contract": "pool.tst",
         "args": json.dumps({"who": CREATOR}), "as": CREATOR,
         "expect": "reg-ok"},
        {"method": "register", "contract": "pool.tst",
         "args": json.dumps({"who": TRADER}), "as": TRADER,
         "expect": "reg-ok"},
        # creator: whole launched supply + boot-straddle QT
        {"method": "mint", "contract": TOKEN,
         "args": json.dumps({"to": CREATOR, "amt": str(SUP)}),
         "as": OWNER, "expect": "ok"},
        {"method": "mint", "contract": QT,
         "args": json.dumps({"to": CREATOR, "amt": str(10 ** 19)}),
         "as": OWNER, "expect": "ok"},
        # trader QT budget
        {"method": "mint", "contract": QT,
         "args": json.dumps({"to": TRADER, "amt": str(QT_BUDGET)}),
         "as": OWNER, "expect": "ok"},
    ]


def req_probe_steps(segs, case_id):
    """(rx, ry) per segment via position_requirements on a throwaway
    state (commas stripped; req must be probed at the placement d)."""
    out = {}
    for lo, hi, _ in segs:
        s = [
            {"method": "new", "contract": "tokx.tst", "args": "{}",
             "as": OWNER, "expect": "ok"},
            {"method": "new", "contract": "toky.tst", "args": "{}",
             "as": OWNER, "expect": "ok"},
            {"method": "init", "contract": "pool.tst",
             "args": '{"toka":"tokx.tst","tokb":"toky.tst"}',
             "as": OWNER, "expect": "init-ok"},
            {"method": "position_requirements", "contract": "pool.tst",
             "args": json.dumps({"d": str(PROBE_D), "a": lo, "b": hi}),
             "as": OWNER, "view": True, "expect": "rx:"},
        ]
        p = os.path.join(DIR, f"{case_id}_req_{lo}_{hi}.json")
        st = os.path.join(DIR, f"{case_id}_req.bin")
        if os.path.exists(st):
            os.remove(st)
        with open(p, "w") as f:
            json.dump({"name": "reqprobe", "manifest": MANIFEST,
                       "state": st, "steps": s}, f)
        r = sh(MOCK, "scenario", p)
        m = [l for l in (r.stdout + r.stderr).splitlines()
             if "rx:" in l and ":ry:" in l]
        assert m, f"req probe {lo}-{hi} produced no rx line"
        seg = m[-1].split("rx:")[-1]
        rxs, rys = seg.split(":ry:")
        out[(lo, hi)] = (int(rxs.replace(",", "") or 0),
                         int(rys.replace(",", "") or 0))
    return out


def buy_step(qt_amt):
    """QT in → tokens out (the walk consumes asks below P)."""
    return {"method": "ft_transfer_call", "contract": QT,
            "as": TRADER,
            "args": json.dumps({"receiver_id": "pool.tst",
                                "amount": str(qt_amt),
                                "msg": json.dumps({"a": "swap",
                                                   "mo": "0"})}),
            "expect": "ok"}


def sell_step(tok_amt, expect="ok"):
    """Tokens in → QT out of the bid stack."""
    return {"method": "ft_transfer_call", "contract": TOKEN,
            "as": TRADER,
            "args": json.dumps({"receiver_id": "pool.tst",
                                "amount": str(tok_amt),
                                "msg": json.dumps({"a": "swap",
                                                   "mo": "0"})}),
            "expect": expect}


def run_case(name):
    fails = []
    segs = segments(name)
    wsum = sum(w for _, _, w in segs)
    cid = f"lp_{name}"

    # phase 0: setup (fresh state)
    if os.path.exists(STATE):
        os.remove(STATE)
    ok0, out0 = run_steps(cid, setup_steps(), "p0")
    if not ok0:
        return False, [f"{name}: p0 failed"] + \
            [l for l in out0.splitlines() if "✗" in l][:3]

    # per-segment req probes (token side = ry for Y-only ask ranges)
    probes = req_probe_steps(segs, cid)
    ds = []
    for (lo, hi, w) in segs:
        rq = probes[(lo, hi)][1]        # token side
        target = SUP * w // wsum
        d = int(PROBE_D * target / rq) if rq > 0 else 0
        ds.append(max(d, 1))

    # phase 1: boot straddle (two-sided, P0 inside) + ask placement
    # attach req+1% (view/enforcer req divergence law — exact attaches
    # ERR_SHORT); boot over-attaches 10x (1-2-cell reqs round hard)
    s1 = []
    for tok in (QT, TOKEN):
        s1.append({"method": "ft_transfer_call", "contract": tok,
                   "as": CREATOR,
                   "args": json.dumps({"receiver_id": "pool.tst",
                                       "amount": str(10 ** 18),
                                       "msg": json.dumps(
                                           {"a": "mint", "d": str(BOOT_D),
                                            "lo": -8, "hi": 2})}),
                   "expect": "ok"})
    for (lo, hi, w), d in zip(segs, ds):
        target = SUP * w // wsum
        s1.append({"method": "ft_transfer_call", "contract": TOKEN,
                   "as": CREATOR,
                   "args": json.dumps({"receiver_id": "pool.tst",
                                       "amount": str(target + target // 100
                                                     + 1000),
                                       "msg": json.dumps(
                                           {"a": "mint", "d": str(d),
                                            "lo": lo, "hi": hi,
                                            "one": "1"})}),
                   "expect": "ok"})
    ok1, out1 = run_steps(cid, s1, "p1")
    if not ok1:
        return False, [f"{name}: p1 (placement) failed"] + \
            [l for l in out1.splitlines() if "✗" in l or "PANIC" in l][:3]

    st1 = dump_state()
    px0 = g(st1, "pool.tst", "6|P")

    # phase 2: 8 buy tranches
    tranche = QT_BUDGET // 8
    s2 = [buy_step(tranche) for _ in range(8)]
    ok2, out2 = run_steps(cid, s2, "p2")
    if not ok2:
        return False, [f"{name}: buys failed"] + \
            [l for l in out2.splitlines() if "✗" in l][:3]
    st2 = dump_state()
    raise_qt = num(g(st2, "pool.tst", "6|ESCX"))   # QT custody = the raise
    escy_mid = num(g(st2, "pool.tst", "6|ESCY"))
    px1 = g(st2, "pool.tst", "6|P")
    trader_tok = num(g(st2, TOKEN, f"b:{TRADER}"))

    if raise_qt <= 0:
        fails.append(f"{name}: no floor after buys (ESCX={raise_qt})")
    if trader_tok <= 0:
        fails.append(f"{name}: trader got no tokens for QT")

    # phase 3: sell everything back — measures spread + floor payout
    s3 = [sell_step(trader_tok, "ok")]
    ok3, out3 = run_steps(cid, s3, "p3")
    if not ok3:
        return False, [f"{name}: final exit failed"] + \
            [l for l in out3.splitlines() if "✗" in l][:3]
    st3 = dump_state()
    qt_back = num(g(st3, QT, f"b:{TRADER}"))
    raise_end = num(g(st3, "pool.tst", "6|ESCX"))
    if qt_back > QT_BUDGET:
        fails.append(f"{name}: payout {qt_back} exceeds budget {QT_BUDGET}")

    # global conservation (mints: creator SUP token + 1e19 QT, trader budget)
    for tok, want in ((TOKEN, SUP), (QT, QT_BUDGET + 10 ** 19)):
        tot = sum(num(g(st3, tok, f"b:{who}"))
                  for who in (CREATOR, TRADER, "pool.tst"))
        if tot != want:
            fails.append(f"{name}: conservation {tok}: {tot} != {want}")

    row = dict(profile=name, legs=len(segs), p0=px0, p_after_buys=px1,
               p_end=g(st3, "pool.tst", "6|P"), raise_qt=raise_qt,
               escy_mid=escy_mid, qt_returned=qt_back,
               spread_cost=QT_BUDGET - qt_back, raise_end=raise_end,
               fails=fails)
    return len(fails) == 0, row


def main():
    os.makedirs(DIR, exist_ok=True)
    rows = []
    all_ok = True
    for name in ("flat", "front", "back", "step"):
        ok, row = run_case(name)
        all_ok &= ok
        rows.append(row)
        if isinstance(row, dict):
            print(json.dumps(row, ensure_ascii=False))
        else:
            print(name, "FAIL", row)
    with open(os.path.join(DIR, "profile_results.json"), "w") as f:
        json.dump(rows, f, indent=1, ensure_ascii=False)
    print("ALL CLEAN" if all_ok else "FAILURES PRESENT")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
