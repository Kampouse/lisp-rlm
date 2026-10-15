#!/usr/bin/env python3
"""Differential fuzz: fee-ledger wasm vs independent Python oracle.

Random op sequences (register/deposit/claim/grow/withdraw/strand-pay),
run through near-mock, compare full contract storage after each run
against the oracle's state. The oracle is written from User_Ledger.thy,
NOT from the lisp — a port bug shows up as a storage divergence.
"""
import base64, json, random, subprocess, sys, os

MOCK = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")
D = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/scenarios/lisp/fee-ledger")
WASM = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/contracts/fee-ledger/target/fee-ledger.wasm")
TWO64 = 1 << 64
ACCT = "fl.clmm.test.near"
USERS = ["u1.test.near", "u2.test.near", "u3.test.near"]

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

class Oracle:
    """User_Ledger.thy as data. G in 2^64 fixed point, ck per user."""
    def __init__(self):
        self.s = {}   # key -> int (oracle-side view of contract storage)

    def g(self, k, d=0):
        return self.s.get(k, d)
    def put(self, k, v):
        self.s[k] = v

def settle(o, who):
    G, ck = o.g("G"), o.g(f"ck:{who}")
    SHT, sh = o.g("SHT"), o.g(f"sh:{who}")
    if G < ck or SHT == 0:
        return 0
    return (G - ck) * sh // TWO64

def registered(o, who):
    return o.g(f"reg:{who}") == 1 or o.g(f"sh:{who}") != 0

def op_register(o, who, amt):
    o.put(f"reg:{who}", 1)

def op_deposit(o, who, amt):
    if amt == 0 or not registered(o, who):
        return "trap"
    owed = settle(o, who)
    o.put(f"bal:{who}", o.g(f"bal:{who}") + owed)
    o.put(f"ck:{who}", o.g("G"))
    o.put(f"sh:{who}", o.g(f"sh:{who}") + amt)
    o.put("SHT", o.g("SHT") + amt)
    return "ok"

def op_claim(o, who, amt):
    if not registered(o, who):
        return "trap"
    owed = settle(o, who)
    o.put(f"bal:{who}", o.g(f"bal:{who}") + owed)
    o.put(f"ck:{who}", o.g("G"))
    return "ok"

