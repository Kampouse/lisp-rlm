# Tree-Walker Removal: Status & Remaining Work

## Summary

Deleted the tree-walking evaluator (~4,200 lines) from the Lisp-RLM runtime. The bytecode VM is now the sole evaluation path. All binaries, REPL, and test helpers migrated to `program::run_program`.

**Date:** 2026-04-29
**Test baseline:** 302 pass / 315 fail (49%)

---

## What Was Deleted

| File | Before | After | Delta |
|------|--------|-------|-------|
| `src/eval/cps_eval.rs` | 2,204 lines | deleted | -2,204 |
| `src/eval/continuation.rs` | 134 lines | deleted | -134 |
| `src/eval/mod.rs` | 2,291 lines | 430 lines | -1,861 |
| `src/bytecode.rs` | 3,724 lines | 3,488 lines | -236 |
| **Total** | | | **-4,435** |

## What Remains in eval/mod.rs (430 lines)

- `json_to_lisp` / `lisp_to_json` — JSON helpers (used by dispatch_json)
- `RLM_SYSTEM_PROMPT` — constant
- Stub `lisp_eval` — delegates to `program::run_program(&[expr], env, state)` (backward compat for benchmarks)
- Stub `apply_lambda` — returns error
- Stub `dispatch_call_with_args` — delegates to `eval_builtin`
- Stub `call_val` — delegates to `vm_call_lambda`
- 24 unit tests

## Bugs Fixed During Migration

1. **`nth` arg order** — was `(Num, List)`, should be `(List, Num)`
2. **`reverse nil`** — returned `nil` instead of `()`
3. **`eval_builtin` dispatch fallback** — `if let Ok(Some)` silently swallowed `Err` from dispatch modules
4. **Hardcoded `dict`** — `eval_builtin` had `"dict" => Ok(LispVal::Map(HashMap::new()))` ignoring arguments
5. **LLM test assertions** — updated for new "unknown builtin" error (LLM builtins not wired into eval_builtin yet)

## Test Results by File

| Test File | Pass | Fail | Notes |
|-----------|------|------|-------|
| lib (unit) | 24 | 0 | |
| core_language | 120 | 40 | match, variadic, progn, try/catch, mod edge cases |
| fuzz_test | 7 | 0 | GREEN |
| test_bytecode_shadow | 2 | 0 | GREEN |
| test_compiler_v2 | 29 | 0 | GREEN |
| test_shadow_minimal | 1 | 0 | GREEN |
| test_compiler_extensions | 11 | 5 | |
| test_compiler_v3 | 9 | 9 | |
| test_fast_path | 11 | 3 | |
| test_hof_fastpaths | 23 | 1 | |
| test_lambda_hof | 5 | 3 | |
| test_stdlib_tier1 | 16 | 10 | |
| test_runtime_features | 13 | 27 | |
| test_macros | 10 | 21 | |
| test_pure_types | 2 | 13 | |
| test_types_extended | 6 | 35 | |
| test_pure_probe_arrow | 3 | 19 | |
| test_budget | 5 | 7 | |
| test_types | 1 | 17 | |
| test_loop_recur | 3 | 8 | |
| test_bytecode_coverage | 1 | 10 | |
| test_edge | 0 | 3 | |
| test_harness | 0 | 3 | load-file + env persistence |
| test_harness_extended | 0 | 36 | load-file + env persistence |
| test_harness_full | 0 | 29 | load-file + env persistence |
| test_closure_mutation | 0 | 5 | |
| test_compiler_bugs | 0 | 4 | |
| norvig_tests | 0 | 1 | builtins as first-class values |
| test_fb_compile | 0 | 1 | |
| test_pb | 0 | 1 | |
| test_set_captured | 0 | 1 | |
| test_set_compile | 0 | 1 | |
| test_shadow_debug | 0 | 1 | |
| test_shadow_trace | 0 | 1 | |
| **TOTAL** | **302** | **315** | |

---

## Failure Categories & Root Causes

### Category 1: Compiler doesn't support the construct
These forms fail with "compilation failed for desugared program":
- **`match`** — 15+ tests. The `match` special form is not compiled by the bytecode compiler.
- **`progn`** — multi-expression body in non-lambda context.
- **`try`/`catch`** — error handling special form.
- **`macros`** — 21 tests. `defmacro`, macro expansion not compiled.
- **`type` declarations** — `deftype`, `type?` predicates in test_pure_types/test_types_extended.

### Category 2: Builtins aren't first-class values
Builtins like `+`, `list`, `append` have no `LispVal` representation. When passed as function arguments (e.g., `(reduce + 0 args)`, `(compose list twice)`), they resolve to `nil` or `Sym` instead of being callable.
- **variadic tests** — `(reduce + 0 args)` fails
- **norvig tests** — `(compose list twice)` fails
- **hof fastpaths** — passing builtins to map/filter/reduce

### Category 3: `load-file` env persistence
`program::run_program` snapshots the env, so `define` inside loaded files doesn't persist to the outer env. The tree-walker's `define` did `env.push(name, value)` directly.
- **test_harness** — 3 failures (load-file + boot)
- **test_harness_extended** — 36 failures (all depend on loaded harness.lisp)
- **test_harness_full** — 29 failures (same)

### Category 4: Missing VM opcodes
- **`set!` on captured variables** — needs `StoreCaptured(idx)` opcode
- **Zero-arg lambdas** — compiler rejects `params.is_empty()`

### Category 5: Builtin behavior gaps
- **`mod` edge cases** — mod with negative numbers, zero divisor
- **`take`/`drop`** — arg order issues (similar to `nth` bug)
- **`str-concat` in match patterns** — compilation issue
- **`partition`** — depends on builtins as values

---

## Attack Plan

### Phase 1: Quick wins (builtin fixes)
- [ ] Audit all 2-arg builtins for swapped arg order (like `nth` was)
- [ ] Fix `take`/`drop` arg order if swapped
- [ ] Fix `mod` edge cases

### Phase 2: Builtins as first-class values
- [ ] Add `LispVal::BuiltinFn(String)` variant
- [ ] Populate env with builtin fn values at startup
- [ ] Update `vm_call_lambda` to dispatch `BuiltinFn` through `eval_builtin`
- [ ] Fix norvig tests (compose with builtins)

### Phase 3: Compiler gaps
- [ ] Implement `match` compilation (biggest win — 15+ tests)
- [ ] Implement `progn` compilation
- [ ] Implement `try`/`catch` compilation
- [ ] Implement `set!` on captured vars (`StoreCaptured` opcode)

### Phase 4: load-file env persistence
- [ ] Fix `run_program` to sync `StoreSlot` defines back to outer env
- [ ] Or: fix harness tests to use single `run_program` call for multi-form programs

### Phase 5: Advanced features
- [ ] Macro compilation
- [ ] Type system compilation
- [ ] Zero-arg lambda support

---

## Verification Commands

```bash
# Full test suite
RUST_MIN_STACK=16777216 cargo test --tests

# Individual test files
RUST_MIN_STACK=16777216 cargo test --test core_language
RUST_MIN_STACK=16777216 cargo test --test norvig_tests

# Quick smoke test
RUST_MIN_STACK=16777216 cargo test --lib
```
