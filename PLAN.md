# lisp-rlm Optimization Plan — Port near-lisp bytecode compiler

## Goal
Merge near-lisp's bytecode compiler + VM optimizations + all non-NEAR features into lisp-rlm, creating a fast standalone Lisp runtime.

## Architecture
Port bytecode.rs (strip near-sdk deps), wire it into eval.rs loop/recur path + map/filter HOFs, and add the `len`/`append`/`nth` builtins that are missing.

## Files to modify
1. `src/bytecode.rs` — NEW: port from near-lisp, strip near-sdk imports
2. `src/eval.rs` — add bytecode call in loop handler + lambda fast-path in map/filter
3. `src/lib.rs` — add `mod bytecode;` + re-exports
4. `src/helpers.rs` — add missing builtins to `is_builtin_name`

## What's NOT being ported (NEAR-specific)
- contract.rs (smart contract layer)
- vm.rs (yield/resume, ccall machinery)
- near/* builtins (storage, chain state, cross-contract calls)
- NEP-297 events
- near-sdk dependency

## Task breakdown

### Task 1: Create bytecode.rs (port from near-lisp)
- Copy near-lisp's bytecode.rs
- Remove `use near_sdk::*` imports (line 1-6)
- Replace with `use std::collections::{BTreeMap, HashMap};` only
- The `crate::types::check_gas` call already works standalone

### Task 2: Wire bytecode into eval.rs loop handler
- Add `use crate::bytecode::{exec_compiled_loop, try_compile_loop};`
- In the "loop" special form, after computing binding_vals, try `try_compile_loop` before tree-walk fallback
- Same pattern as near-lisp eval.rs

### Task 3: Wire bytecode into map/filter (lambda fast-path)
- In dispatch_call's "map" and "filter" handlers, add compiled lambda fast-path
- Use `try_compile_lambda` + `run_compiled_lambda` when lambda has 1 param

### Task 4: Add missing builtins to helpers.rs
- `len` and `length` (near-lisp has both, lisp-rlm only uses `len`)
- `append`, `nth` (present in lisp-rlm's is_builtin_name but verify)
- Add `defmacro` and `macroexpand` to is_builtin_name for forward compat

### Task 5: Update lib.rs
- Add `mod bytecode;`
- Add bytecode re-exports

### Task 6: Build + test
- `cd /tmp/lisp-rlm && cargo build`
- Run REPL and test loop/recur performance
- Test map/filter with lambdas
