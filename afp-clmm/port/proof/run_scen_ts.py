#!/usr/bin/env python3
"""Run all 16 AFP scenarios against the TS twins.

Transforms each green lisp s.json: manifest → ts/<name>-ts/target/*.wasm,
method names → TS exports (get-paid → get_paid, get-fee → get_fee,
get-split → get_split). Asserts/pins ride along untouched — they pin
storage keys + oracle values, not method names. Fresh state per run.
"""
import glob
import json
import os
import shutil
import subprocess as sp
import sys

import sys as _sys, os as _os; _sys.path.insert(0, _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
HERE = os.path.dirname(os.path.abspath(__file__))
TS = os.path.join(HERE, "ts")
import paths
OUT = paths.SCRATCH if False else paths.SCEN_TS
NM = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")

RENAME = {"get-paid": "get_paid", "get-fee": "get_fee", "get-split": "get_split"}


def transform(sc):
    man = sc["manifest"]
    parts = []
    for ent in man.split(","):
        acct, path = ent.split("=", 1)
        name = acct.split(".")[0]            # pa.clmm.test.near → pa
        import paths; wasm = paths.ts_wasm(name)
        if not os.path.exists(wasm):
            raise SystemExit(f"missing TS wasm for {acct}: {wasm}")
        parts.append(f"{acct}={wasm}")
    sc["manifest"] = ",".join(parts)
    sc["name"] = sc["name"] + "-ts"
    for st in sc["steps"]:
        if st["method"] in RENAME:
            st["method"] = RENAME[st["method"]]
    return sc


def main():
    only = sys.argv[1:] or sorted(
        os.path.basename(d) for d in glob.glob(os.path.join(paths.SCEN_LISP, "*"))
        if os.path.exists(os.path.join(d, "s.json")))
    only = [s for s in only if s != "x"]  # x/ = oct9 debug leftover
    if not sys.argv[1:]:
        print("scenarios:", " ".join(only))
    results = {}
    for s in only:
        src = os.path.join(paths.SCEN_LISP, s, "s.json")
        d = os.path.join(OUT, s)
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
        sc = transform(json.load(open(src)))
        with open(os.path.join(d, "s.json"), "w") as f:
            json.dump(sc, f, indent=1)
        r = sp.run([NM, "scenario", "s.json"], cwd=d,
                   capture_output=True, text=True, timeout=300)
        out = r.stdout + r.stderr
        fails = [ln.strip()[:170] for ln in out.splitlines() if "✗" in ln or "❌" in ln]
        ok = r.returncode == 0 and not fails
        results[s] = (ok, fails, out)
        print(f"{'GREEN ✓' if ok else 'RED   ✗'} {s} (exit={r.returncode})")
        for fl in fails:
            print("   ", fl)
    green = sum(1 for ok, _, _ in results.values() if ok)
    print(f"\nTS-SCENARIOS: {green}/{len(results)} GREEN")
    sys.exit(0 if green == len(results) else 1)


if __name__ == "__main__":
    main()
