# TASK: Timelock Vault v1 — first lisp-side consumer of block_timestamp

**Priority:** runs AFTER TASK-parser-sweep.md completes (one heavy lane at a time).
**Deliverables:** `lib/timelock_vault_v1.lisp` + `tests/test_timelock_vault.rs`

## Goal
Smallest real custody contract: deposit an ft2 token, locked until an absolute NEAR timestamp; withdraw only after unlock. This is also the showcase contract for lisp-rlm: two ops, one guard, real money semantics.

## Phase 0 — MANDATORY ABI pin (do this FIRST, before any vault logic)
`(near/block_timestamp)` is wired (wasm_emit host fn 9, i64 ns; lambda.rs `near/block_timestamp` → need_host(9)) but has ZERO lisp-side callers today. History: this builtin family once had a silent millis-vs-nanos 1e6× divergence in the mock (fixed, see hosts.rs comment). Do not trust it untested.

Build a minimal contract that returns the timestamp through the same number→string path the vault will use. Pin `NEAR_MOCK_BLOCK_TS=1800000000000000000` (pattern: tests/test_ts_airdrop.rs:41) and assert the contract observes EXACTLY that ns value — string-equal, no tolerance. If it doesn't roundtrip, STOP and report: that's an ABI bug in the i64→lisp-numerics coercion, and everything downstream is meaningless until it's fixed.

## Vault spec
Conventions mirror `lib/clmm_pool_v5.lisp` (read it first — esp. p5-on-transfer, the 6|PM record packing, and the pay/claim reversal arms).

**State (all via near/store-bytes / near/load-bytes):**
- `VL|<tok>|<owner>` = `<amt>|<unlock_ns>` — open lock record. Split at FIRST `|` (amounts never contain `|`; same packing rule as acc|rm, lib line ~75).
- `ESC|<tok>` = escrow float mirror (C2b discipline: escrow is credited by ft_on_transfer, drained only by payout successes).

**Entry — ft_on_transfer** (msg JSON):
- `{"op":"lk","u":"<absolute unlock ns>"}` — reject `u ≤ block_timestamp()` (past unlock = instant vault, pointless and ambiguous). Record predecessor as owner. Escrow the FULL amount. Reject zero.
- Unknown msg / malformed → REJECT + zero state change (identity discipline, same as WdFail in v5).

**Ops (near/input JSON, `op` dispatch like v5 line ~1071):**
- `wd` — caller must match record owner (predecessor guard, v4 self-call? pattern). Guard: `block_timestamp() ≥ unlock_ns` (boundary INCLUSIVE). Payout = single-leg ft_transfer promise to owner (the claim pattern from C2b: owed ledger + escrow restored if the receipt fails — single-leg reversal; two legs are never atomic). Record + escrow drained ONLY in the pay success branch. A second wd on a cleared record → REJECT.
- `view` — dump open records + escrow floats (test helper).

**Non-goals v1:** no beneficiary ≠ depositor, no partial withdrawal, no extend, no cancel. Keep it small.

## Pin test (tests/test_timelock_vault.rs)
Model on tests/test_pool_v5_c2b.rs (harness: m.cross / m.ret / escrow / bal helpers, real ft2 tokens).

Time travel: the mock reads `NEAR_MOCK_BLOCK_TS` per call but a spawned process's env is fixed. Pattern: run the mock in serve mode with a `--state` file; phase 1 at t0 (lock), kill + restart with later `NEAR_MOCK_BLOCK_TS` pointing at the SAME state file, phase 2 (withdraw). If the harness can't phase cleanly, adding a tiny test-only mock advance op is acceptable (~20 lines) — but prefer the restart pattern first; less mock surface.

Required paths (all with custody identities asserted):
1. lock → view shows record + escrow
2. early wd → REJECT, state unchanged (record + escrow intact)
3. advance past unlock → wd → payout lands, record cleared, escrow drained, `used` correct
4. payout receipt FAILS → lock + escrow intact (retry succeeds after)
5. double wd → second REJECTs (no double pay)
6. `u` in the past → lock REJECTed
7. boundary: ts == unlock exactly → wd allowed
8. Phase 0 ABI pin (can live in the same test file)

Custody identities: `ESC|<tok>` == Σ open VL records, at every step.

## Acceptance
- Pin test green (all 8 paths)
- test_pool_v5_custody + test_pool_v5_c2b still green (re-run both)
- Single commit: `timelock_vault_v1: absolute-ns timelock vault — block_timestamp lisp debut, 8-path pin`
- Do NOT push.

## Machine budget (HARD)
- ONE cargo lane at a time; `pgrep -f 'cargo|rustc'` first; `CARGO_BUILD_JOBS=2` always; no unattended full-suite runs.

## Report format
Phase 0 ABI result first (exact ns roundtripped or not), commit hash, test output for all 8 paths, any mock changes made (with justification), gas numbers for lk/wd if handy.
