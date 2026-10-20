#!/usr/bin/env python3
"""Exhaustive liquidity-provision matrix for the v5 pool (mock, deterministic).

Axes: liquidity size d x provision shape (one-side-X / one-side-Y / two-leg)
x range x price event (none / sim up/down small/huge) x exit pattern
(single full burn / half+half). Plus fault cases and real-ft-swap flagship
round trips.

Per case: fresh mock state, seed alice 1e26 of both tokens, mint(s), optional
price event, burns + collects. Then a storage-dump audit:
  - no-swap/sim cases: POS gone, OW*/ESC*/PM zeroed, pool balances == 0,
    alice back to exactly 1e26 on both tokens (charged out, claimed back).
  - flagships (real swaps): pool drains to fee residue, participants whole,
    the swapper actually received the other token.

Usage: provision_matrix.py [--slice i:N] [--groups core,extend,fault,flagship]
Results append to /tmp/matrix/results.jsonl (crash-safe; rerun resumes past
cases already recorded for the same binary stamp).
"""
import base64
import itertools
import json
import os
import subprocess
import sys

REPO = os.path.expanduser("~/dev/lisp-rlm")
MOCK = os.path.join(REPO, "target/release/near-mock")
DIR = "/tmp/matrix"
STATE = os.path.join(DIR, "state.bin")
RESULTS = os.path.join(DIR, "results.jsonl")
MANIFEST = ("tokx.tst=/tmp/lf-mock/ft.wasm,toky.tst=/tmp/lf-mock/ft.wasm,"
            "pool.tst=/tmp/lf-mock/pool.wasm")
OWNER = "owner.test.near"
ALICE = "alice.tst"
BOB = "bob.tst"
SEED = 10 ** 26
ATTACH = 10 ** 25

WASM_STAMP = open("/tmp/lf-mock/pool.wasm.stamp").read() if os.path.exists(
    "/tmp/lf-mock/pool.wasm.stamp") else "nostamp"


