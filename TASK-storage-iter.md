# TASK: storage-iter builtins + storage cleaner contract

**Goal:** Expose NEAR key-enumeration to Lisp contracts, then ship a `clean()` contract that wipes ALL contract storage. Needed to reset gcpool26.testnet after a code redeploy left stale state.

**Fenced paths (touch ONLY these):** `src/verifier.rs`, `src/typing/types.rs`, `src/bytecode/mod.rs`, `src/wasm_emit/call_near_storage.rs`, `examples/storage_cleaner.lisp`, plus one focused test file if needed. **Do NOT touch** `data/`, `scripts/rlm-tasks/`, `dashboard/`, or anything else — RLM runtime loops write there constantly. **Do NOT git commit** — leave the working tree for review.

## New builtins (both dash and underscore name forms, house convention)

1. `(storage-iter-prefix prefix:str) → num`
   - NEAR host fn 36: `storage_iter_prefix(prefix_len u32, prefix_ptr u32) -> u64 iterator_id`
   - `""` prefix = iterate ALL keys. Return the id as a tagged num.
2. `(storage-iter-next iter-id:num) → str|nil`
   - NEAR host fn 38: `storage_iter_next(iterator_id u64, key_register u64, value_register u64) -> u32` (1 = got entry, 0 = exhausted)
   - Use key_register=1, value_register=2 (cheap; we ignore value).
   - On 1: `register_len(1)` (host 1) → bump-allocate from RUNTIME_HEAP_PTR (addr 56, 8-aligned), `read_register` (host 0) into it, return tagged string (high32=len, low32=ptr) — mirror the `near/storage_read` miss path in `src/wasm_emit/call_near_storage.rs` (~line 415+, locals `__sg_len/__sg_tmp/...` pattern, `Self::host_call(N)`, `emit_tag_str`).
   - On 0: return `TAG_NIL`.
   - `need_host(36)`, `need_host(38)`, `need_host(1)`, `need_host(0)` where applicable.

## Four-surface discipline (ALL of these, or don't ship)

1. `src/verifier.rs` — add names to the known-builtins list (see storage-write/read/remove entries ~line 89).
2. `src/typing/types.rs` — add to the builtin dispatch (~line 51, where `storage-clear-all` already appears in a match arm).
3. `src/bytecode/mod.rs` — interpreter: implement against the mock NEAR storage map. See existing `storage-write`/`storage-read`/`storage-remove` dispatch ~lines 4936–5170 and the mock storage state in `src/types.rs` (`near_promises`, storage map fields) / `src/bin/near_mock.rs`. Iteration semantics: `iter-prefix` returns an id bound to a sorted key list + cursor; `iter-next` returns current key and advances, `nil` when exhausted. **Removing keys during iteration must work** (iterate a snapshot of sorted keys, advance cursor past the returned key).
4. `src/wasm_emit/call_near_storage.rs` — the `"near/storage_iter_prefix"` / `"near/storage_iter_next"` match arms as spec'd above. Arg-count errors in house style: `"near/storage_iter_prefix: need exactly 1 arg (prefix)"`.

## The contract — `examples/storage_cleaner.lisp`

Mirror an existing callable-method example for the export/entry convention (look at how methods receive JSON args — check other `examples/*.lisp` contracts first; `get_pool`-style views suggest plain `(define (method) ...)`).

```lisp
;; entry: clean() — wipes ALL storage keys, returns number removed
;; loop: (storage-iter-next id) → key | nil ; (storage-remove key) ; recurse/loop
```

- Count removals, return the count (so a caller can re-invoke until 0).
- No nil? predicate assumptions — check what the null-check idiom actually is in this codebase (grep how `storage-read` misses are checked in existing examples, e.g. `(equal? x nil)` or `??` default op) and use that.
- Keep it burn-after-use simple. No owner guard (cleaner contract is temporary).

## Build & test (all local — NO chain deploys)

1. `cargo build --bin near_compile` must pass clean.
2. `cargo test` — existing suite green (at least bytecode + typing + wasm_emit tests).
3. New focused test (wherever the existing storage builtin tests live — find them first): write 3 keys via storage-write, `storage-iter-prefix ""`, loop `storage-iter-next`, collect keys, `storage-remove` each, assert `storage-has-key` false for all three. Also test non-empty prefix.
4. `./target/release/near-compile build` the cleaner project (create a `storage-cleaner/` scaffold with near.json like other example projects — copy the layout from an existing one) → wasm builds, no emit errors.

## Report back

- Diff summary per file, test output tail, and the exact near.json/wasm paths. Flag any landmine you hit (interp vs emitter semantics drift is the known killer here — if the two can't agree on semantics, STOP and report instead of shipping a divergence).
