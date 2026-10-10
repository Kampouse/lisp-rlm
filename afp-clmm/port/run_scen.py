#!/usr/bin/env python3
"""Differential scenarios, leg 2: different grids + refine/slice + fees.

Contracts: pa (grid 1/2/4e9), pb (grid 1/3/4e9), pd (= pool_join(refine pa,
refine pb) on 1/2/3/4e9), splt (equalized optimal split -> legs).
Imports gen.py directly for the model (single source of truth) and reads
pins.json for every numeric pin — nothing re-typed.

Order matters: direct comparison swaps FIRST, split LAST (the splitter's
callbacks settle around subsequent receipts). Asserts use storage ground
truth (paid:/left: keys), not logs.
"""
import json, os, shutil, subprocess as sp, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen

NM = "/Users/asil/dev/lisp-rlm/target/release/near-mock"
BASE = HERE
ACCTS = ("pa", "pb", "pd", "splt", "pc", "pj")
MANIFEST = ",".join(f"{a}.clmm.test.near=../../{a}/{a}.wasm" for a in ACCTS)  # relative: run_scen invokes near-mock with cwd=scen/<name>
TR = "trader.test.near"
SPLT = "splt.clmm.test.near"

PINS = json.load(open(os.path.join(BASE, "pins.json")))
BASE_P, INSIDE, OPT, JN = PINS["base"], PINS["inside"], PINS["opt"], PINS["join"]

def step_swap(contract, y, contains, start=None):
    args = {"min_out": "0"}
    if start is not None:
        args["start"] = str(start)
    return {"method": "swap", "as": TR, "contract": f"{contract}.clmm.test.near",
            "attach": str(y), "args": args, "contains": contains}

def steps_base(tag):
    b = BASE_P[tag]; y = b["y"]
    return [
        step_swap("pd", y, f"paid:{TR}={b['comb']}"),
        step_swap("pa", y, f"paid:{TR}={gen.swap(gen.GA, gen.LA, gen.QA, gen.GA[0], y)[0]}"),
        {"method": "split", "as": TR, "contract": SPLT, "attach": str(y), "args": {}},
        {"method": "get-paid", "as": TR, "contract": SPLT, "view": True,
         "args": {}, "contains": f"paid:{TR}={b['sum']}"},
        {"method": "get-paid", "as": TR, "contract": "pb.clmm.test.near",
         "view": True, "args": {},
         "contains": f"left:{SPLT}={b['leftB']}"},
    ]

def steps_inside():
    sqp = PINS["inside_sqp"]
    return [
        step_swap("pd", gen.INSIDE_Y, f"paid:{TR}={INSIDE['pd']['out']}", start=sqp),
        step_swap("pa", gen.INSIDE_Y, f"paid:{TR}={INSIDE['pa']['out']}", start=sqp),
        step_swap("pb", gen.INSIDE_Y, f"paid:{TR}={INSIDE['pb']['out']}", start=sqp),
    ]

def steps_opt(tag):
    s, pa_paid, pb_paid = [], 0, 0
    for alt in OPT[tag]["alts"]:
        y2, y = alt["y2"], BASE_P[tag]["y"]
        pa_paid += alt["outA"]; pb_paid += alt["outB"]
        s.append(step_swap("pa", y2, f"paid:{TR}={pa_paid}"))
        s.append(step_swap("pb", y - y2, f"paid:{TR}={pb_paid}"))
    return s

def run(name, steps_list):
    d = os.path.join(BASE, "scen", name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    sc = {"name": f"clmm-{name}", "manifest": MANIFEST, "steps": steps_list}
    with open(os.path.join(d, "s.json"), "w") as f:
        json.dump(sc, f, indent=1)
    r = sp.run([NM, "scenario", "s.json"], cwd=d, capture_output=True, text=True, timeout=120)
    out = r.stdout + r.stderr
    ok = r.returncode == 0 and "panicked" not in out and "❌" not in out
    print(f"== {name}: exit={r.returncode} ok={ok}")
    return ok, out

def steps_join():
    """pool_fee_join decomposition: joined vs legs direct, union-fee export,
    then an alt split (oracle asserts legs <= joined)."""
    y = JN["Y"]
    outA_d = gen.swap(gen.GA, gen.LA, gen.QA, gen.GA[0], y)[0]
    outC_d = gen.swap(gen.GA, gen.LC, gen.QC, gen.GA[0], y)[0]
    a = JN["alt"]
    altA = gen.swap(gen.GA, gen.LA, gen.QA, gen.GA[0], a["y1"])[0]
    altC = gen.swap(gen.GA, gen.LC, gen.QC, gen.GA[0], y - a["y1"])[0]
    return [
        step_swap("pj", y, f"paid:{TR}={JN['out']}"),
        step_swap("pa", y, f"paid:{TR}={outA_d}"),
        step_swap("pc", y, f"paid:{TR}={outC_d}"),
        {"method": "get-fee", "as": TR, "contract": "pj.clmm.test.near",
         "view": True, "args": {}, "contains": str(JN["Fh"][0])},
        step_swap("pa", a["y1"], f"paid:{TR}={outA_d + altA}"),
        step_swap("pc", y - a["y1"], f"paid:{TR}={outC_d + altC}"),
    ]

def steps_inside_join():
    """Interior entry on the heterogeneous join (pj honors \"start\")."""
    sqp = PINS["inside_sqp"]
    y = gen.INSIDE_Y
    return [
        step_swap("pj", y, f"paid:{TR}={gen.swap(gen.GA, JN['LK'], JN['QBK'], sqp, y)[0]}", start=sqp),
        step_swap("pa", y, f"paid:{TR}={gen.swap(gen.GA, gen.LA, gen.QA, sqp, y)[0]}", start=sqp),
        step_swap("pc", y, f"paid:{TR}={gen.swap(gen.GA, gen.LC, gen.QC, sqp, y)[0]}", start=sqp),
    ]

results = {}
for tag in ("exact", "offgrid", "cross"):
    results[tag] = run(tag, steps_base(tag))
results["inside"] = run("inside", steps_inside())
results["join"] = run("join", steps_join())
results["inside-join"] = run("inside-join", steps_inside_join())
for tag in ("exact", "offgrid", "cross"):
    results[f"opt-{tag}"] = run(f"opt-{tag}", steps_opt(tag))

print()
allok = True
for name, (ok, out) in results.items():
    allok &= ok
    print(f"──── {name} {'GREEN' if ok else 'RED'}")
    if ok:
        for line in out.splitlines():
            if "✓ contains" in line:
                print("  " + line.strip()[:96])
    else:
        print(out[-1500:])
print("\nALL-SCENARIOS", "GREEN" if allok else "RED")
sys.exit(0 if allok else 1)
