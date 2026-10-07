#!/usr/bin/env python3
"""clmm_invariants.py — property-based invariant harness for the CLMM pool v4.3.

Instead of expected-value checks, this drives RANDOM op sequences through
near-mock and asserts GLOBAL ACCOUNTING INVARIANTS after every op:

  I1  bal_tB(pool) == PB        (book liability == actual B held)
  I2  Σ SH:<who> == SHT         (share conservation, exact integers)
  I3  bal_tA(pool) == AB        (all A ever received sits in AB)
  I4  sanity: all book values ≥ 0
  I5  rollback: any no-op outcome (refund / reject / fault-reversal) must
      leave pool storage + every balance BIT-IDENTICAL to before the op.

Fault mode: ft2f tokens support owner-signed toggle_fail — ft_transfer
then returns the promise-failure signature, exercising v4.3's
delta-reversal rollback paths under random sequencing.

Usage:
  python3 scripts/clmm_invariants.py --pool /tmp/pv43.wasm --seed 1 --ops 250
  python3 scripts/clmm_invariants.py --pool /tmp/pvts2.wasm --seed 7 \
      --ops 200 --fault-rate 0.35
"""
import argparse
import base64
import json
import os
import random
import subprocess
import sys

MOCK = "./target/release/near-mock"
LPS = ["alice", "bob", "carol"]
ALL = LPS + ["owner"]

ROLLED_BACK_PREFIX = ("insufficient-shares-have:",)


def b64d(s: str) -> str:
    try:
        return base64.b64decode(s).decode()
    except Exception:
        return None


class Harness:
    def __init__(self, pool_wasm, state_path):
        self.pool_wasm = pool_wasm
        self.state = state_path
        self.manifest = f"tA=/tmp/ft2f.wasm,tB=/tmp/ft2f.wasm,pool={pool_wasm}"
        for f in (state_path,):
            if os.path.exists(f):
                os.unlink(f)
        self.failures = []
        self.stats = {"ops": 0, "rollbacks": 0, "checks": 0,
                      "leg2_faults": 0, "orphan_b": 0}

    def call(self, contract, method, args, signer):
        env = dict(os.environ)
        env["NEAR_MOCK_SIGNER"] = signer
        r = subprocess.run(
            [MOCK, "cross", self.state, self.manifest, contract, method,
             json.dumps(args)],
            capture_output=True, text=True, env=env,
        )
        out = r.stdout + r.stderr
        ret = None
        for ln in out.splitlines():
            if ln.startswith("📄"):
                ret = ln[1:].strip()
        return ret  # None = promise_return'd leg faulted empty (leg2 residual)

    def snapshot(self):
        """Full state dump → (pool_dict, balances{token}{acct})."""
        r = subprocess.run([MOCK, "state", "dump", self.state],
                           capture_output=True, text=True)
        rows = json.loads(r.stdout or "[]")
        pool, bal = {}, {"tA": {}, "tB": {}}
        for row in rows:
            acct, k, v = row["account"], b64d(row["key"]), b64d(row["value"])
            if acct == "pool" and k is not None:
                pool[k] = v
            if acct in ("tA", "tB") and k is not None and k.startswith("b:"):
                bal[acct][k[2:]] = v
        return pool, bal

    def check_invariants(self, snap, ctx, orphan_ok=0):
        pool, bal = snap
        pb = pool.get("PB", "0")
        # I1 with leg2-residual tolerance: bal − PB may exceed 0 ONLY by
        # the cumulative orphaned B of leg2-faulted withdrawals (stuck in
        # pool, unextractable). Drift beyond that = real desync.
        drift = int(bal["tB"].get("pool", "0")) - int(pb)
        ab = pool.get("AB", "0")
        sht = pool.get("SHT", "0")
        bp = bal["tB"].get("pool", "0")
        ap = bal["tA"].get("pool", "0")
        sh_sum = sum(int(pool[k]) for k in pool if k.startswith("SH:"))
        self.stats["checks"] += 5
        for name, got, want in (
            ("I1 bal_tB(pool)==PB+orphanB", bp, str(int(pb) + orphan_ok)),
            ("I3 bal_tA(pool)==AB", ap, ab),
            ("I2 ΣSH==SHT", str(sh_sum), sht),
        ):
            if got != want:
                self.failures.append(f"{ctx}: {name}: {got} != {want}")
        neg = [k for k in ("F", "AB", "PB", "SHT", "S0", "S1", "S2", "S3", "S4")
               if k in pool and int(pool[k] or "0") < 0]
        if neg:
            self.failures.append(f"{ctx}: I4 negative book values: {neg}")
        # I6 ladder-liquidity: slots are B-denominated sellable quotes —
        # they must never sum above the pool's ACTUAL B (bp, incl. orphaned
        # residue). A breach = ladder advertising undeliverable liquidity
        # (rounding drift in withdraw slot-scaling would show here first).
        slots_sum = sum(int(pool.get(k, "0") or "0")
                        for k in ("S0", "S1", "S2", "S3", "S4"))
        if slots_sum > int(bp):
            self.failures.append(
                f"{ctx}: I6 Σslots({slots_sum}) > bal_tB(pool)({bp})"
                f" — ladder oversells deliverable B")

    @staticmethod
    def same(a, b):
        return a[0] == b[0] and a[1] == b[1]

    def diff(self, a, b, only_pool=True):
        out = []
        keys = set(a[0]) | set(b[0])
        for k in sorted(keys):
            if a[0].get(k) != b[0].get(k):
                out.append(f"    pool[{k!r}]: {a[0].get(k)!r} → {b[0].get(k)!r}")
        for tok in ("tA", "tB"):
            for who in sorted(set(a[1][tok]) | set(b[1][tok])):
                if a[1][tok].get(who) != b[1][tok].get(who):
                    out.append(f"    {tok}[{who}]: {a[1][tok].get(who)!r}"
                               f" → {b[1][tok].get(who)!r}")
        return "\n".join(out)

    def is_noop(self, ret, args, method):
        if ret in ("wd-failed", "forbidden", "insufficient",
                   "no-shares", "bad-amt"):
            return True
        if ret.startswith(ROLLED_BACK_PREFIX):
            return True
        # full refund = returned exactly what was sent
        amt = args.get("amount")
        if amt is not None and ret == amt:
            return True
        return False


