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

HERE = os.path.dirname(os.path.abspath(__file__))
NM = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")

LEG4 = ["npool-over", "npool-two", "npool-star", "npool-under", "opt-n"]

# contracts the leg-4 scenarios load directly (pa via splitter legs)
ENSURE = ["pa", "n1", "n2", "splt3"]


def wasm_path(c):
    for p in (os.path.join(HERE, c, f"{c}.wasm"),
              os.path.join(HERE, c, "target", f"{c}.wasm")):
        if os.path.exists(p):
            return p
    return None


def ensure_built(contracts):
    """near-compile build . for anything missing (fresh-clone friendly)."""
    nc = os.path.expanduser("~/dev/lisp-rlm/target/release/near-compile")
    built = []
    for c in contracts:
        if wasm_path(c) is None:
            r = sh(f"{nc} build .", cwd=os.path.join(HERE, c))
            if r.returncode != 0:
                return [f"{c}: BUILD FAILED"] + r.stderr.strip().splitlines()[-2:]
            built.append(f"built {c}")
    return built


def sh(cmd, cwd=None):
    return sp.run(cmd, shell=True, cwd=cwd or HERE, capture_output=True, text=True)


def stage_lisp():
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
        sb = os.path.join(HERE, "scen", name, "state.bin")
        if os.path.exists(sb):
            os.unlink(sb)
        r = sh(f"{NM} scenario scen/{name}/s.json")
        # summary format: "3 pass / 0 fail" — don't substring-match "FAIL"
        # (it appears in "0 fail" on every PASSING run)
        ok = r.returncode == 0 and "/ 0 fail" in r.stdout
        ok_all &= ok
        results.append(f"{name}: {'GREEN' if ok else 'RED'}")
    return ok_all, results


def stage_ts():
    # build TS wasms if any missing
    missing = any(
        not os.path.exists(os.path.join(HERE, "ts", d, "target", f"{d.replace('-ts','')}-ts.wasm"))
        for d in os.listdir(os.path.join(HERE, "ts"))
        if os.path.isdir(os.path.join(HERE, "ts", d))
    )
    if missing:
        b = sh("python3 build_ts.py")
        if b.returncode != 0:
            return False, ["build_ts.py FAILED"] + b.stderr.strip().splitlines()[-3:]
    r = sh("python3 run_scen_ts.py")
    ok = r.returncode == 0 and "FAIL" not in r.stdout.upper()
    return ok, r.stdout.strip().splitlines()[-3:]


STAGES = {
    "lisp": ("leg 2+3 differential (lisp)", stage_lisp),
    "leg4": ("leg 4 N-pool (lisp)", stage_leg4),
    "ts": ("TS twins differential", stage_ts),
}


def main():
    wants = sys.argv[1:] or list(STAGES)
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
