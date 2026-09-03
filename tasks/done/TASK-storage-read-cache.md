# TASK: Storage-Read Cache (wasm emitter) — biggest real-contract gas win

> **Status: DONE** — merged. Full history in `git log`.

**Branch:** `perf/storage-read-cache` (branch off current `main`). NEVER commit to main, NEVER push.
**Repo:** `/Users/asil/.openclaw/workspace/lisp-rlm`
**Timebox:** ~90 min. Commit early and often. A partial, well-tested result + honest report beats a sprawling half-working rewrite.

## Context

lisp-rlm compiles a Lisp/TS dialect to NEAR wasm. Today every `(storage_get "k")` / read builtin emits a raw imported `storage_read` host call. Contracts that read the same keys repeatedly (oracles, counters, FTs) pay full host-call gas every time. Within one transaction, storage is immutable except through our own `storage_set!` — so reads of the same key can be memoized.

## Goal

Emit a per-transaction memo cache for storage reads in the **wasm emitter only** (interp is out of scope):
1. First read of key K → imported storage_read, remember result.
2. Later reads of K in the same tx → serve from cache (linear memory), no host call.
3. Any `storage_set!`/write path → update or invalidate K's entry (semantics must be exact: set then get must return the new value).
4. Miss-vs-nil semantics: `storage_get` returns nil on miss (fixed recently in e48d64c — cache must preserve nil-on-miss, including "key was set then deleted within tx" behavior if a delete exists).

## Method (STRICT order)

1. **Explore (15 min):** find where storage builtins are emitted (grep `storage_get`/`storage_read` in `src/wasm_emit/`, `src/checker` for typing). There are TWO define-body emission sites in `compile_near` paths (lesson from annotation work: ~line 716 source path + ~line 1160 from_exprs path) — whatever transform you do must land in BOTH if it touches define bodies.
2. **Baseline BEFORE coding:** build a bench program that reads the same key N times in a loop + a nostr-gov-shaped read pattern; record gas with the existing runner (`near-vm-run` / whatever `diff-ts.sh`-adjacent harness exists — look in `projects/`, `tests/`). Numbers in a table.
3. **Design note (10 min):** write 10 lines in the task report: cache layout (suggest: fixed region in linear memory, open-addressed or linear-scan slot table; keys are short strings — cap entry count, fall back to uncached on overflow), invalidation points.
4. **Implement guarded:** keep the cache OFF by default or trivially correct — simplest correct version first (even "cache only exact-match key string literals" is a win if common). NO speculative complexity.
5. **Verify:**
   - `cargo test --workspace --no-fail-fast` must stay green (currently 1426+/0).
   - Value-equivalence: corpus / nostr-gov differential must still pass (gas numbers may change; VALUES must be identical). If a harness compares gas traces, adapt expectations honestly and say so.
   - Test through **near-vm-run** (it is STRICTER than wasmtime — PrepareError on malformed modules that wasmtime tolerates).
   - Add focused tests: read-read, set-then-read, read-set-read, miss-nil, cache overflow fallback.
6. **After-gas table:** same benches, before vs after. If the win is <3% on realistic shapes, STOP and report — the design may not be worth landing.

## Rules

- NEVER `git add -A` — shared tree. Stage explicit paths only. (A prior agent swept WIP this way.)
- Do not touch: `stash@{0}` (call_string WIP), branch `wip/kampouse-line`, `projects/rust-twin-bench/`, untracked files not yours.
- Machine-verify any constant you embed in tests (python3) — hand-computed constants have burned us 6× in one day.
- Report: commits list, bench tables, what's guarded/unfinished, known risks.
