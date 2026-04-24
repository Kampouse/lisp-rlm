# lisp-rlm — Standalone Lisp Interpreter for RLM Orchestration

STATUS: **COMPLETE** — All planned and follow-up work is done. The interpreter is fully functional with bytecode compilation, a macro system, fast-path optimizations, and runtime execution budgets.

---

## Project Summary

lisp-rlm is a standalone Lisp runtime forked from near-lisp, stripped of all NEAR/blockchain dependencies and rebuilt as a general-purpose interpreter with:

- **Tree-walk eval** with tail-call optimization (`loop`/`recur`)
- **Bytecode compiler + VM** for hot loops and lambda bodies
- **Hygienic macro system** (`defmacro`, `quasiquote`, `unquote`, `unquote-splicing`)
- **Fast-path optimization** for `map`/`filter` over compiled lambdas with graceful fallback
- **Runtime execution budget** (`eval_count` + `eval_budget` in `Env`) replacing the old gas system

---

## Completed Work

### Phase 1: Initial Fork (991d32c)
- Forked near-lisp into standalone `lisp-rlm`
- Removed all near-sdk dependencies and blockchain-specific code
- Established core interpreter: parser, eval, types, helpers
- Added `loop`/`recur` for iteration, basic builtins

### Phase 2: Bytecode Compiler + VM (32e43fb)
- Ported `bytecode.rs` from near-lisp with near-sdk stripped
- Wired bytecode compilation into eval's `loop` handler
- Removed gas system (replaced later with execution budget)
- Added stacker-based stack protection for deep recursion
- Updated `lib.rs` with `mod bytecode` and re-exports
- Added missing builtins (`len`, `append`, `nth`) to helpers

### Phase 3: Macro System (d026b87)
- Fixed `defmacro` — macro arguments are no longer evaluated prematurely
- Corrected quasiquote expansion logic
- Fixed `unquote` and `unquote-splicing` — splice flattening now handles nested cases
- Added `macroexpand` builtin for debugging
- 31 new tests covering all macro features

### Phase 4: Fast Path + Bug Fix (7205d84)
- Added bytecode fast path for `map` and `filter` when given compiled lambda arguments
- Graceful fallback to tree-walk eval when compilation fails
- Fixed `str-concat` double-quoting bug
- 14 new tests for fast-path and string operations

### Phase 5: Execution Budget (7b443e5)
- Implemented `eval_count` and `eval_budget` fields on `Env`
- Replaces the removed gas system with a lightweight evaluation counter
- Budget checked on each eval step; exceeds raise a clean error
- 12 new tests for budget enforcement and exhaustion behavior

---

## Architecture

### Source Files
| File | Purpose |
|------|---------|
| `src/bytecode.rs` | Bytecode compiler + VM (ported from near-lisp, no NEAR deps) |
| `src/eval.rs` | Tree-walk evaluator with loop/recur TCO, macro expansion |
| `src/helpers.rs` | Builtins: arithmetic, list ops, predicates, map/filter, str-concat |
| `src/types.rs` | `LispVal`, `Env`, `Lambda`, `BytecodeFn`, eval budget tracking |
| `src/parser.rs` | S-expression parser (atoms, lists, quoting, quasiquote) |
| `src/lib.rs` | Module declarations and public API |
| `src/bin/rlm.rs` | Main REPL binary |
| `src/bin/test_runner.rs` | Test harness |
| `src/bin/bench.rs` | Benchmark runner |
| `src/bin/minimal.rs` | Minimal REPL |

### What Was NOT Ported (NEAR-specific, correctly excluded)
- `contract.rs` (smart contract layer)
- `vm.rs` (yield/resume, ccall machinery)
- `near/*` builtins (storage, chain state, cross-contract calls)
- NEP-297 events
- near-sdk dependency
