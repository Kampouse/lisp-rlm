#!/usr/bin/env python3
"""Build all TS twins under ts/ with near-compile; report sizes/failures."""
import os
import subprocess as sp
import sys

import sys as _sys, os as _os; _sys.path.insert(0, _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
import glob
import paths

# Fresh-binary law: prefer the repo-source build — a stale PATH binary
# (e.g. ~/.cargo/bin predating a host-fn addition) rejects valid contracts.
NC_REPO = os.path.expanduser("~/dev/lisp-rlm/target/release/near-compile")
NC_CARGO = os.path.expanduser("~/.cargo/bin/near-compile")
NC = NC_REPO if os.path.exists(NC_REPO) else NC_CARGO

fail = []
for d in sorted(glob.glob(os.path.join(paths.HERE, "contracts", "*", "*", "ts"))):
    name = os.path.basename(os.path.dirname(d))  # contract dir name
    if not os.path.isdir(d):
        continue
    r = sp.run([NC, "build", "."], cwd=d, capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip().splitlines()
    size = next((ln for ln in out if "bytes" in ln and "validated" in ln), "?")
    ok = r.returncode == 0
    if not ok:
        fail.append(name)
    print(f"{'OK ' if ok else 'ERR'} {name:9} {size[-60:] if ok else out[-1][:120]}")
print("\nALL-TS-BUILT" if not fail else f"\nFAILED: {fail}")
sys.exit(1 if fail else 0)
