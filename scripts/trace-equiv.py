#!/usr/bin/env python3
"""trace-equiv.py — interpreter vs wasm trace-equivalence harness.

Differential testing: runs each tests/equiv/*.lisp probe through BOTH
surfaces and machine-diffs the observable traces:

  INTERP  : lisp-run <file+(main)>            → println lines from stdout
  WASM    : near-compile → near-mock --once   → "  LOG: ..." lines

Probes must define (main). The harness appends (main) for the interpreter
run (lisp-run only evaluates top-level forms; near-compile wraps main as
the contract entry).

Categories per probe:
  MATCH          both ran, traces identical (line-by-line)
  DIVERGE        both ran, traces differ (prints unified diff)
  WASM_CERR      wasm compile error (probe uses unsupported surface) —
                 interp trace shown for context
  INTERP_ERR     interpreter errored
  BOTH_ERR       both errored — messages compared loosely (class match)

Usage:
  python3 scripts/trace-equiv.py [-v] [probe.lisp ...]
                                  (default: all tests/equiv/*.lisp)

State isolation: /tmp/near-mock-state.bin is removed before every wasm run.
"""

import subprocess
import sys
import glob
import os
import re
import argparse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EQUIV_DIR = os.path.join(ROOT, "tests", "equiv")
LISP_RUN = os.path.join(ROOT, "target", "debug", "lisp-run")
NEAR_COMPILE = os.path.join(ROOT, "target", "debug", "near-compile")
NEAR_MOCK = os.path.join(ROOT, "target", "debug", "near-mock")
STATE_FILE = "/tmp/near-mock-state.bin"

# lisp-run debug noise to strip (form echoes, => value lines, banners)
NOISE = re.compile(r"^(\[|  =>|Parsed |Evaluated |$)")


def interp_trace(path: str):
    """Run probe via lisp-run with (main) appended; return (lines, err)."""
    src = open(path).read()
    if "(main)" not in src:
        return None, "probe has no (main)"
    tmp = "/tmp/trace-equiv-interp.lisp"
    with open(tmp, "w") as f:
        f.write(src + "\n(main)\n")
    p = subprocess.run([LISP_RUN, tmp], capture_output=True, text=True, timeout=60)
    lines = [l for l in p.stdout.splitlines() if not NOISE.match(l)]
    # lisp-run echoes the final form's value as a bare REPL-style line AFTER
    # (main) — our appended (main) is last, so drop that echo (always present)
    if lines:
        lines = lines[:-1]
    err = None
    if p.returncode != 0 or "ERROR" in p.stdout:
        m = re.search(r"ERROR at form \d+: (.*)", p.stdout)
        err = m.group(1).strip() if m else (p.stderr.strip()[:200] or "nonzero exit")
    return lines, err


def wasm_trace(path: str):
    """Compile via near-compile, run via near-mock --once; return (lines, err)."""
    wasm = "/tmp/trace-equiv.wasm"
    if os.path.exists(STATE_FILE):
        os.remove(STATE_FILE)
    c = subprocess.run([NEAR_COMPILE, path, wasm], capture_output=True, text=True, timeout=60)
    if c.returncode != 0:
        m = re.search(r"Compile error: (.*)", c.stdout + c.stderr)
        return None, "compile: " + (m.group(1).strip() if m else (c.stderr.strip()[:200] or "unknown"))
    r = None
    try:
        r = subprocess.run([NEAR_MOCK, wasm, "_run", "{}", "--once"],
                           capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired:
        return None, "WASM_HANG: execution did not finish in 30s (infinite loop?)"
    lines = []
    for l in r.stdout.splitlines():
        if l.startswith("  LOG: "):
            lines.append(l[len("  LOG: "):])
    err = None
    if "Pre-serialized error" in r.stdout or "❌" in r.stdout:
        m = re.search(r"(Pre-serialized error[^\n]*|❌[^\n]*)", r.stdout)
        err = m.group(1).strip() if m else "wasm execution error"
    elif r.returncode != 0:
        err = (r.stderr.strip()[:200] or "nonzero exit")
    return lines, err


def classify(a_lines, a_err, w_lines, w_err):
    if a_err and w_err:
        if "WASM_HANG" in w_err:
            return "WASM_HANG", None
        # loose class match on error text
        same = a_err.split(":")[0].lower()[:20] == w_err.split(":")[0].lower()[:20]
        return ("BOTH_ERR_MATCH" if same else "BOTH_ERR_DIFF"), None
    if a_err:
        return "INTERP_ERR", None
    if w_err:
        return "WASM_HANG" if "WASM_HANG" in w_err else "WASM_CERR", None
    if a_lines == w_lines:
        return "MATCH", None
    import difflib
    diff = list(difflib.unified_diff(a_lines, w_lines, "interp", "wasm", lineterm=""))
    return "DIVERGE", diff


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="*", help="probe files (default: tests/equiv/*.lisp)")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    files = args.files or sorted(glob.glob(os.path.join(EQUIV_DIR, "*.lisp")))
    if not files:
        print("no probes found in", EQUIV_DIR)
        sys.exit(2)

    counts, failures = {}, []
    for f in files:
        name = os.path.basename(f)
        a_lines, a_err = interp_trace(f)
        w_lines, w_err = wasm_trace(f)
        cat, diff = classify(a_lines, a_err, w_lines, w_err)
        counts[cat] = counts.get(cat, 0) + 1
        if cat in ("DIVERGE", "BOTH_ERR_DIFF", "INTERP_ERR"):
            failures.append(name)
        mark = {"MATCH": "✅", "DIVERGE": "❌", "WASM_CERR": "⚠️ ",
                "INTERP_ERR": "💥", "BOTH_ERR_MATCH": "✅", "BOTH_ERR_DIFF": "❌",
                "WASM_HANG": "⏰"}.get(cat, "? ")
        print(f"{mark}{cat:<14} {name}")
        if args.verbose or cat in ("DIVERGE", "BOTH_ERR_DIFF", "INTERP_ERR", "WASM_HANG"):
            if a_err: print(f"    interp-err: {a_err[:160]}")
            if w_err: print(f"    wasm-err:   {w_err[:160]}")
            if diff:
                for d in diff[:24]:
                    print("    " + d)

    total = len(files)
    print(f"\n{total} probes: " + "  ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
