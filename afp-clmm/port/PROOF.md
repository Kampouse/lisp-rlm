# PROOF — what is verified, how, and current status

Every stage re-runnable via **`python3 prove.py`** (from `port/`; builds
must exist — see preflight notes there). Status date: 2026-10-10.

| stage | runner | scope | status |
|-------|--------|-------|--------|
| leg 2 — differential (lisp) | `python3 run_scen.py` | 9 scenarios: `exact`, `inside`, `cross`, `offgrid`, `opt-exact`, `opt-cross`, `opt-offgrid`, `join`, `inside-join` — byte-exact vs `pins.json` | GREEN 9/9 |
| leg 4 — N-pool (lisp) | `near-mock scenario scen/<name>/s.json` ×5 | `npool-over`, `npool-two`, `npool-star`, `npool-under`, `opt-n` — byte-exact vs `pins3.json` | GREEN 5/5 |
| leg 4 — split sweeps | runs inside `python3 gen3.py` | FULL fraction grid 33³ = 39,304 splits + 20k random splits; zero optimality violations beyond `VIOLWIN = 2*N` dust | GREEN (per gen3.py run) |
| TS twins — differential | `python3 run_scen_ts.py` | 14 scenarios, same Isabelle pins, zero tolerance (byte-exact oracle equality) | GREEN 14/14 |
| repo differential twin | `cargo test --release -p lisp-rlm-wasm --test test_clmm_ts` | in-repo hand-written `examples/clmm.ts` vs `examples/clmm.lisp` through the same backend, near-mock | GREEN 2/2 |

## What "byte-exact vs pins" means

`gen.py` / `gen3.py` compute expected values ORACLE-SIDE (in Python, with
every floor mirrored exactly as the contract does it) and write them to
`pins.json` / `pins3.json`. The runners then execute the compiled wasms via
near-mock and assert storage/output equality with zero tolerance. A pin
failure means contract and model diverged — there is no "close enough".

## Reading the scenario names

- `exact` / `inside` / `cross` / `offgrid` — swap entry points: at grid
  boundary, interior of a cell, crossing cells, off any grid
- `join`, `inside-join` — leg-3 heterogeneous-fee join
- `opt-*` — split-optimality pins (the splitter's allocation vs best legal alternative)
- `npool-*` — leg-4 N=3 composition shapes (over/two/star/under = deposit
  size vs zone structure)