def sh(*args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def steps_json(case_id, steps):
    return {"name": case_id, "manifest": MANIFEST,
            "state": STATE, "steps": steps}


def run_steps(case_id, steps, phase):
    p = os.path.join(DIR, f"{case_id}.{phase}.json")
    with open(p, "w") as f:
        json.dump(steps_json(case_id, steps), f)
    r = sh(MOCK, "scenario", p)
    out = r.stdout + r.stderr
    line = [l for l in out.splitlines() if "pass /" in l]
    ok = bool(line) and " / 0 fail" in line[-1]
    return ok, out


def import_key(account, key, value):
    dump = {"account": account, "values": [
        {"key": base64.b64encode(key.encode()).decode(),
         "value": base64.b64encode(value.encode()).decode()}]}
    p = os.path.join(DIR, "import.json")
    with open(p, "w") as f:
        json.dump(dump, f)
    r = sh(MOCK, "state", "import", STATE, p)
    return r.returncode == 0


def dump_state():
    r = sh(MOCK, "state", "dump", STATE)
    rows = json.loads(r.stdout)
    st = {}
    for row in rows:
        k = base64.b64decode(row["key"]).decode(errors="replace")
        v = base64.b64decode(row["value"]).decode(errors="replace")
        st[(row["account"], k)] = v
    return st


def g(st, account, key):
    return st.get((account, key))


def zeroish(st, account, key):
    v = g(st, account, key)
    return v is None or v == "" or v == "0"


def setup_steps(mint_bob=False):
    s = [
        {"method": "new", "contract": "tokx.tst", "args": "{}", "as": OWNER,
         "expect": "ok"},
        {"method": "new", "contract": "toky.tst", "args": "{}", "as": OWNER,
         "expect": "ok"},
        {"method": "init", "contract": "pool.tst",
         "args": '{"toka":"tokx.tst","tokb":"toky.tst"}', "as": OWNER,
         "expect": "init-ok"},
        {"method": "register", "contract": "pool.tst",
         "args": json.dumps({"who": ALICE}), "as": ALICE, "expect": "reg-ok"},
        {"method": "mint", "contract": "tokx.tst",
         "args": json.dumps({"to": ALICE, "amt": str(SEED)}), "as": OWNER,
         "expect": "ok"},
        {"method": "mint", "contract": "toky.tst",
         "args": json.dumps({"to": ALICE, "amt": str(SEED)}), "as": OWNER,
         "expect": "ok"},
    ]
    if mint_bob:
        s += [
            {"method": "register", "contract": "pool.tst",
             "args": json.dumps({"who": BOB}), "as": BOB, "expect": "reg-ok"},
            {"method": "mint", "contract": "tokx.tst",
             "args": json.dumps({"to": BOB, "amt": str(SEED)}), "as": OWNER,
             "expect": "ok"},
            {"method": "mint", "contract": "toky.tst",
             "args": json.dumps({"to": BOB, "amt": str(SEED)}), "as": OWNER,
             "expect": "ok"},
        ]
    return s


def mint_step(tok, lo, hi, d, one=False, amount=None, who=ALICE):
    msg = {"a": "mint", "d": str(d), "lo": lo, "hi": hi}
    if one:
        msg["one"] = "1"
    args = {"receiver_id": "pool.tst",
            "amount": str(amount or ATTACH), "msg": json.dumps(msg)}
    return {"method": "ft_transfer_call", "contract": tok,
            "as": who, "args": json.dumps(args), "expect": "ok"}


def pos_pin(lo, hi, d):
    return {"method": "get_positions", "contract": "pool.tst",
            "args": json.dumps({"who": ALICE}), "view": True,
            "expect": f"pos:{lo}:{hi}:{d}"}


_REQ_CACHE = {}


def range_reqs(lo, hi, d):
    """rx/ry over [lo,hi] via a fresh throwaway mock state, AT THE CASE'S d —
    req scales with d and underflows to 0 for tiny d, so a d=1e24 probe
    would misjudge a d=1 mint's gate (2026-10-20 matrix finding)."""
    if (lo, hi, d) in _REQ_CACHE:
        return _REQ_CACHE[(lo, hi, d)]
    p = os.path.join(DIR, "req_state.bin")
    if os.path.exists(p):
        os.remove(p)
    s = [
        {"method": "new", "contract": "tokx.tst", "args": "{}", "as": OWNER,
         "expect": "ok"},
        {"method": "new", "contract": "toky.tst", "args": "{}", "as": OWNER,
         "expect": "ok"},
        {"method": "init", "contract": "pool.tst",
         "args": '{"toka":"tokx.tst","tokb":"toky.tst"}', "as": OWNER,
         "expect": "init-ok"},
        {"method": "position_requirements", "contract": "pool.tst",
         "args": json.dumps({"d": str(d), "a": lo, "b": hi}),
         "as": OWNER, "view": True, "expect": "rx:"},
    ]
    ok, out = None, None
    pjson = os.path.join(DIR, "req.json")
    with open(pjson, "w") as f:
        json.dump({"name": "reqprobe", "manifest": MANIFEST,
                   "state": os.path.join(DIR, "req_state.bin"),
                   "steps": s}, f)
    r = sh(MOCK, "scenario", pjson)
    m = [l for l in (r.stdout + r.stderr).splitlines() if "rx:" in l and ":ry:" in l]
    if not m:
        _REQ_CACHE[(lo, hi, d)] = None
        return None
    seg = m[-1].split("rx:")[-1]
    rx, ry = seg.split(":ry:")
    _REQ_CACHE[(lo, hi, d)] = (int(rx), int(ry))
    return _REQ_CACHE[(lo, hi, d)]


def legs(prov, lo, hi, d, amount=None):
    if prov == "x1":
        return [("tokx.tst", dict(lo=lo, hi=hi, d=d, one=True, amount=amount))]
    if prov == "y1":
        return [("toky.tst", dict(lo=lo, hi=hi, d=d, one=True, amount=amount))]
    return [("tokx.tst", dict(lo=lo, hi=hi, d=d, amount=amount)),
            ("toky.tst", dict(lo=lo, hi=hi, d=d, amount=amount))]


def burn_collect_steps(lo, hi, d, halves):
    s = []
    for part in halves:
        s.append({"method": "burn", "contract": "pool.tst",
                  "args": json.dumps({"d": str(part), "a": lo, "b": hi}),
                  "as": ALICE, "expect": "ok"})
        s.append({"method": "collect", "contract": "pool.tst",
                  "args": '{"t":"X"}', "as": ALICE, "expect": "ok"})
        s.append({"method": "collect", "contract": "pool.tst",
                  "args": '{"t":"Y"}', "as": ALICE, "expect": "ok"})
    return s


DUST = 1000  # yocto-scale rounding residue from half-burn floors


def audit_full_exit(st, case_id, fails):
    for (acct, k) in st:
        if (k.startswith("6|POS:") or k.startswith("6|PM:")) and \
                st[(acct, k)] not in ("0", ""):
            fails.append(f"{case_id}: residual {acct}/{k}={st[(acct,k)][:30]}")
    for k in ("6|OWTX:alice.tst", "6|OWTY:alice.tst", "6|OWTX", "6|OWTY"):
        v = g(st, "pool.tst", k)
        if v not in (None, "", "0"):
            fails.append(f"{case_id}: pool {k}={v[:30]}")
    escx = int(g(st, "pool.tst", "6|ESCX") or 0)
    escy = int(g(st, "pool.tst", "6|ESCY") or 0)
    if escx > DUST or escy > DUST:
        fails.append(f"{case_id}: escrow residue escx={escx} escy={escy}")
    for tok in ("tokx.tst", "toky.tst"):
        bal = int(g(st, tok, "b:pool.tst") or 0)
        if bal > DUST:
            fails.append(f"{case_id}: pool balance {tok}={bal}")
        ab = int(g(st, tok, f"b:{ALICE}") or 0)
        if SEED - ab > DUST:
            fails.append(f"{case_id}: alice {tok} short {SEED - ab}")


def build_core():
    cases = []
    ranges_x1 = [(2, 16), (3, 3), (0, 8)]
    ranges_y1 = [(-16, -2), (-8, -8), (-8, 0)]
    ranges_two = [(-8, 8), (0, 8), (-8, 0), (2, 16), (-16, -2)]
    events = [("none", None), ("up-s", 10 ** 12), ("up-h", 10 ** 24),
              ("dn-s", -10 ** 12), ("dn-h", -10 ** 24)]
    for d in (1, 10 ** 24):
        for prov, lo, hi in ([(p, a, b) for p, rs in
                              (("x1", ranges_x1), ("y1", ranges_y1),
                               ("two", ranges_two)) for a, b in rs]):
            for ev_name, ev in events:
                for exit_name in ([["full"]] if d == 1
                                  else [["full"], ["half", "half"]]):
                    cid = (f"c_{prov}_{lo}_{hi}_d{d}_{ev_name}_"
                           f"{'-'.join(exit_name)}")
                    cases.append(dict(cid=cid, prov=prov, lo=lo, hi=hi, d=d,
                                      ev=ev, ev_name=ev_name,
                                      halves=[d] if exit_name == ["full"]
                                      else [d // 2, d - d // 2]))
    return cases


def build_extend():
    cases = []
    for d in (10 ** 12, 10 ** 20):
        for prov, lo, hi in (
                [("x1", a, b) for a, b in [(2, 16), (3, 3), (0, 8)]] +
                [("y1", a, b) for a, b in [(-16, -2), (-8, -8), (-8, 0)]] +
                [("two", a, b) for a, b in
                 [(-8, 8), (0, 8), (-8, 0), (2, 16), (-16, -2)]]):
            cases.append(dict(cid=f"x_{prov}_{lo}_{hi}_d{d}", prov=prov,
                              lo=lo, hi=hi, d=d, ev=None, ev_name="none",
                              halves=[d]))
    return cases


FAULTS = ["f_x1_wrongside", "f_y1_wrongside", "f_x1_underattach",
          "f_two_dmismatch", "f_d0", "f_two_yonnerange"]


def build_fault():
    cases = []
    cases.append(dict(cid="f_x1_wrongside", prov="x1", lo=-16, hi=-2,
                      d=10 ** 24, ev=None, ev_name="none", halves=None,
                      mint_amount=ATTACH))
    cases.append(dict(cid="f_y1_wrongside", prov="y1", lo=2, hi=16,
                      d=10 ** 24, ev=None, ev_name="none", halves=None,
                      mint_amount=ATTACH))
    cases.append(dict(cid="f_x1_underattach", prov="x1", lo=2, hi=16,
                      d=10 ** 24, ev=None, ev_name="none", halves=None,
                      mint_amount=1))
    cases.append(dict(cid="f_d0", prov="x1", lo=2, hi=16, d=0, ev=None,
                      ev_name="none", halves=None, mint_amount=ATTACH))
    # two-leg with mismatched d between legs: handled specially in run_case
    cases.append(dict(cid="f_two_dmismatch", prov="two-mismatch", lo=-8,
                      hi=8, d=10 ** 24, ev=None, ev_name="none", halves=None))
    # two-leg on an all-X range: the Y leg's req is 0 — pool must not trap
    # and must not strand the unused Y attach
    cases.append(dict(cid="f_two_yonnerange", prov="two", lo=2, hi=16,
                      d=10 ** 24, ev=None, ev_name="none", halves=None,
                      expect_pos=True))
    return cases


FLAGSHIPS = ["g_two_sweepdown", "g_x1_sweepdown", "g_y1_sweepdown"]


def build_flagship():
    # Sweep directions (ground truth from lf_lifecycle): Y-in = price falls =
    # sells X high->low. A one-sided position whose side is on the far side
    # of the price is a LIMIT ORDER: no counter-liquidity exists, the swap
    # must no-fill without trapping, and the burn must return exactly the
    # charge.
    return [
        dict(cid="g_two_sweepdown", prov="two", lo=-8, hi=8, d=10 ** 23,
             real_swap=5 * 10 ** 22, halves=[10 ** 23]),
        dict(cid="g_two_sweeppast", prov="two", lo=-8, hi=8, d=10 ** 23,
             real_swap=10 ** 24, halves=[10 ** 23]),
        dict(cid="g_two_sweepup", prov="two", lo=-8, hi=8, d=10 ** 23,
             real_swap=10 ** 24, up=True, halves=[10 ** 23]),
        dict(cid="g_x1_unfillable", prov="x1", lo=2, hi=16, d=10 ** 24,
             real_swap=10 ** 24, up=True, halves=[10 ** 24],
             unfillable=True),
        dict(cid="g_y1_unfillable", prov="y1", lo=-16, hi=-2, d=10 ** 24,
             real_swap=10 ** 24, halves=[10 ** 24], unfillable=True),
    ]


def run_case(c):
    cid = c["cid"]
    fails = []
    os.makedirs(DIR, exist_ok=True)
    for f in (STATE,):
        if os.path.exists(f):
            os.remove(f)

    flagship = "real_swap" in c
    mint_bob = flagship
    s1 = setup_steps(mint_bob=mint_bob)

    if c.get("prov") == "two-mismatch":
        s1.append(mint_step("tokx.tst", c["lo"], c["hi"], c["d"]))
        s1.append(mint_step("toky.tst", c["lo"], c["hi"], 10 ** 23))
        expect_pos = False
    else:
        ma = c.get("mint_amount")
        expect_pos = False
        reqs = range_reqs(c["lo"], c["hi"], c["d"]) if not cid.startswith("f_") else None
        one_ok = True
        if c["prov"] == "x1" and reqs and reqs[1] != 0:
            one_ok = False   # other side has a nonzero req: PM-pending, not a pos
        if c["prov"] == "y1" and reqs and reqs[0] != 0:
            one_ok = False
        if not one_ok:
            # pending-mint recovery path: leg 1 is escrowed as PM; leg 2 with
            # the matching d completes it into a real position
            leg1, kw1 = legs(c["prov"], c["lo"], c["hi"], c["d"])[0]
            other = "toky.tst" if c["prov"] == "x1" else "tokx.tst"
            s1.append(mint_step(leg1, c["lo"], c["hi"], c["d"], one=True))
            s1.append(mint_step(other, c["lo"], c["hi"], c["d"]))
            expect_pos = True
        else:
            for tok, kw in legs(c["prov"], c["lo"], c["hi"], c["d"]):
                s1.append(mint_step(tok, kw["lo"], kw["hi"], kw["d"],
                                    one=kw.get("one", False),
                                    amount=kw.get("amount") or ma))
        expect_pos = expect_pos or (c.get("expect_pos", False) or
                                    (c["halves"] is not None and one_ok))
    if expect_pos:
        s1.append(pos_pin(c["lo"], c["hi"], c["d"]))

    ok1, out1 = run_steps(cid, s1, "p1")
    if not ok1:
        # fault cases may legitimately fail steps; core must not
        if not cid.startswith(("f_",)):
            return False, [f"{cid}: phase1 failed"] + [
                l for l in out1.splitlines() if "✗" in l][:3]

    s2 = []
    if c.get("ev"):
        if not import_key("pool.tst", "6|SIMSWAP", "1"):
            return False, [f"{cid}: SIMSWAP import failed"]
        s2.append({"method": "exec", "contract": "pool.tst",
                   "args": json.dumps({"op": "swap", "v": str(c["ev"])}),
                   "as": ALICE, "expect": "ok"})
    if flagship:
        swaptok = "tokx.tst" if c.get("up") else "toky.tst"
        s2.append({"method": "ft_transfer_call", "contract": swaptok,
                   "as": BOB,
                   "args": json.dumps({"receiver_id": "pool.tst",
                                       "amount": str(c["real_swap"]),
                                       "msg": json.dumps({"a": "swap",
                                                          "mo": "0"})}),
                   "expect": "ok"})
    if c.get("halves"):
        s2 += burn_collect_steps(c["lo"], c["hi"], c["d"], c["halves"])
    if s2:
        ok2, out2 = run_steps(cid, s2, "p2")
        if not ok2:
            return False, [f"{cid}: phase2 failed"] + [
                l for l in out2.splitlines() if "✗" in l][:3]

    st = dump_state()

    if c["halves"] and not flagship:
        audit_full_exit(st, cid, fails)
    elif flagship:
        v = c["real_swap"]
        # global conservation: every minted token sits with someone
        tot = 0
        for tok in ("tokx.tst", "toky.tst"):
            for who in (ALICE, BOB, "pool.tst"):
                tot += int(g(st, tok, f"b:{who}") or 0)
        if tot != 4 * SEED:
            fails.append(f"{cid}: global conservation broken: {tot} "
                         f"!= {4 * SEED}")
        for (acct, k) in st:
            if (k.startswith("6|POS:") or k.startswith("6|PM:")) and \
                    st[(acct, k)] not in ("0", ""):
                fails.append(f"{cid}: residual {k}")
        px = int(g(st, "tokx.tst", "b:pool.tst") or 0)
        py = int(g(st, "toky.tst", "b:pool.tst") or 0)
        if px + py > v // 50:
            fails.append(f"{cid}: pool residue too big {px}+{py}")
        if not zeroish(st, "pool.tst", "6|ESCX") or not zeroish(
                st, "pool.tst", "6|ESCY"):
            fails.append(f"{cid}: escrow not drained "
                         f"{g(st,'pool.tst','6|ESCX')}/{g(st,'pool.tst','6|ESCY')}")
        bx = int(g(st, "tokx.tst", f"b:{BOB}") or 0)
        by = int(g(st, "toky.tst", f"b:{BOB}") or 0)
        if c.get("unfillable"):
            # limit order on the far side: no fill, everyone whole
            if bx != SEED or by != SEED:
                fails.append(f"{cid}: unfillable case moved bob "
                             f"(bx={bx} by={by})")
            ax = int(g(st, "tokx.tst", f"b:{ALICE}") or 0)
            ay = int(g(st, "toky.tst", f"b:{ALICE}") or 0)
            if ax != SEED or ay != SEED:
                fails.append(f"{cid}: unfillable case didn't refund alice "
                             f"(ax={ax} ay={ay})")
        elif c.get("up"):
            if by <= SEED:
                fails.append(f"{cid}: bob got no Y out (by={by})")
            if bx >= SEED:
                fails.append(f"{cid}: bob X not debited (bx={bx})")
        else:
            if bx <= SEED:
                fails.append(f"{cid}: bob got no X out (bx={bx})")
            if by >= SEED:
                fails.append(f"{cid}: bob Y not debited (by={by})")
    else:
        # fault: either rejected cleanly (alice whole, pool empty) or a
        # DESIGNED pending-mint recovery state (PM recorded, funds escrowed
        # exactly — recoverable by completing leg 2 / re-minting).
        has_pm = not zeroish(st, "pool.tst", "6|PM:alice.tst") and \
            g(st, "pool.tst", "6|PM:alice.tst") != "0"
        escx = int(g(st, "pool.tst", "6|ESCX") or 0)
        escy = int(g(st, "pool.tst", "6|ESCY") or 0)
        for (acct, k) in st:
            if k.startswith("6|POS:") and not c.get("expect_pos"):
                fails.append(f"{cid}: unexpected {k}={st[(acct,k)][:30]}")
            if k.startswith("6|PM:") and not has_pm and \
                    not c.get("expect_pos") and st[(acct, k)] != "0":
                fails.append(f"{cid}: unexpected {k}={st[(acct,k)][:30]}")
        ax = int(g(st, "tokx.tst", f"b:{ALICE}") or 0)
        ay = int(g(st, "toky.tst", f"b:{ALICE}") or 0)
        dx, dy = SEED - ax, SEED - ay
        if has_pm:
            # escrow-accounted pending mint: pool holds exactly what alice
            # is down, recoverable
            if dx != escx or dy != escy:
                fails.append(f"{cid}: pending not accounted "
                             f"dx={dx} escx={escx} dy={dy} escy={escy}")
            px = int(g(st, "tokx.tst", "b:pool.tst") or 0)
            py = int(g(st, "toky.tst", "b:pool.tst") or 0)
            if px != escx or py != escy:
                fails.append(f"{cid}: pool/escrow mismatch "
                             f"px={px} escx={escx} py={py} escy={escy}")
        elif not c.get("expect_pos"):
            if dx != 0 or dy != 0:
                fails.append(f"{cid}: alice down {dx}/{dy} with no pos")
            for tok in ("tokx.tst", "toky.tst"):
                if g(st, tok, "b:pool.tst") not in (None, "0"):
                    fails.append(f"{cid}: pool holds {tok}="
                                 f"{g(st, tok, 'b:pool.tst')}")
    return (not fails), fails


def main():
    args = sys.argv[1:]
    groups = "core,extend,fault,flagship"
    if "--groups" in args:
        groups = args[args.index("--groups") + 1]
    sl = None
    if "--slice" in args:
        i, n = args[args.index("--slice") + 1].split(":")
        sl = (int(i), int(n))
    cases = []
    if "core" in groups:
        cases += build_core()
    if "extend" in groups:
        cases += build_extend()
    if "fault" in groups:
        cases += build_fault()
    if "flagship" in groups:
        cases += build_flagship()
    if sl:
        cases = cases[sl[0]::sl[1]]
    os.makedirs(DIR, exist_ok=True)
    done = set()
    if os.path.exists(RESULTS) and "--resume" in args:
        for line in open(RESULTS):
            try:
                r = json.loads(line)
                if r.get("stamp") == WASM_STAMP and r["ok"]:
                    done.add(r["cid"])
            except Exception:
                pass
    npass = nfail = 0
    for c in cases:
        if c["cid"] in done:
            npass += 1
            continue
        ok, fails = run_case(c)
        with open(RESULTS, "a") as f:
            f.write(json.dumps({"cid": c["cid"], "ok": ok, "fails": fails,
                                "stamp": WASM_STAMP}) + "\n")
        if ok:
            npass += 1
        else:
            nfail += 1
            print("FAIL", c["cid"], fails[:2], flush=True)
    print(f"matrix: {npass} pass / {nfail} fail of {len(cases)} "
          f"(stamp {WASM_STAMP[:16]})")


if __name__ == "__main__":
    main()
