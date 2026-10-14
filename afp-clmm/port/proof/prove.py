#!/usr/bin/env python3
"""prove.py — one-shot verification of the whole port.

Runs every stage of PROOF.md and prints the table. From port/:

    python3 prove.py            # all stages
    python3 prove.py lisp ts    # subset

Preflight:
  * lisp stage: `near-compile` + `near-mock` from ~/dev/lisp-rlm/target/release
    (build them: cd ~/dev/lisp-rlm && cargo build --release -p near-compile -p lisp-rlm-wasm)
  * run_scen.py (leg 2) builds the lisp contracts itself if needed
  * leg-4 scenarios use prebuilt <contract>/target/<contract>.wasm — if missing,
    build: cd <contract> && ~/dev/lisp-rlm/target/release/near-compile build .
  * ts stage:  `python3 build_ts.py` first (or let prove.py build if wasms absent)
"""
import json
import os
import subprocess as sp
import sys

import sys as _sys, os as _os; _sys.path.insert(0, _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
HERE = os.path.dirname(os.path.abspath(__file__))
NM = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")

LEG4 = ["npool-over", "npool-two", "npool-star", "npool-under", "opt-n"]

# contracts the leg-4 scenarios load directly (pa via splitter legs)
ENSURE = ["pa", "n1", "n2", "splt3"]


import paths

def wasm_path(c):
    p = paths.lisp_wasm(c)
    return p if os.path.exists(p) else None


def ensure_built(contracts):
    """near-compile build . for anything missing (fresh-clone friendly)."""
    nc = os.path.expanduser("~/dev/lisp-rlm/target/release/near-compile")
    built = []
    for c in contracts:
        if wasm_path(c) is None:
            r = sh(f"{nc} build .", cwd=paths.cdir(c))
            if r.returncode != 0:
                return [f"{c}: BUILD FAILED"] + r.stderr.strip().splitlines()[-2:]
            built.append(f"built {c}")
    return built


def sh(cmd, cwd=None):
    return sp.run(cmd, shell=True, cwd=cwd or HERE, capture_output=True, text=True)


def stage_lisp():
    # fresh-clone friendly: build every lisp contract the manifest needs
    results = ensure_built(paths.ACCTS)
    if any("FAILED" in r for r in results):
        return False, results
    r = sh("python3 run_scen.py")
    ok = r.returncode == 0 and "RED" not in r.stdout and "GREEN" in r.stdout
    return ok, r.stdout.strip().splitlines()[-3:] + r.stderr.strip().splitlines()[-2:]


def stage_leg4():
    results = ensure_built(ENSURE)
    if any("FAILED" in r for r in results):
        return False, results
    results.append("")
    ok_all = True
    for name in LEG4:
        # additive-ledger disinfection: near-mock LOADS state.bin if present
        # (see README pitfall) — always start leg-4 runs from zero state
        sb = os.path.join(paths.scen_lisp(name), "state.bin")
        if os.path.exists(sb):
            os.unlink(sb)
        r = sh(f"{NM} scenario s.json", cwd=paths.scen_lisp(name))
        # summary format: "3 pass / 0 fail" — don't substring-match "FAIL"
        # (it appears in "0 fail" on every PASSING run)
        ok = r.returncode == 0 and "/ 0 fail" in r.stdout
        ok_all &= ok
        results.append(f"{name}: {'GREEN' if ok else 'RED'}")
    return ok_all, results


def stage_ts():
    # build TS wasms if any missing
    missing = any(not os.path.exists(paths.ts_wasm(a)) for a in paths.ACCTS)
    if missing:
        b = sh(f"python3 {os.path.join(paths.GENERATORS, 'build_ts.py')}")
        if b.returncode != 0:
            return False, ["build_ts.py FAILED"] + b.stderr.strip().splitlines()[-3:]
    r = sh("python3 run_scen_ts.py")
    ok = r.returncode == 0 and "FAIL" not in r.stdout.upper()
    return ok, r.stdout.strip().splitlines()[-3:]


STAGES = {
    "pools": ("pool-join + fee-join differential", stage_lisp),
    "npool": ("n-pool composition", stage_leg4),
    "ts": ("TS twins differential", stage_ts),
}


def main():
    wants = [w if w not in ("lisp","leg4") else {"lisp":"pools","leg4":"npool"}[w] for w in (sys.argv[1:] or list(STAGES))]
    unknown = [w for w in wants if w not in STAGES]
    if unknown:
        print(f"unknown stages: {unknown}; choose from {list(STAGES)}")
        return 2
    any_fail = False
    print(f"{'stage':<34} status   detail")
    for w in wants:
        label, fn = STAGES[w]
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001 — report, don't crash the table
            ok, detail = False, [f"{type(e).__name__}: {e}"]
        any_fail |= not ok
        print(f"{label:<34} {'GREEN' if ok else 'RED  '}   {detail[0] if detail else ''}")
        for line in detail[1:4]:
            print(f"{'':<34}          {line}")
    print("\nALL GREEN" if not any_fail else "\nFAILURES PRESENT")
    return 0 if not any_fail else 1


if __name__ == "__main__":
    sys.exit(main())
