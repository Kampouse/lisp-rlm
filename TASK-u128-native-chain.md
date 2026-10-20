# TASK: u128 Provenance — Native Chain Between Codec Edges

**Repo:** lisp-rlm · **Risk:** medium (touches typing + emit + interpreter) · **Verification:** live A/B gas
**Machine budget:** single build lane, `CARGO_BUILD_JOBS=2`, no overnight suites, verify procs dead when parking.

## Phasing (decided 2026-10-12 with JP)
- **PHASE 1 (this spawn):** op-to-op native chain only. Storage stays decimal-string. Checker provenance + wasm_emit native locals + interpreter parity + tests + battery + rebuild pool-a/b disasm evidence. Live testnet A/B done by main agent (keys stay with main).
- **PHASE 2 (later spawn, design hooks NOW):** binary storage auto-codec (16-byte LE for proven-native values at storage edges; decimal kept for human-audit ledger keys). Checker must tag provenance at storage edges so phase 2 only changes the edge codec, not the flow.


## Why (measured 2026-10-12)
- Unit costs (testnet bench, naive codec): parse 4.28 Ggas · fmt 9.81 Ggas · round-trip 13.8 Ggas · native u128 op 0.099 Ggas → **~140x**
- Live pool-a (`poola.gork-agent.testnet`): swap(0 steps)=2.04T, swap(2 cells)=2.85T, get-paid=1.30T → **~75-80% of swap gas is codec/string carrying**
- Cause: wrappers are string-typed (`add` = parse×2 + native + fmt×1; `mdiv` parse×3; `step` ≈ 24 parse + 10 fmt per call), storage decimal-string

## Goal
Values whose provenance is provably u128 stay native (i64-pair locals) between ops. Parse once at birth (ABI/storage edge), format once at sink (return/storage write). Unproven → current behavior (taint = fallback, not error).

## Design
1. **typing/checker.rs**: tag integer types `Provenance::{Native, Stringly}`.
   - Native births: `U128` ctor, string literals in u128 op positions, `arg()`-guarded json reads declared u128, arithmetic results.
   - Stringly: json reads, storage_get, extern returns, merges of mixed provenance (taint, propagate).
2. **wasm_emit**: Native values live as (i64, i64) locals; ops lower to `__h_u128_*` directly — zero parse/format between Native-Native ops. Stringly sinks (storage_set, log, return, str-cat) format once at the edge; Stringly→Native ops parse once at use.
3. **bytecode interpreter**: mirror value handling so parity holds (Value::U128 native through ops; convert only at edges).
4. **Storage binary option** (SEPARATE follow-up, not this task): 16-byte LE codec for u128 storage values — do not bundle.

## Non-goals / fences
- No parser changes, no new sugar, no clmm logic edits. Contracts compile unchanged; gas changes, semantics must not (same outputs, bit-for-bit).
- Do not touch afp-clmm accrual WIP (uncommitted files in port/contracts/accrual — leave them alone).

## Verification ladder
1. `cargo test -j2` green (174+22 suite).
2. layer_parity harness: pool-a + one clmm scenario, zero divergence (:u128 noise already whitelisted).
3. Rebuild pool-a → deploy `poolan.gork-agent.testnet` → same swap sequence (0.05N, 0.3N, get-paid) — **acceptance: swap(2 cells) ≤ 1.2 Tgas, identical outputs (`out=199999`, refunds, paid ledger).**
4. Compare disassembly: `call 35/36` (parse/fmt) sites in hot path ≈ 0 between edges (only entry/exit).

## Deliverables
- commits on branch `u128-native-chain`, one layer per commit (checker → interpreter → emit), then the A/B script `afp-clmm/port/contracts/pool-join/pool-a/../live-ab.js` output pasted in PR.
