#!/usr/bin/env python3
"""Differential fuzz: fee-ledger + token (TWO contracts) vs one oracle.

Extends difffuzz.py: the withdraw leg is now the REAL NEP-141 chain —
withdraw parks, wd-cb dispatches ft_transfer at tok.test.near, wd-deliver
moves str->paid ONLY if the transfer committed. The fuzz scenarios FUND
the token escrow, so deliveries actually happen and the deliver/park
branch itself gets randomized coverage.

Oracle is written from User_Ledger.thy + the token's spec, NOT from the
lisp. Deliver rule (WdOk/WdFail identity, per the thy): a withdraw moves
credit into str, then str->paid IFF the escrow balance covers it — the
token transfer moves escrow->user atomically with the deliver; if the
escrow is short, NOTHING moves (str stays parked, no tokens move).
"""
import base64, json, random, subprocess, sys, os

MOCK = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")
D = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/scenarios/lisp/fee-ledger")
FL = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/contracts/fee-ledger/target/fee-ledger.wasm")
TOK = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/contracts/token/target/token.wasm")
FLA = "fl.clmm.test.near"
TOKA = "tok.test.near"
OWNER = "fl-owner.test.near"
USERS = ["u1.test.near", "u2.test.near", "u3.test.near"]
TWO64 = 1 << 64
SUPPLY = 10**12

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

# ── User_Ledger.thy + token spec as data ─────────────────────────────
def settle(o, who):
    G, ck = o.get("G", 0), o.get(f"ck:{who}", 0)
    SHT, sh = o.get("SHT", 0), o.get(f"sh:{who}", 0)
    if G < ck or SHT == 0:
        return 0
    return (G - ck) * sh // TWO64

def registered(o, who):
    return o.get(f"reg:{who}", 0) == 1 or o.get(f"sh:{who}", 0) != 0

def op_register(o, who, amt):
    o[f"reg:{who}"] = 1

def op_deposit(o, who, amt):
    if amt == 0 or not registered(o, who):
        return "trap"
    owed = settle(o, who)
    o[f"bal:{who}"] = o.get(f"bal:{who}", 0) + owed
    o[f"ck:{who}"] = o.get("G", 0)
    o[f"sh:{who}"] = o.get(f"sh:{who}", 0) + amt
    o["SHT"] = o.get("SHT", 0) + amt
    return "ok"

def op_claim(o, who, amt):
    if not registered(o, who):
        return "trap"
    owed = settle(o, who)
    o[f"bal:{who}"] = o.get(f"bal:{who}", 0) + owed
    o[f"ck:{who}"] = o.get("G", 0)
    return "ok"

