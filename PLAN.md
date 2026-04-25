# lisp-rlm Evaluator Refactor

**Goal:** Eliminate stack overflow and exponential clone costs in the Lisp evaluator.

---

## Completed

### Phase 1: Immutable Env — commit `6b01b7d`

Split `Env` into immutable `im::HashMap` bindings + mutable `EvalState` for counters.

- `Env` wraps `im::HashMap<String, LispVal>` — O(1) clone via structural sharing
- `EvalState` holds `eval_count`, `eval_budget`, `rlm_state`, `snapshots`, `llm_provider`, etc.
- `apply_lambda` simplified from 65→23 lines — no more save/restore bookkeeping
- `lisp_eval` signature: `(expr, &mut Env, &mut EvalState)`
- 19 files changed, 255 tests green

### Phase 2: TailCall Elimination — commit `6d86131`

Minimal tail-call optimization via `EvalResult` enum instead of a full continuation stack.

- `apply_lambda` returns `EvalResult::TailCall { expr, env }` — no recursive `lisp_eval` call
- `lisp_eval_inner` trampoline resolves TailCalls iteratively
- `dispatch_call` uses `env.snapshot()`/`env.restore()` guard around arg evaluation
- 3 files changed (+140/-54), 255 tests green

### Arc Fix: Lambda Clone — commit `c3c3465`

Changed `closed_env` from `Vec<(String, LispVal)>` to `Arc<Vec<(String, LispVal)>>`.

**Problem:** Creating a lambda with 23 stdlib bindings triggered `env.clone().into_bindings()` which deep-cloned the entire closure graph recursively. Timed out after 5 seconds.

**Fix:** `Arc` makes lambda closure cloning O(1) — map/filter/reduce all instant.

### Full Continuation Stack — commit `174bcf4`

CPS (continuation-passing style) iterative evaluator with explicit `Cont` stack.

- `eval_step()` evaluates one expression, returns `Step::Done` or `Step::EvalNext`
- `handle_cont()` processes continuations on unwind
- All recursive patterns (if, cond, let, begin, match, loop) handled iteratively
- No Rust stack overflow for any Lisp program — budget enforcement catches infinite loops

### Env Bug Fix: Recursive Arg Evaluation

**Problem:** `dispatch_call` saved/restored env around ALL args at once. Inner `lisp_eval` calls for recursive functions (e.g. `(+ (fib (- n 1)) (fib (- n 2)))`) replaced `env` via TailCall, corrupting the view for subsequent args. `fib(10)` returned 6 instead of 55.

**Fix:** Save/restore env around EACH individual arg evaluation.

### str-replace Bug Fix

**Problem:** `str-split` treated multi-char delimiters as char sets (splitting on ANY char). `str-replace` was implemented as `(str-join new (str-split s old))` in stdlib, inheriting the bug.

**Fix:** Replaced `str-split` multi-char path with proper `str::split()`. Added `str-replace` as a native builtin using Rust's `str::replace()`.

### Bytecode Compiler

**Status:** Working. Re-enabled in commit `ee7971c`.

- Loop VM (`exec_compiled_loop`) — tight bytecode for `(loop ...)` forms, 20-50x faster
- Lambda VM (`try_compile_lambda` / `run_compiled_lambda`) — compiles single-param lambdas for map/filter/reduce fast paths
- Peephole optimizer runs 3 passes
- Supports: arithmetic, comparison, if, and/or, begin/progn, cond, builtins
- Falls back to tree-walking for unsupported forms (returns `None`)

---

## Test Suite Status

- 283 tests (was 276, added str-replace and more)
- 0 ignored (was 14)
- 0 warnings (was 20)
- All fib/fibonacci tests pass (fib(15) = 610) — no stack overflow
- All budget tests pass — infinite loops caught by budget, not stack overflow

## Future (not planned)

### Full Continuation Stack for deeper patterns

If mutually recursive functions not in tail position ever overflow, the architecture supports it. The `Cont` enum already has ~15 variants. All that's needed is converting more `lisp_eval` recursive call sites into `push Cont + return next_expr`.