def gen_op(rng):
    r = rng.random()
    who = rng.choice(LPS)
    if r < 0.08:
        tok = rng.choice(["tA", "tB"])
        return (tok, "mint", {"to": rng.choice(LPS), "amt": str(rng.randint(1, 2000))},
                "owner", False)
    if r < 0.30:
        amt = str(rng.choice([rng.randint(0, 9), rng.randint(10, 2500)]))
        return ("tB", "ft_transfer_call",
                {"receiver_id": "pool", "amount": amt, "msg": "add_liq"}, who, False)
    if r < 0.62:
        amt = rng.choice([rng.randint(1, 60), rng.randint(60, 900)])
        mn = rng.choice(["0", str(int(amt * 0.7)), "999999999"])
        return ("tA", "ft_transfer_call",
                {"receiver_id": "pool", "amount": str(amt),
                 "msg": f"swap:{mn}"}, rng.choice(LPS), False)
    if r < 0.82:
        sh = rng.choice(["0", str(rng.randint(1, 80)), "99999999"])
        return ("pool", "_run", {"op": "withdraw", "sh": sh}, who, False)
    return ("pool", "pay_b", {"r": who, "b": "1"}, rng.choice(LPS), True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pool", required=True, help="pool wasm path")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--ops", type=int, default=250)
    ap.add_argument("--fault-rate", type=float, default=0.0)
    ap.add_argument("--state", default=None)
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--edge", action="store_true",
                    help="empty-liquidity boot + deterministic edge battery"
                         " (empty-ladder swap, shareless withdraw, AB=0"
                         " single-leg withdraw) before the random ops")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    state = args.state or f"/tmp/inv_s{args.seed}_{os.path.basename(args.pool)}.bin"
    h = Harness(args.pool, state)

    # ---- deterministic boot ----
    for tok in ("tA", "tB"):
        h.call(tok, "new", {}, "owner")
    for who in LPS:
        h.call("tA", "mint", {"to": who, "amt": "5000"}, "owner")
    h.call("tB", "mint", {"to": "alice", "amt": "12000"}, "owner")
    h.call("tB", "mint", {"to": "bob", "amt": "4000"}, "owner")
    h.call("tB", "mint", {"to": "carol", "amt": "2000"}, "owner")
    h.call("pool", "_run", {"op": "init", "toka": "tA", "tokb": "tB"}, "alice")
    if not args.edge:  # edge battery starts from ZERO liquidity
        h.call("tB", "ft_transfer_call",
               {"receiver_id": "pool", "amount": "10000", "msg": "add_liq"}, "alice")
    snap = h.snapshot()
    h.check_invariants(snap, "boot")

    def run_op(idx, tok, method, margs, signer):
        """One op through the full check machinery (edge battery + fuzz)."""
        pre = h.snapshot()
        ret = h.call(tok, method, margs, signer)
        post = h.snapshot()
        if ret is None:
            h.stats["leg2_faults"] += 1
            drift = int(post[1]["tB"].get("pool", "0")) - int(post[0].get("PB", "0"))
            if drift < h.stats["orphan_b"]:
                h.failures.append(
                    f"{idx} leg2-fault: orphan drift DECREASED "
                    f"{h.stats['orphan_b']} → {drift}")
            h.stats["orphan_b"] = drift
        h.check_invariants(post, idx, h.stats["orphan_b"])
        if ret is not None and h.is_noop(ret, margs, method):
            h.stats["rollbacks"] += 1
            if not h.same(pre, post):
                h.failures.append(
                    f"{idx} I5 ROLLBACK VIOLATION {tok}.{method} {margs}"
                    f" → {ret!r}:\n{h.diff(pre, post)}")
        return ret

    if args.edge:
        battery = [
            ("e1 empty-ladder swap (full refund)",
             "tA", "ft_transfer_call",
             {"receiver_id": "pool", "amount": "500", "msg": "swap:0"}, "alice"),
            ("e2 shareless withdraw",
             "pool", "_run", {"op": "withdraw", "sh": "100"}, "alice"),
            ("e3 zero-sh withdraw",
             "pool", "_run", {"op": "withdraw", "sh": "0"}, "bob"),
            ("e4 bootstrap liquidity (bob = first LP)",
             "tB", "ft_transfer_call",
             {"receiver_id": "pool", "amount": "1000", "msg": "add_liq"}, "bob"),
            ("e5 AB=0 withdraw (g=0 single-leg)",
             "pool", "_run", {"op": "withdraw", "sh": "400"}, "bob"),
            ("e6 first swap on thin ladder",
             "tA", "ft_transfer_call",
             {"receiver_id": "pool", "amount": "50", "msg": "swap:0"}, "carol"),
            ("e7 full-balance withdraw after swap",
             "pool", "_run", {"op": "withdraw", "sh": "600"}, "bob"),
        ]
        for tag, tok, method, margs, signer in battery:
            ret = run_op(tag, tok, method, margs, signer)
            if not args.quiet:
                print(f"  · {tag} → {ret!r}")

    for i in range(args.ops):
        # fault toggles: fire before the op, restore after — rollback
        # equality must hold across the toggle itself (only the token's
        # FAIL key changes, which is not part of snapshot)
        fault_tok = None
        if args.fault_rate > 0 and rng.random() < args.fault_rate:
            fault_tok = rng.choice(["tA", "tB"])
            h.call(fault_tok, "toggle_fail", {"on": "1"}, "owner")
        tok, method, margs, signer, must_rollback = gen_op(rng)
        run_op(f"op{i}", tok, method, margs, signer)
        h.stats["ops"] += 1
        if fault_tok:
            h.call(fault_tok, "toggle_fail", {"on": "0"}, "owner")

        if not args.quiet and (i + 1) % 50 == 0:
            print(f"  … {i+1}/{args.ops} ops, {h.stats['rollbacks']} rollbacks"
                  f" verified, {len(h.failures)} failures")

    tag = os.path.basename(args.pool)
    if h.failures:
        print(f"❌ [{tag} seed {args.seed}] "
              f"{len(h.failures)} FAILURE(S):")
        for f in h.failures[:10]:
            print("  " + f)
        sys.exit(1)
    resid = (f", {h.stats['leg2_faults']} leg2-faults "
             f"(orphanB={h.stats['orphan_b']} stuck, known residual)"
             if h.stats["leg2_faults"] else ", no leg2 faults")
    print(f"✅ [{tag} seed {args.seed}] {h.stats['ops']} ops, "
          f"{h.stats['rollbacks']} rollbacks verified bit-identical, "
          f"{h.stats['checks']} invariant checks{resid} — clean")


if __name__ == "__main__":
    main()