def op_grow(o, who, amt):
    f = amt
    if f == 0:
        return "trap"
    SHT = o.get("SHT", 0)
    o["G"] = o.get("G", 0) + (0 if SHT == 0 else f * TWO64 // SHT)
    o["fees"] = o.get("fees", 0) + f
    o["grows"] = o.get("grows", 0) + 1
    return "ok"

def escrow(o):
    return o.get(f"tok:bal:{FLA}", 0)

def op_withdraw(o, who, amt):
    n = amt
    if not registered(o, who) or o.get(f"sh:{who}", 0) < n:
        return "trap"
    owed = settle(o, who)
    credit = o.get(f"bal:{who}", 0) + owed
    o[f"bal:{who}"] = 0
    o[f"ck:{who}"] = o.get("G", 0)
    o[f"sh:{who}"] = o.get(f"sh:{who}", 0) - n
    o["SHT"] = o.get("SHT", 0) - n
    # NEP-141 chain with FUNDED escrow: deliver iff escrow covers credit.
    # Deliver = atomic triple move: str->paid on ledger, escrow->user on
    # token. Short escrow = WdFail identity: nothing moves anywhere.
    if escrow(o) >= credit:
        o[f"tok:bal:{FLA}"] = escrow(o) - credit
        o[f"tok:bal:{who}"] = o.get(f"tok:bal:{who}", 0) + credit
        o[f"paid:{who}"] = o.get(f"paid:{who}", 0) + credit
        o.pop(f"str:{who}", None)
    else:
        o[f"str:{who}"] = o.get(f"str:{who}", 0) + credit
    return "ok"

def op_strandpay(o, who, amt):
    st = o.get(f"str:{who}", 0)
    if st:                             # thy StrandPay: pure ledger move
        o[f"paid:{who}"] = o.get(f"paid:{who}", 0) + st
        o.pop(f"str:{who}", None)
    return "ok"

OPS = [("register", op_register, 0, 0),
       ("deposit", op_deposit, 1, 10**14),
       ("claim", op_claim, 0, 0),
       ("grow", op_grow, 1, 10**10),
       ("withdraw", op_withdraw, 1, 10**22),
       ("strand-pay", op_strandpay, 0, 0)]

def gen_seq(rng, n):
    """Oracle-guided: evolves a scratch oracle while generating so withdraws
    land in-range (80%) and actually reach the deliver/park branch. The
    escrow fund is drawn PER ROUND (1e9..1e10) while credits land 1e9..1e11
    — the escrow STRADDLES the credit scale, so deliver, park, and
    strand-pay recovery all fire."""
    seq = []
    o = {}
    o["tok:supply"] = SUPPLY
    o[f"tok:bal:{OWNER}"] = SUPPLY
    escrow_amt = rng.randint(10**9, 10**10)
    o[f"tok:bal:{FLA}"] = escrow_amt
    o[f"tok:bal:{OWNER}"] -= escrow_amt
    who = rng.choice(USERS)
    seq.append(("register", who, 0)); op_register(o, who, 0)
    seq.append(("deposit", who, rng.randint(1, 10**14)))
    op_deposit(o, who, seq[-1][2])
    w2 = rng.choice([u for u in USERS if u != who])
    seq.append(("register", w2, 0)); op_register(o, w2, 0)
    seq.append(("deposit", w2, rng.randint(1, 10**14)))
    op_deposit(o, w2, seq[-1][2])
    def credit_of(u):
        return o.get(f"bal:{u}", 0) + settle(o, u)
    while len(seq) < n:
        name, fn, lo, hi = rng.choice(OPS)
        who = rng.choice(USERS)
        if name == "withdraw":
            cands = [u for u in USERS if o.get(f"sh:{u}", 0) > 0]
            if cands and rng.random() < 0.85:
                roll = rng.random()
                if roll < 0.55:
                    who = max(cands, key=credit_of)
                    amt = rng.randint(1, o[f"sh:{who}"])   # deliver-sized
                elif roll < 0.85:
                    who = rng.choice(cands)
                    amt = rng.randint(1, o[f"sh:{who}"])
                else:
                    # deliberate over-park: credit >> escrow, wd-deliver
                    # must refuse, credit parks, strand-pay can recover
                    who = max(cands, key=credit_of)
                    amt = rng.randint(1, o[f"sh:{who}"]) * (3 * escrow_amt)
            else:
                amt = rng.randint(1, 10**22)      # raw: traps / giant parks
        else:
            amt = rng.randint(lo, hi) if hi else 0
        seq.append((name, who, amt))
        prev_str = o.get(f"str:{who}", 0)
        {"register": op_register, "deposit": op_deposit, "claim": op_claim,
         "grow": op_grow, "withdraw": op_withdraw,
         "strand-pay": op_strandpay}[name](o, who, amt)
        # just parked? recover half the time right away — exercises the
        # strand-pay recovery path against real parked credits
        if name == "withdraw" and o.get(f"str:{who}", 0) > prev_str \
                and rng.random() < 0.5 and len(seq) < n:
            seq.append(("strand-pay", who, 0))
            op_strandpay(o, who, 0)
    return seq, escrow_amt

def mock_steps(seq, preds, escrow_amt):
    steps = [
        {"method": "init", "as": OWNER, "contract": TOKA,
         "args": {"supply": str(SUPPLY)}},
        {"method": "ft_transfer", "as": OWNER, "contract": TOKA,
         "args": {"receiver_id": FLA, "amount": str(escrow_amt)}},
    ]
    for (name, who, amt), res in zip(seq, preds):
        step = {"method": name, "as": who, "contract": FLA, "args": {}}
        if name == "deposit":
            step["attach"] = str(amt)
        elif name == "grow":
            # pay-gate: attached deposit must equal f exactly; f is already
            # capped at SUPPLY//2 at generation
            step["attach"] = str(amt)
            step["args"] = {"f": str(amt)}
        elif name == "withdraw":
            step["args"] = {"sh": str(amt)}
        if res == "trap":
            step["expect"] = "trap"
        steps.append(step)
    return {"name": "fuzz2", "manifest": f"{FLA}={FL},{TOKA}={TOK}", "steps": steps}

def dump_state(binpath):
    r = run([MOCK, "state", "dump", binpath])
    if r.returncode != 0:
        return None
    out = {}
    for row in json.loads(r.stdout):
        acct = row.get("account", "?")
        k = base64.b64decode(row["key"]).decode("utf-8", "replace").lstrip("\x00\x01")
        if "near-" in k:
            continue
        v = base64.b64decode(row["value"])
        if len(v) == 17 and v[0] == 0:
            val = int.from_bytes(v[1:], "little")
        else:
            s = v.decode("utf-8", "replace")
            val = int(s) if s.isdigit() else s
        out[f"{acct}:{k}"] = val
    return out

def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    rng = random.Random(seed)
    fails = 0
    for rnd in range(rounds):
        seq, escrow_amt = gen_seq(rng, rng.randint(4, 12))
        o = {}
        preds = []
        # seed the oracle with the two setup steps' effects (init + escrow
        # pre-fund), so its token namespace matches the mock from step 0
        o["tok:supply"] = SUPPLY
        o[f"tok:bal:{OWNER}"] = SUPPLY - escrow_amt
        o[f"tok:bal:{FLA}"] = escrow_amt
        for name, who, amt in seq:
            preds.append({"register": op_register, "deposit": op_deposit,
                          "claim": op_claim, "grow": op_grow,
                          "withdraw": op_withdraw,
                          "strand-pay": op_strandpay}[name](o, who, amt))
        sc = mock_steps(seq, preds, escrow_amt)
        spath = f"{D}/fuzz2-r{rnd}.json"
        bpath = f"{D}/state.bin"
        with open(spath, "w") as f:
            json.dump(sc, f)
        if os.path.exists(bpath):
            os.remove(bpath)
        r = run([MOCK, "scenario", f"fuzz2-r{rnd}.json"], cwd=D)
        ok = r.returncode == 0 and "0 fail" in r.stdout
        got = dump_state(bpath) if ok else None
        if got is None:
            print(f"round {rnd}: mock rc={r.returncode}\n{r.stdout[-400:]}")
            fails += 1
            continue
        # oracle -> dump namespace (ledger keys plain, token keys tok:)
        want = {}
        for k, v in o.items():
            want[f"{FLA}:{k}"] = v
        for k, v in o.items():
            if k.startswith("tok:"):
                want[f"{TOKA}:{k[4:]}"] = v
        for k in list(want):
            if k.startswith(f"{FLA}:tok:"):
                del want[k]
        # planted storage checks the ledger writes (reg:/sh:/bal:/...) stay
        # ledger-side; token bals move to the tok namespace above. Compare.
        want = {k: v for k, v in want.items() if v != 0}
        diverge = [(k, got.get(k, 0), want.get(k, 0))
                   for k in sorted(set(got) | set(want))
                   if got.get(k, 0) != want.get(k, 0)]
        if diverge:
            print(f"round {rnd} DIVERGE:")
            for k, g_, w_ in diverge[:8]:
                print(f"  {k}: mock={g_} oracle={w_}")
            print("  seq:", seq)
            fails += 1
        else:
            traps = preds.count("trap")
            print(f"round {rnd}: OK ({len(seq)} ops, {traps} traps)")
        os.remove(spath)
        if os.path.exists(bpath):
            os.remove(bpath)
    print(f"\n{rounds - fails}/{rounds} rounds clean, seed={seed}")
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
