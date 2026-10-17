#!/usr/bin/env python3
"""Differential fuzz: pool-ledger wasm vs independent Python oracle.

Ops: pdep/psweep (attach), pgrow f, pwd b f. Oracle written from
Pool_Ledger.thy, NOT from the lisp — a port bug is a storage divergence.

Pool peculiarity: the thy makes guard violations SOUND NO-OPS (identity),
never traps. So every mock step must succeed AND commit exactly what the
oracle says — the fuzzer here catches wrongful COMMITS (over-grow /
over-payout moving buckets), which expect:trap fuzzing can't see.

Per round, oracle-side we also assert the two proven theorems:
  psum == pded (conservation) and burnt <= pded (pool_no_vaporize).
"""
import base64, json, random, subprocess, sys, os

MOCK = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")
D = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/scenarios/lisp/pool-ledger")
WASM = os.path.expanduser("~/dev/lisp-rlm/afp-clmm/port/contracts/pool-ledger/target/pool-ledger.wasm")
ACCT = "pl.clmm.test.near"
SIGNERS = ["u1.test.near", "u2.test.near", "u3.test.near"]

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

# ── Pool_Ledger.thy as data ──────────────────────────────────────────
def op_pdep(o, m, f=0):
    o["principal"] = o.get("principal", 0) + m        # pstep (PDep m)

def op_psweep(o, m, f=0):
    o["unalloc"] = o.get("unalloc", 0) + m            # pstep (PSweep m)

def op_pgrow(o, f, _m=0):
    ua = o["unalloc"]                                 # pstep (PGrow f)
    if f <= ua:                                       # guard: else identity
        o["unalloc"] = ua - f
        o["setaside"] = o.get("setaside", 0) + f

def op_pwd(o, b, f):
    p, sa = o["principal"], o["setaside"]             # pstep (PWd b f)
    if b <= p and f <= sa:                            # guard: else identity
        o["principal"] = p - b
        o["setaside"] = sa - f
        o["burnt"] = o.get("burnt", 0) + b + f

OPS = [("pdep", op_pdep, 1, 10**6),
       ("psweep", op_psweep, 1, 10**6),
       ("pgrow", op_pgrow, 1, 10**6),
       ("pwd", op_pwd, 1, 10**6)]

def gen_seq(rng, n):
    seq = []
    for _ in range(n):
        name, fn, lo, hi = rng.choice(OPS)
        seq.append((name, fn, rng.randint(lo, hi), rng.randint(lo, hi)))
    return seq

def mock_steps(seq):
    steps = []
    for i, (name, fn, a, b) in enumerate(seq):
        step = {"method": name, "as": SIGNERS[i % 3], "contract": ACCT,
                "args": {}}
        if name == "pdep" or name == "psweep":
            step["attach"] = str(a)
        elif name == "pgrow":
            step["args"] = {"f": str(a)}
        elif name == "pwd":
            step["args"] = {"b": str(a), "f": str(b)}
        steps.append(step)
    return {"name": "poolfuzz", "manifest": f"{ACCT}={WASM}", "steps": steps}

def dump_state(binpath):
    r = run([MOCK, "state", "dump", binpath])
    if r.returncode != 0:
        return None
    out = {}
    for row in json.loads(r.stdout):
        k = base64.b64decode(row["key"]).decode("utf-8", "replace").lstrip("\x00\x01")
        if "near-" in k:
            continue
        v = base64.b64decode(row["value"])
        if len(v) == 17 and v[0] == 0:
            out[k] = int.from_bytes(v[1:], "little")
        else:
            s = v.decode("utf-8", "replace")
            out[k] = int(s) if s.isdigit() else s
    return out

def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    rng = random.Random(seed)
    fails = 0
    for rnd in range(rounds):
        seq = gen_seq(rng, rng.randint(4, 12))
        # oracle pass + theorem checks (psum conservation, no-vaporize)
        o = {"principal": 0, "setaside": 0, "unalloc": 0, "burnt": 0}
        pded = 0
        for name, fn, a, b in seq:
            fn(o, a, b)
            pded += a if name in ("pdep", "psweep") else 0
            psum = sum(o.values())
            if psum != pded:
                print(f"round {rnd}: ORACLE psum {psum} != pded {pded}")
                fails += 1
                break
            if o["burnt"] > pded:
                print(f"round {rnd}: ORACLE burnt {o['burnt']} > pded {pded}")
                fails += 1
                break
        else:
            # mock pass (fresh state per round); NO expect:trap — every
            # step must succeed and commit exactly the oracle's move
            sc = mock_steps(seq)
            spath = f"{D}/poolfuzz-r{rnd}.json"
            bpath = f"{D}/state.bin"
            with open(spath, "w") as f:
                json.dump(sc, f)
            if os.path.exists(bpath):
                os.remove(bpath)
            r = run([MOCK, "scenario", f"poolfuzz-r{rnd}.json"], cwd=D)
            ok = r.returncode == 0 and "0 fail" in r.stdout
            got = dump_state(bpath) if ok else None
            want = {k: v for k, v in o.items() if v != 0}
            diverge = None
            if got is None:
                print(f"round {rnd}: mock rc={r.returncode}\n{r.stdout[-400:]}")
                diverge = [("MOCK", r.returncode, 0)]
            else:
                diverge = [(k, got.get(k, 0), want.get(k, 0))
                           for k in sorted(set(got) | set(want))
                           if got.get(k, 0) != want.get(k, 0)]
            if diverge:
                print(f"round {rnd} DIVERGE:")
                for k, g_, w_ in diverge[:6]:
                    print(f"  {k}: mock={g_} oracle={w_}")
                print("  seq:", [(n, a, b) for n, _, a, b in seq])
                fails += 1
            else:
                print(f"round {rnd}: OK ({len(seq)} ops)")
            os.remove(spath)
            if os.path.exists(bpath):
                os.remove(bpath)
    print(f"\n{rounds - fails}/{rounds} rounds clean, seed={seed}")
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
