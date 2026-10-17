#!/usr/bin/env python3
"""Wire the machine-checked accrual fixtures into the smoke scenarios.

Reads vectors-gen/accrual-vectors.json (produced by the Poly/ML runner
from the export_code of Aggregate_Accrual.thy, cross-checked by
crosscheck.py against the independent Python mirror) and appends one
assert step per (row, cap) to both twins' s.json:

    l = "1"  -> accrue_step computes (g - 0) * 1 div 2^64 = owed_one
    l = g    -> accrue_step computes (g - 0) * g div 2^64 = owed_g

Every expected value is copied from the JSON -- nothing here is
hand-computed, so a fixture drift fails the scenario, not the assert.
`undivided` (prodsum, no final div) has no twin surface: it is covered
by crosscheck.py at the vectors layer.

The ERR_ORDER trap is kept last: accruef's precondition (g >= c) is
part of the theorem boundary, so the scenario pins the guard too.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
VECTORS = os.path.join(HERE, os.pardir, os.pardir, "vectors-gen",
                       "accrual-vectors.json")
SCENARIOS = {
    "lisp": os.path.join(HERE, os.pardir, "scenarios", "lisp",
                         "accrual-smoke", "s.json"),
    "ts": os.path.join(HERE, os.pardir, "scenarios", "ts",
                       "accrual-ts", "s.json"),
}
METHOD = {"lisp": "accrue_step", "ts": "accrueStep"}


def fixture_steps(rows, flavor):
    steps = []
    for i, row in enumerate(rows, 1):
        g = str(row["g"])
        for cap, owed in (("one", row["owed_one"]), ("g", row["owed_g"])):
            acct = f"fix{i}cap{cap}.test.near"
            steps.append({
                "method": METHOD[flavor],
                "as": acct,
                "contract": "accrual.test.near",
                "attach": "0",
                "args": {"g": g, "l": "1" if cap == "one" else g},
                "contains": f"a:bal:{acct}={owed}",
            })
    return steps


def main():
    with open(VECTORS) as f:
        rows = json.load(f)
    if len(rows) != 5:
        sys.exit(f"expected 5 fixture rows, got {len(rows)}")
    for flavor, path in SCENARIOS.items():
        with open(path) as f:
            scenario = json.load(f)
        base = [s for s in scenario["steps"] if "trap_ok" not in s]
        trap = [s for s in scenario["steps"] if "trap_ok" in s]
        if len(trap) != 1:
            sys.exit(f"{path}: expected exactly one trap step")
        scenario["steps"] = base + fixture_steps(rows, flavor) + trap
        with open(path, "w") as f:
            json.dump(scenario, f, indent=1)
            f.write("\n")
        print(f"wrote {path} ({len(scenario['steps'])} steps)")


if __name__ == "__main__":
    main()
