# Full Continuation Stack — Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Convert the entire evaluator from recursive to iterative, eliminating ALL stack overflow potential.

**Architecture:** Replace `lisp_eval_inner`'s `'_trampoline` loop + recursive `lisp_eval()` calls with a single `Vec<Cont>` stack. Each special form pushes a continuation describing "what to do with the result", then returns the next expression to evaluate. The main loop is flat — no recursion at all.

**Why:** The TailCall fix (Phase 2) eliminated stack overflow from the lambda chain, but there are 24 other recursive `lisp_eval()` calls in special forms (if, cond, let, begin, and, or, try, match, loop, define, set!, etc). Deep nesting of these still grows the native stack. A full continuation stack makes the evaluator fully iterative.

**Tech Stack:** `im::HashMap` for O(1) Env cloning (already done in Phase 1).

---

## Overview of the Conversion

The current `lisp_eval_inner` is a `'_trampoline: loop` that handles:
1. **Atoms** — return immediately (no continuation needed)
2. **Special forms** — evaluate sub-expressions recursively via `lisp_eval()`
3. **Function calls** — delegate to `dispatch_call()` which evaluates args recursively

The new architecture:
```
lisp_eval(expr, env, state) {
    let mut stack: Vec<Cont> = vec![];
    let mut current = expr;
    let mut current_env = env;

    loop {
        // budget check
        let value = eval_step(current, &mut current_env, state)?;
        // eval_step returns Step::Done(v) or Step::EvalNext { expr, env, cont }

        match value {
            Step::Done(v) => {
                // Pop continuations, compute next action
                loop {
                    match stack.pop() {
                        None => return Ok(v),
                        Some(Cont::...) => compute next current/current_env from v + cont
                    }
                }
            }
            Step::EvalNext { expr, env, cont } => {
                if let Some(c) = cont { stack.push(c); }
                current = expr;
                current_env = env;
            }
        }
    }
}
```

### Continuation Variants Needed

Based on the 24 recursive call sites in `lisp_eval_inner`:

| Cont Variant | Source Special Form | What it does with the value |
|---|---|---|
| `DefineSet { name }` | `define` | Bind `name` in env |
| `IfBranch { then, els }` | `if` | Pick branch based on truthiness |
| `CondTest { arms, idx }` | `cond` | Test next arm |
| `LetVal { names, vals, remaining, body }` | `let` | Accumulate binding value |
| `BeginSeq { remaining }` | `begin`/`progn` | Eval next, discard current |
| `AndShort { remaining }` | `and` | Short-circuit if falsy |
| `OrShort { remaining }` | `or` | Short-circuit if truthy |
| `NotArg` | `not` | Negate truthiness |
| `TryBody { catch_var, catch_body }` | `try` | Catch errors |
| `MatchScrutinee { arms }` | `match` | Pattern match |
| `LoopEvalBindings { names, remaining_vals, body }` | `loop` | Evaluate binding init vals |
| `LoopBody { names, vals }` | `loop` | Run body, handle recur |
| `RecurArgs { count }` | `recur` | Collect args into Recur |
| `SetVal { name }` | `set!` | Set variable |
| `FinalVal` | `final` | Store in rlm_state |
| `AssertCheck { msg }` | `assert` | Check condition |
| `RequireModule { name, body, module_env }` | `require` | Evaluate module body |
| `Quasiquote` | `quasiquote` | Expand then eval |
| `EvalExpanded` | (macro result) | Eval expanded code |
| `RlmSetVal { name }` | `rlm-set` | Store in rlm_state |
| `ArgEval { head, remaining, done, env }` | function call | Evaluate next arg |
| `HeadEval { args }` | function call | Eval head, then eval args |

### What stays recursive

Some calls are in `dispatch_call` and `call_val` — not in the main eval loop. These stay as-is because they're not in the deep recursion path:
- `dispatch_call` → arg evaluation loop (can be converted to ArgEval continuation)
- `call_val` → macro expansion (rare, bounded depth)
- `rlm_fractal`, `llm-code`, etc. → LLM calls (IO-bound, not stack-bound)

---

## Task Breakdown

### Task 1: Define Cont enum and Step type

**Objective:** Add continuation and step types to `continuation.rs`.

**Files:**
- Modify: `src/eval/continuation.rs`

Add `Step` enum and `Cont` enum with all variants listed above.

### Task 2: Rewrite `lisp_eval` as iterative loop

**Objective:** Replace the current `lisp_eval` + `lisp_eval_inner` with a single function using `Vec<Cont>`.

**Files:**
- Modify: `src/eval/mod.rs`

Delete `lisp_eval_inner`. Rewrite `lisp_eval` to use the `Step`/`Cont` machinery. Move all special form logic into `eval_step()`.

### Task 3: Convert simple special forms (quote, if, cond, define, set!, not, and, or, begin)

**Objective:** Handle the 9 simplest special forms that have 1-2 recursive calls.

**Files:**
- Modify: `src/eval/mod.rs`

### Task 4: Convert let, let*, match, try/catch

**Objective:** Handle binding forms and error handling.

**Files:**
- Modify: `src/eval/mod.rs`

### Task 5: Convert loop/recur, require, final, assert, rlm-set

**Objective:** Handle remaining special forms.

**Files:**
- Modify: `src/eval/mod.rs`

### Task 6: Convert function call path (dispatch_call arg evaluation)

**Objective:** Make arg evaluation in `dispatch_call` use `ArgEval` continuation instead of recursive `lisp_eval`.

**Files:**
- Modify: `src/eval/mod.rs`

### Task 7: Run full test suite, fix regressions

**Objective:** All 255 tests pass.

### Task 8: Commit and push

---

## Key Design Decisions

1. **`Cont` owns `Env`** — each continuation captures the env at push time. O(1) with `im::HashMap`.
2. **`eval_step` returns `Step`** — either `Done(value)` (atom/immediate) or `EvalNext { expr, env, cont }` (need to evaluate sub-expression).
3. **`dispatch_call` stays mostly as-is** — the arg eval loop becomes an `ArgEval` continuation in the main loop.
4. **TailCall is subsumed** — `EvalResult::TailCall` from `apply_lambda` is handled naturally: the main loop just sets `current = body; current_env = local_env` without pushing a continuation.
5. **`call_val` stays recursive** — it's not in the deep path. Macro expansion is bounded.

## Testing Strategy

After every task: `cargo test -- -q` must show 255 passed, 0 failed.
