# CLMM port — the AFP entry, proved in WASM

Machine-checked Isabelle/HOL model of Uniswap-v3-style concentrated
liquidity (pool-combination theorem + split optimality), ported to
lisp-rlm contracts and differentially tested against the model's own
numbers.

```
cd port
make prove    # pool-join + fee-join: 9/9 · n-pool: 5/5 · TS twins: 14/14 — exit 0 only if ALL GREEN
```

## Layout (6 entries, that's all)

| path | what it is |
|------|------------|
| `contracts/` | the 9 contracts, grouped by leg — each dir: `src/main.lisp` + `ts/main.ts` + `README.md` + `near.json` |
| `scenarios/` | `lisp/` + `ts/` — 14 scenario manifests each, named after what they exercise (`inside` = interior entry, `opt-*` = optimality pins, `npool-*` = leg-4 shapes) |
| `proof/` | the oracle harness: `prove.py`, `run_scen.py`, `run_scen_ts.py`, `pins.json`/`pins3.json` (expected values), `PROOF.md` (status table) |
| `generators/` | `gen.py`/`gen3.py`/`gen_ts.py` — all constants live here, contracts are GENERATED; `build_ts.py` compiles the TS twins |
| `paths.py` | the one file that knows where everything is |
| `Makefile` | `prove` / `build-ts` / `regenerate` / `clean` |

## The 9 contracts

| dir | pool | leg | what it pins |
|-----|------|-----|--------------|
| `contracts/pool-join/pool-a/` | pa | 2 | baseline: grid [1,2,4]e9, fee 0.3% |
| `contracts/pool-join/pool-b/` | pb | 2 | partner on a DIFFERENT grid [1,3,4]e9 — makes refinement real work |
| `contracts/pool-join/join-ab/` | pd | 2 | `pool_join(refine a, refine b)` — swapping the joined pool ≡ swapping a+b (the theorem) |
| `contracts/pool-join/split-ab/` | splt | 2 | no (a,b) split beats the equalized one (optimality) |
| `contracts/fee-join/pool-c/` | pc | 3 | same grid as a, fee 0.5% — the fee contrast |
| `contracts/fee-join/fee-join-ac/` | pj | 3 | `pool_fee_join(a, c)` — heterogeneous fees |
| `contracts/n-pool/pool-n1/`, `pool-n2/` | n1, n2 | 4 | members of the N=3 composition (our generalization, not in the AFP entry) |
| `contracts/n-pool/split-3/` | splt3 | 4 | equal-ending-price optimum across a+n1+n2 |

Legs: **2** = pairwise combination (AFP theorem) · **3** = heterogeneous
fees · **4** = N-pool composition (generalization).

NEAR account ids are historical wire format (`pa.clmm.test.near`,
`splt.clmm.test.near`, …) — baked into scenario asserts, don't rename.
Each contract dir has its own README with grid/liquidity/fee specifics.

## Reading order (first visit)

1. `../README.md` — 6-line orientation for `afp-clmm/`
2. `../theory/` — the verbatim AFP entry: `CLMM_Description.thy` (the
   math), `NOTES.md` (model → port mapping)
3. `generators/gen.py` — every constant, once
4. any contract's `README.md` → its `src/main.lisp` (headers carry the math)
5. `proof/run_scen.py` / `proof/run_scen_ts.py` — how it's tested;
   `proof/PROOF.md` — the stage × scenario × status table

## Proving it yourself

| command | does |
|---------|------|
| `make prove` | every stage, ALL GREEN or exit 1 |
| `make regenerate` | rewrite contracts from `generators/` — result is byte-identical to what's committed |
| `make build-ts` | compile all 9 TS twins |
| `make clean` | remove build artifacts + scenario `state.bin` (see pitfall below) |

Needs `near-compile` + `near-mock` on PATH or at
`~/dev/lisp-rlm/target/release/` (build:
`cargo build --release -p near-compile -p lisp-rlm-wasm`), Python ≥ 3.9.
Testnet wasms of the same contracts: `twap-c.lisp-demo2-1788293746.testnet`,
`twap-pool-ts.lisp-demo2-1788293746.testnet`.

## Pitfalls (each one cost a red)

1. **`state.bin` compounds additive ledgers.** `near-mock scenario`
   loads `state.bin` if present and writes it at the end. Splitters
   credit `paid:<who>` additively, so an ad-hoc rerun without cleanup
   inflates the assert (×2, ×3, …). Symptom: `paid:trader=K×expected`
   while pure-function steps still pass exactly. Fix:

   ```
   rm scenarios/lisp/*/state.bin   # or just: make clean
   ```

   `prove.py` disinfects before each leg-4 run; the Rust harness
   zeroes state itself; only bare CLI reruns need the manual `rm`.
2. **TS: `u128.mulDiv` does not exist** — the 3-arg muldiv is the free
   fn `u128MulDiv(a, b, d)`. Used to fail silently (zeroed outputs);
   the frontend now hard-errors at compile with the fix spelled out.
   Namespace is closed: add/sub/mul/div/mod/lt/gt/eq/fromI64/toI64/isZero.
3. **TS: unroll interlocking loop state.** Loop-carried u128-string
   mutation across 2+ `let`s once stalled silently after iteration 1
   (fixed in the compiler, probed 2026-10-10) — the generator still
   unrolls binary search into unique-named `const` stages
   (`_bsearch_stages()`): same shape as the lisp `let*` chain, and
   immune by construction.
4. **TS dialect quickies:** u128 predicates are real bools; storage
   reads need `?? "0"`; exports use underscores (`get_paid`); the
   scenario `manifest` is one comma-separated string
   (`acct=path,acct=path`), relative to the scenario dir.
5. **Don't edit generated files.** Contracts come from `generators/` —
   edit there, run `make regenerate`; the committed tree stays
   byte-identical to generator output.
