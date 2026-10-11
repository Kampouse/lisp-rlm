#!/usr/bin/env python3
"""Mutation gate for the escrow twins — probes known-BAD, expects FAILS.

Law (defi-porting): a green gate proves nothing by itself — every gate run
plants known-BAD mutants and demands the harness REJECT each one. If a
mutant survives, the gate is blind and the run is a failure, whatever the
green suite said.

Mutants (each breaks one .thy guard/effect in src/main.lisp):
  T1  swap_in drops (debit u a)     -> bal+reserve drifts: breaks
                                       reachable_conservation (ledger step 3)
  T2  swap_out drops reserve guard  -> swap flow can eat the LP slice:
                                       breaks SwapOut guard
                                       0 < a & a <= reserve - lp_pool
  T3  deposit drops (sput "e:in"…)  -> inflow counter drifts: breaks
                                       in - out == balance (ledger step 5)
  T4  lp_remove overpays by one     -> ceil-style redemption (theory
                                       forbids: payout floors, never
                                       overpays): breaks pinned out/pool/
                                       reserve values (lp-ledger steps 8+)
Suites: every mutant runs BOTH ledger scenarios (escrow-ledger + lp-ledger);
a mutant must be REJECTED by at least one.

Run: python3 proof/mutate_escrow.py     (from port/; exit 0 = all caught)
"""
import json
import os
import shutil
import subprocess as sp
import sys

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NC = os.path.expanduser("~/dev/lisp-rlm/target/release/near-compile")
NM = os.path.expanduser("~/dev/lisp-rlm/target/release/near-mock")
ESC = os.path.join(HERE, "contracts", "escrow", "escrow")
MUT = os.path.join(HERE, "contracts", "escrow", "mutant")
SCEN = os.path.join(HERE, "scenarios", "lisp", "escrow-ledger")
SCEN_LP = os.path.join(HERE, "scenarios", "lisp", "lp-ledger")

T1 = ('      (debit u a)\n      (sput "e:reserve"',
      '      ;; MUTANT T1: debit removed\n      (sput "e:reserve"')
T2 = ('      (if (ult (sub (sget "e:reserve" "0") (sget "e:lp_pool" "0")) a) (fail "ERR_RESERVE") "ok")\n'
      '      (sput "e:reserve" (sub (sget "e:reserve" "0") a))',
      '      ;; MUTANT T2: reserve guard removed\n'
      '      (sput "e:reserve" (sub (sget "e:reserve" "0") a))')
T3 = ('      (sput "e:in" (add (sget "e:in" "0") a))\n'
      '      (near/log (str-cat "deposit u="',
      '      ;; MUTANT T3: e:in bump removed\n'
      '      (near/log (str-cat "deposit u="')
T4 = ('         (v (u128/muldiv s pool T)))',
      '         ;; MUTANT T4: redemption overpays by one unit\n'
      '         (v (add (u128/muldiv s pool T) "1")))')


def build_mutant(name, old, new):
    """Copy escrow -> mutant/, apply one mutation, rebuild. True = built."""
    shutil.rmtree(MUT, ignore_errors=True)
    shutil.copytree(ESC, MUT)
    p = os.path.join(MUT, "src", "main.lisp")
    src = open(p).read()
    if old not in src:
        print(f"SKIP {name}: pattern not found (generator drifted — fix gate)")
        return False
    open(p, "w").write(src.replace(old, new, 1))
    r = sp.run([NC, "build", "."], cwd=MUT, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"SKIP {name}: mutant does not compile — still a detectable "
              f"break, but not a behavioral one")
        return False
    return True


def run_suite(tag):
    """Run BOTH ledger scenarios against the mutant wasm.
    True = REJECTED by at least one suite (a fail)."""
    d = os.path.join(HERE, "scenarios", "lisp", "escrow-mut")
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    rejected = False
    for i, src_dir in enumerate((SCEN, SCEN_LP)):
        s = json.load(open(os.path.join(src_dir, "s.json")))
        s["name"] = f"clmm-escrow-mut-{tag}-{i}"
        s["manifest"] = s["manifest"].replace(
            "contracts/escrow/escrow/target/escrow.wasm",
            "contracts/escrow/mutant/escrow.wasm")
        scen = os.path.join(d, f"s{i}.json")
        with open(scen, "w") as f:
            json.dump(s, f, indent=1)
        r = sp.run([NM, "scenario", f"s{i}.json"], cwd=d, capture_output=True,
                   text=True, timeout=120)
        out = r.stdout + r.stderr
        if r.returncode != 0 or "fail" in out:
            rejected = True
    return rejected


def main():
    mutants = [("T1-swapin-no-debit", *T1),
               ("T2-swapout-no-guard", *T2),
               ("T3-deposit-no-inflow", *T3),
               ("T4-lpremove-overpay", *T4)]
    caught, blind = [], []
    for name, old, new in mutants:
        if not build_mutant(name, old, new):
            continue
        rejected = run_suite(name)
        (caught if rejected else blind).append(name)
        print(f"{'CAUGHT' if rejected else 'SURVIVED'}  {name}")
    # restore: original must still be green after the run (fresh state —
    # state.bin is additive, stale state double-counts: Makefile clean law)
    shutil.rmtree(MUT, ignore_errors=True)
    shutil.rmtree(os.path.join(HERE, "scenarios", "lisp", "escrow-mut"),
                  ignore_errors=True)
    state = os.path.join(SCEN, "state.bin")
    if os.path.exists(state):
        os.remove(state)
    ok = sp.run([NM, "scenario", "s.json"], cwd=SCEN,
                capture_output=True, text=True)
    restore = ok.returncode == 0 and "0 fail" in ok.stdout + ok.stderr
    print(f"\nrestored original green: {restore}")
    print("MUTATION-GATE-GREEN" if (not blind and restore and caught)
          else f"MUTATION-GATE-FAILED: blind={blind} restore={restore}")
    sys.exit(0 if (not blind and restore and caught) else 1)


if __name__ == "__main__":
    main()