def op_grow(o, who, amt):
    # amt = f; pay-gate: attached must equal f (mock attaches f)
    f = amt
    if f == 0:
        return "trap"
    SHT = o.g("SHT")
    o.put("G", o.g("G") + (0 if SHT == 0 else f * TWO64 // SHT))
    o.put("fees", o.g("fees") + f)
    o.put("grows", o.g("grows") + 1)
    return "ok"

def op_withdraw(o, who, amt):
    n = amt
    if not registered(o, who) or o.g(f"sh:{who}") < n:
        return "trap"
    owed = settle(o, who)
    credit = o.g(f"bal:{who}") + owed
    o.put(f"bal:{who}", 0)
    o.put(f"ck:{who}", o.g("G"))
    o.put(f"sh:{who}", o.g(f"sh:{who}") - n)
    o.put("SHT", o.g("SHT") - n)
    # NEP-141 chain: withdraw parks; wd-cb dispatches ft_transfer at the
    # token; wd-deliver moves str->paid ONLY if the transfer succeeded.
    # Fuzz scenarios don't fund the token escrow, so the transfer always
    # fails: str stays parked (WdFail = identity). paid: never moves.
    o.put(f"str:{who}", o.g(f"str:{who}") + credit)
    return "ok"

def op_strandpay(o, who, amt):
    st = o.g(f"str:{who}")
    o.put(f"paid:{who}", o.g(f"paid:{who}") + st)
    if st != 0:
        o.s.pop(f"str:{who}", None)
    return "ok"

OPS = [
    ("register",  op_register,  0, 0),
    ("deposit",   op_deposit,   1, 10**24),
    ("claim",     op_claim,     0, 0),
    ("grow",      op_grow,      1, 10**18),
    ("withdraw",  op_withdraw,  1, 10**22),
    ("strand-pay", op_strandpay, 0, 0),
]

def oracle_run(seq, seeds):
    """Apply seq to a fresh oracle; returns final oracle state dict."""
    o = Oracle()
    for i, who in enumerate(seeds["users"]):
        pass
    return o

def gen_seq(rng, n):
    seq = []
    for _ in range(n):
        name, fn, lo, hi = rng.choice(OPS)
        amt = rng.randint(lo, hi) if hi else 0
        who = rng.choice(USERS)
        seq.append((name, who, amt))
    return seq

def mock_steps(seq, predicted):
    """Build a scenario JSON; steps the oracle traps get expect:trap."""
    steps = []
    for (name, who, amt), res in zip(seq, predicted):
        step = {"method": name, "as": who, "contract": ACCT, "args": {}}
        if name == "deposit":
            step["attach"] = str(amt)
        elif name == "grow":
            step["attach"] = str(amt)
            step["args"] = {"f": str(amt)}
        elif name == "withdraw":
            step["args"] = {"sh": str(amt)}
        if res == "trap":
            step["expect"] = "trap"
        steps.append(step)
    return {"name": "fuzz", "manifest": f"{ACCT}={WASM}", "steps": steps}

def dump_state(binpath):
    r = run([MOCK, "state", "dump", binpath])
    if r.returncode != 0:
        return None
    out = {}
    for row in json.loads(r.stdout):
        k = base64.b64decode(row["key"]).decode("utf-8", "replace").lstrip("\x00\x01")
        if "near-" in k:
            continue  # mock-internal bookkeeping
        v = base64.b64decode(row["value"])
        if len(v) == 17 and v[0] == 0:
            out[k] = int.from_bytes(v[1:], "little")     # tagged u128 (LE)
        else:
            s = v.decode("utf-8", "replace")
            out[k] = int(s) if s.isdigit() else s        # ASCII literal/flag
    return out

def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    rng = random.Random(seed)
    fails = 0
    for rnd in range(rounds):
        seq = gen_seq(rng, rng.randint(4, 12))
        # oracle pass, collecting predictions (incl. trap flags)
        o = Oracle()
        preds = []
        for name, who, amt in seq:
            r = {"register": op_register, "deposit": op_deposit,
                 "claim": op_claim, "grow": op_grow,
                 "withdraw": op_withdraw, "strand-pay": op_strandpay}[name](o, who, amt)
            preds.append(r)
        traps = preds.count("trap")
        # mock pass (fresh state dir per round), traps pre-declared
        sc = mock_steps(seq, preds)
        spath = f"{D}/fuzz-r{rnd}.json"
        bpath = f"{D}/state.bin"  # mock writes state.bin beside the scenario JSON
        with open(spath, "w") as f:
            json.dump(sc, f)
        if os.path.exists(bpath):
            os.remove(bpath)
        r = run([MOCK, "scenario", f"fuzz-r{rnd}.json"], cwd=D)
        if r.returncode != 0 and "0 fail" not in r.stdout:
            print(f"round {rnd}: mock rc={r.returncode}\n{r.stdout[-400:]}")
            fails += 1
            continue
        got = dump_state(bpath)
        if got is None:
            print(f"round {rnd}: state dump failed")
            fails += 1
            continue
        want = {k: v for k, v in o.s.items() if v != 0}
        # wd-cb is async: pending str: keys stay parked in the mock until
        # strand-pay; oracle models that (str: accumulates, paid only on
        # strand-pay) so a direct compare is fair. Compare common keys.
        diverge = []
        for k in sorted(set(got) | set(want)):
            if got.get(k, 0) != want.get(k, 0):
                diverge.append((k, got.get(k, 0), want.get(k, 0)))
        if diverge:
            print(f"round {rnd} DIVERGE ({traps} traps oracle-side):")
            for k, g_, w_ in diverge[:6]:
                print(f"  {k}: mock={g_} oracle={w_}")
            print("  seq:", seq)
            fails += 1
        else:
            print(f"round {rnd}: OK ({len(seq)} ops, {traps} traps)")
        os.remove(spath)
    print(f"\n{rounds - fails}/{rounds} rounds clean, seed={seed}")
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
