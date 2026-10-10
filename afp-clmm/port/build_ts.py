#!/usr/bin/env python3
"""Build all TS twins under ts/ with near-compile; report sizes/failures."""
import os
import subprocess as sp
import sys

TS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ts")
NC = os.path.expanduser("~/dev/lisp-rlm/target/release/near-compile")

fail = []
for name in sorted(os.listdir(TS)):
    d = os.path.join(TS, name)
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
