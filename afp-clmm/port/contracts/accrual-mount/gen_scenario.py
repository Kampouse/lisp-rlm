#!/usr/bin/env python3
"""accrual-mount harness scenario generator.

The mount design (accrual-mount.md, PS section): ONE global per-share
fee accumulator in 2^64 fixed point,

    PS += f * 2^64 // SHT          (per fee wave, H1)
    CK:<lp> := PS                  (at deposit, H2)
    owed = (PS - CK:<lp>) * sh // 2^64   (at claim, H3)

which IS accruef with Lf = sh (constant cap): the checkpoint carries
the fold's c, each wave contributes (PS_i - PS_{i-1}) * sh / 2^64.
Theorem `accrue_within_charged_book` => total owed <= total fees
(floored, fail-closed). Late LP skim pays only post-entry waves.

This script computes every expected number HERE (single source) and
emits s.json -- nothing hand-typed. It also asserts the solvency
property on the computed table before emitting.
"""
import json
import os

TWO64 = 1 << 64
HERE = os.path.dirname(os.path.abspath(__file__))
PORT = os.path.abspath(os.path.join(HERE, os.pardir, os.pardir))
REPO = os.path.abspath(os.path.join(PORT, os.pardir, os.pardir))
OUT = os.path.join(PORT, "scenarios", "lisp", "accrual-mount", "s.json")
SRC = os.path.join(HERE, "src", "main.lisp")
BUILT = os.path.join(HERE, "src", "main.built.lisp")
# lib fragments the harness leans on: limb_math (u128-muldiv),
# clmm_core (li-add/li-sub/li-cmp). Included, never copied.
FRAGS = [os.path.join(REPO, "lib", "limb_math.lisp"),
         os.path.join(REPO, "lib", "clmm_core.lisp")]

# ---- scenario table (mirrors s.json steps; single source of truth) ----
lp1, lp2, lp3 = "lp1.test.near", "lp2.test.near", "lp3.test.near"

state = {"SHT": 0, "PS": 0}
ck = {lp1: 0, lp2: 0, lp3: 0}
bal = {lp1: 0, lp2: 0, lp3: 0}
sh = {lp1: 0, lp2: 0, lp3: 0}


def deposit(lp, amt):
    assert amt >= 10, "harness sybil guard"
    # harness: sh = amt (v4 mints PB-proportionally at assembly; the
    # ledger mechanics under test are identical)
    sh[lp] += amt
    state["SHT"] += amt
    ck[lp] = state["PS"]


def fee(f):
    assert state["SHT"] > 0, "no LPs: wave unattributable (protocol-side)"
    inc = f * TWO64 // state["SHT"]
    state["PS"] += inc
    return inc


def claim(lp):
    owed = (state["PS"] - ck[lp]) * sh[lp] // TWO64
    bal[lp] += owed
    ck[lp] = state["PS"]
    return owed


deposit(lp1, 6000)
deposit(lp2, 4000)
INC1 = fee(3000)
claim_lp2_1 = claim(lp2)
deposit(lp3, 10)
PS_AT_LP3_ENTRY = state["PS"]
LP3_ENTRY_CK = ck[lp3]          # snapshot before claim() advances it
INC2 = fee(1000)
INC3 = fee(7)
claim_lp3 = claim(lp3)
claim_lp1 = claim(lp1)
claim_lp2_2 = claim(lp2)

total_fees = 3000 + 1000 + 7
total_owed = sum(bal.values())
assert total_owed <= total_fees, f"solvency violated: {total_owed} > {total_fees}"
# skim property: lp3's owed excludes pre-entry waves. Counterfactual
# (no checkpoint, full PS history): (P1+inc2+inc3)*10//2^64 = 4.
# Checkpointed: only waves 2+3, floored = 1. ck must equal PS at entry.
assert LP3_ENTRY_CK == PS_AT_LP3_ENTRY, "checkpoint not pinned at entry"
skim_counterfactual = (PS_AT_LP3_ENTRY + INC2 + INC3) * 10 // TWO64
assert claim_lp3 == 1 and claim_lp3 < skim_counterfactual, \
    f"skim not excluded: owed {claim_lp3} vs counterfactual {skim_counterfactual}"
print(f"PS={state['PS']} SHT={state['SHT']}")
print(f"lp2 wave1 owed={claim_lp2_1}")
print(f"lp3 owed={claim_lp3} (checkpointed) vs {skim_counterfactual} (skim counterfactual)")
print(f"lp1 total owed={claim_lp1}")
print(f"lp2 total owed={bal[lp2]} (= {claim_lp2_1} + {claim_lp2_2})")
print(f"solvency: owed {total_owed} <= fees {total_fees}")


def step(method, args, contains, as_="factory.test.near"):
    return {"method": method, "as": as_, "contract": "acm.test.near",
            "attach": "0", "args": args, "contains": contains}


steps = [
    step("m-init", {}, "SHT:0"),
    step("m-deposit", {"lp": lp1, "amt": "6000"}, "SHT:6000"),
    step("m-deposit", {"lp": lp2, "amt": "4000"}, "SHT:10000"),
    step("m-fee", {"f": "3000"}, f"PS:{INC1}"),
    step("m-claim", {"lp": lp2}, f"BAL:{lp2}={claim_lp2_1}"),
    step("m-deposit", {"lp": lp3, "amt": "10"}, "SHT:10010"),
    step("m-fee", {"f": "1000"}, f"PS:{PS_AT_LP3_ENTRY + INC2}"),
    step("m-fee", {"f": "7"}, f"PS:{state['PS']}"),
    step("m-claim", {"lp": lp3}, f"BAL:{lp3}={claim_lp3}"),
    step("m-claim", {"lp": lp1}, f"BAL:{lp1}={claim_lp1}"),
    step("m-claim", {"lp": lp2}, f"BAL:{lp2}={bal[lp2]}"),
    step("m-view", {}, f"OWEDSUM:{total_owed}"),
]

scenario = {
    "name": "clmm-accrual-mount",
    "manifest": "acm.test.near=../../../contracts/accrual-mount/target/acm.wasm",
    "steps": steps,
}

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w") as f:
    json.dump(scenario, f, indent=1)
    f.write("\n")
print(f"wrote {OUT} ({len(steps)} steps)")

# Assemble the buildable source: lib fragments + harness (single write,
# so the scenario and the wasm always come from the same generator run).
with open(SRC) as f:
    harness = f.read()
parts = [f"; ASSEMBLED by {os.path.basename(__file__)} — edit main.lisp, not this file.\n"]
for frag in FRAGS:
    parts.append(f"; ---- {os.path.relpath(frag, REPO)} ----\n")
    with open(frag) as f:
        parts.append(f.read())
    parts.append("\n")
parts.append("; ---- harness ----\n")
parts.append(harness)
with open(BUILT, "w") as f:
    f.write("".join(parts))
print(f"wrote {BUILT}")
