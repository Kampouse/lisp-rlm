# TASK: Update stale tests to nil-on-miss semantics

> **Status: DONE** — merged. Full history in `git log`.

**Repo:** `/Users/asil/.openclaw/workspace/lisp-rlm` (main = 20e9b0e, includes the storage-read cache)
**Branch:** `fix/nil-on-miss-tests` off `main`. NEVER commit on main, NEVER push.
**Timebox:** ~45 min. This is test-only surgery — **do not change any non-test source.**

## Background

Commit `e48d64c` (Sunday night) changed `storage_get` to return **nil on miss** (was: empty string). It shipped without a battery re-run, and 17 tests still assert the OLD semantics. The runtime change is CORRECT and shipped — the tests are stale.

Current failing set (verify with `cargo test -p lisp-rlm-wasm --test test_budget --test test_safe_corpus --test test_storage_family --test test_type_system_gaps --test test_wallet_diff --no-fail-fast`):
- test_storage_family: wasm_shared_storage_lifecycle, wasm_fresh_memory_persistence, interp_storage_family_lifecycle
- test_type_system_gaps: gap_c1_storage_get_return_type_str_but_can_be_nil, gap_d1_promise_and_variadic_emitter_strict_tc, gap_d2_promise_result_0arg_emitter_1arg_tc + others
- test_wallet_diff: wallet_factory_init, wallet_factory_valid_wasm, full_wallet_factory_compiles, full_factory_init_state
- test_safe_corpus: 1
- test_budget: possibly test_p2_native_http — this one is an ORDER-DEPENDENT FLAKE (/tmp race), NOT semantics — leave it, document only.

## Job

1. Run the targets, read each failing test's intent, update assertions to nil-on-miss semantics. For "gap" tests that PIN the old gap: flip them to pin the NEW resolved behavior (follow the t20 / nm_zero_is_falsy precedent for pinned-semantics tests).
2. If any test turns out to expose a REAL runtime bug (not stale expectations), STOP that test, mark with a `// REAL BUG:` comment + report it — do not force it green.
3. Some failures may be type-checker level (checker says `str`, runtime says `str|nil`) — if the fix belongs in checker/test-harness typing rather than test expectations, you may adjust TEST harness helpers only, not src/. If it genuinely needs src/ changes, report instead.
4. Verify: the 5 targets fully green (minus the p2 flake), then `cargo test --workspace --no-fail-fast` — expect only the p2 order-dependent flake remaining red (verify it passes 7/7 in isolation like baseline).

## Rules
- NEVER `git add -A` — explicit paths only (tests/ + nothing else ideally).
- No pushes, no main commits. Report: per-test change list, final counts.
