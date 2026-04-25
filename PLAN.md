# lisp-rlm Evaluator Refactor

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

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
- `lisp_eval_inner` trampoline resolves TailCalls iteratively: `current_expr = expr; *env = tail_env; continue '_trampoline`
- `dispatch_call` uses `env.snapshot()`/`env.restore()` guard around arg evaluation to prevent TailCall env corruption
- Macro TailCall resolution preserves caller env via `saved_env`
- `dispatch_collections.rs` local `call_val` wrapper resolves TailCalls back to `LispVal`
- 3 files changed (+140/-54), 255 tests green

**Why minimal TailCall instead of full continuation stack:** The stack overflow came from the `lisp_eval → dispatch_call → call_val → apply_lambda → lisp_eval` chain. Breaking that one link (via TailCall) eliminates the problem. Other recursive patterns (if-chains, begin sequences, let nesting) are bounded by program structure, not self-referential loops. A full CPS rewrite (the original Tasks 7-12) would be ~400 net LOC for marginal benefit.

### Arc Fix: Lambda Clone — commit `c3c3465`

Changed `closed_env` from `Vec<(String, LispVal)>` to `Arc<Vec<(String, LispVal)>>`.

**Problem:** Creating a lambda with 23 stdlib bindings triggered `env.clone().into_bindings()` which deep-cloned the entire closure graph recursively. Timed out after 5 seconds.

**Fix:** `Arc` makes lambda closure cloning O(1) — map/filter/reduce all instant.

---

## In Progress

### Bytecode Compiler Fix

**Status:** Disabled in `try_compile_lambda()` (returns `None`). Has an infinite loop bug.

The loop VM (`exec_compiled_loop`) works — used for `(loop ((i 0)) ... (recur ...))`. The lambda compiler (`try_compile_lambda` / `run_compiled_lambda`) is disabled because it enters infinite loops on certain lambda bodies.

**Files:**
- `src/bytecode.rs` — 1475 lines. Loop VM works, lambda VM disabled.
- `src/eval/dispatch_collections.rs` — calls `try_compile_lambda` in map/filter fast paths (currently falls through to `apply_lambda`)

**Plan:**
1. Write a minimal failing test — a lambda that causes the infinite loop
2. Diagnose: likely a missing `Return` op or infinite jump in the compiled code
3. Fix and re-enable
4. Verify map/filter/reduce fast paths work via existing tests

---

## Future (not planned)

### Full Continuation Stack

If deeper recursion patterns (mutually recursive functions not in tail position) ever overflow, a full continuation stack could be added. The architecture from the original Tasks 6-12 would work:

- `Cont` enum with ~15 variants owns its own `Env` (O(1) clone)
- `lisp_eval` becomes a `loop` over `eval_step()` results
- All 27 recursive call sites become `push Cont + return next_expr`
- Estimated ~400 net LOC

This is **not needed** for the current use case — TailCall handles the problematic chain.
