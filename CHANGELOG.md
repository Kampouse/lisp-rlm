# Changelog

All notable changes to lisp-rlm are documented here.

---

## [0.5.0] — 7b443e5 — Runtime Execution Budget
### Added
- `eval_count` and `eval_budget` fields on `Env` struct
- Budget is incremented on every eval step; exceeding it raises a clean error
- Replaces the removed NEAR gas system with a lightweight, standalone mechanism
- 12 new tests for budget enforcement, exhaustion, and per-call reset behavior

---

## [0.4.0] — 7205d84 — Map/Filter Bytecode Fast Path + Bug Fix
### Added
- Bytecode fast path for `map` and `filter` when the function argument is a compilable lambda
- Graceful fallback to tree-walk eval when bytecode compilation fails (e.g., closures, complex bodies)
- 14 new tests covering fast-path correctness and fallback behavior

### Fixed
- `str-concat` double-quoting bug — strings are no longer wrapped in extra quotes when concatenated

---

## [0.3.0] — d026b87 — Macro System Fixes + 31 Tests
### Fixed
- `defmacro` arguments are no longer evaluated prematurely — raw unevaluated forms are passed correctly
- Quasiquote expansion now handles nested quasiquotation properly
- `unquote` evaluates and splices a single value correctly
- `unquote-splicing` flattens lists into the enclosing quasiquoted form (was incorrectly wrapping)

### Added
- `macroexpand` builtin for debugging macro expansion
- 31 new tests covering `defmacro`, `quasiquote`, `unquote`, `unquote-splicing`, and edge cases

---

## [0.2.0] — 32e43fb — Bytecode Compiler + VM
### Added
- Ported `bytecode.rs` from near-lisp with all near-sdk dependencies removed
- Bytecode compilation wired into eval's `loop` special form (compiles before tree-walk fallback)
- Bytecode lambda compilation for single-parameter lambdas in `map`/`filter`
- `stacker`-based stack protection for deep recursion
- Missing builtins added to helpers: `len`, `append`, `nth`
- `mod bytecode` and re-exports in `lib.rs`

### Removed
- NEAR gas system (check_gas, gas accounting) — replaced in v0.5.0 with eval budget
- All near-sdk imports and blockchain-specific code paths

---

## [0.1.0] — 991d32c — Initial Fork
### Added
- Standalone lisp-rlm interpreter forked from near-lisp
- Core tree-walk evaluator with `loop`/`recur` tail-call optimization
- S-expression parser (atoms, lists, quoting)
- Type system: `LispVal`, `Env`, `Lambda`
- Builtins: arithmetic (`+`, `-`, `*`, `/`), list ops (`cons`, `car`, `cdr`, `list`), predicates (`=`, `<`, `>`, `<=`, `>=`, `nil?`, `list?`, `number?`, `string?`)
- REPL binary (`src/bin/rlm.rs`)
- Test runner and benchmark harness
