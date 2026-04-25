# Immutable Env + Continuation Stack Refactor

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Eliminate stack overflow by converting the evaluator from recursive to iterative with an explicit continuation stack, unlocked by making Env immutable via persistent data structures.

**Architecture:** Two-phase refactor. Phase 1 makes Env immutable by splitting bindings (`im::HashMap`) from mutable counters (`EvalState`). Phase 2 replaces the recursive `lisp_eval` with an iterative loop + `Vec<Continuation>`. The immutable Env is a prerequisite — it lets continuations own their own Env without borrow checker fights or `RefCell`.

**Tech Stack:** `im` crate (persistent HashMap with structural sharing, O(log32 n) ops, O(1) clone)

**Why this order:** Phase 1 alone already eliminates the save/restore bookkeeping in `apply_lambda` (30 lines deleted). Phase 2 eliminates stack overflow entirely. Phase 1 without Phase 2 still compiles and passes all tests — each phase is independently correct.

---

## Phase 1: Immutable Env

### Task 1: Add `im` dependency

**Objective:** Add the `im` crate to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

**Step 1:** Add `im = "0.5"` to `[dependencies]`, remove `stacker = "0.1"`

**Step 2:** Run `cargo check` to verify dependency resolves

**Step 3:** Commit

```bash
git commit -m "chore: add im crate, remove stacker"
```

### Task 2: Split Env into Bindings + EvalState

**Objective:** Separate immutable bindings from mutable evaluation counters.

**Files:**
- Modify: `src/types.rs`

**Step 1:** Add `EvalState` struct for mutable counters:

```rust
/// Mutable evaluation state — passed through the eval loop but never captured in continuations.
pub struct EvalState {
    pub eval_count: u64,
    pub eval_budget: u64,
    pub rlm_state: im::OrdMap<String, LispVal>,  // persistent BTreeMap equivalent
    pub tokens_used: usize,
    pub llm_calls: usize,
    pub rlm_depth: usize,
    pub rlm_iteration: usize,
    pub snapshots: Vec<im::HashMap<String, LispVal>>,
    pub llm_provider: Option<Box<dyn crate::eval::llm_provider::LlmProvider>>,
}
```

**Step 2:** Replace `Env` bindings storage:

```rust
pub struct Env {
    bindings: im::HashMap<String, LispVal>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            bindings: im::hashmap! {
                "t".into() => LispVal::Bool(true),
                "true".into() => LispVal::Bool(true),
                "false".into() => LispVal::Bool(false),
            },
        }
    }

    pub fn get(&self, name: &str) -> Option<&LispVal> {
        self.bindings.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn insert(&self, name: String, val: LispVal) -> Self {
        let mut new = self.bindings.clone();  // O(1) structural sharing
        new.insert(name, val);
        Env { bindings: new }
    }

    pub fn insert_mut(&mut self, name: String, val: LispVal) {
        self.bindings.insert(name, val);
    }

    // Remove truncate — no longer needed. Immutable: just drop the extended env.
    // Remove push_saving! — no longer needed.
    // Remove into_bindings — no longer needed.

    pub fn iter(&self) -> im::hashmap::Iter<String, LispVal> {
        self.bindings.iter()
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut LispVal> {
        self.bindings.get_mut(name)
    }

    pub fn clear(&mut self) {
        self.bindings.clear();
    }
}
```

**Step 3:** Implement `Clone` for `Env` — derived automatically since `im::HashMap` is `Clone`.

**Step 4:** Implement `Clone` for `EvalState` — same as current Env clone but without bindings. `llm_provider` set to `None` (same as current behavior).

**Step 5:** `cargo check`. Will have compile errors everywhere `env.eval_count`, `env.push`, `env.truncate` etc. are used. That's expected — fixed in Tasks 3-5.

### Task 3: Update `lisp_eval` signature

**Objective:** Change the public API to accept `Env` (owned/immutable) + `&mut EvalState`.

**Files:**
- Modify: `src/eval/mod.rs`

**Step 1:** Change signature:

```rust
pub fn lisp_eval(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    if state.eval_budget > 0 {
        state.eval_count += 1;
        if state.eval_count > state.eval_budget {
            return Err(format!("execution budget exceeded: {} iterations", state.eval_count));
        }
    }
    // Remove stacker::maybe_grow — no longer needed
    lisp_eval_inner(expr, env, state)
}
```

**Step 2:** Update all internal recursive calls in `lisp_eval_inner` to pass `state`:

Every `lisp_eval(x, env)` → `lisp_eval(x, env, state)`

~27 call sites, each a 1-line change. Mechanical search-and-replace.

**Step 3:** `cargo check`. More errors in callers — fixed in Task 4.

### Task 4: Update `apply_lambda` — eliminate save/restore

**Objective:** Replace the 30-line save/restore pattern with a scoped Env clone.

**Files:**
- Modify: `src/eval/mod.rs`

**Step 1:** Rewrite `apply_lambda`:

```rust
pub fn apply_lambda(
    params: &[String],
    rest_param: &Option<String>,
    body: &LispVal,
    closed_env: &std::sync::Arc<Vec<(String, LispVal)>>,
    args: &[LispVal],
    env: &Env,
    state: &mut EvalState,
) -> Result<(LispVal, Env), String> {
    // Create scoped env — original is never modified
    let mut local_env = env.clone();  // O(1) with im::HashMap

    for (k, v) in closed_env.iter() {
        local_env.insert_mut(k.clone(), v.clone());
    }
    for (i, p) in params.iter().enumerate() {
        local_env.insert_mut(p.clone(), args.get(i).cloned().unwrap_or(LispVal::Nil));
    }
    if let Some(rest_name) = rest_param {
        let rest_args: Vec<LispVal> = args.get(params.len()..).unwrap_or(&[]).to_vec();
        local_env.insert_mut(rest_name.clone(), LispVal::List(rest_args));
    }

    let result = lisp_eval(body, &mut local_env, state)?;
    Ok((result, local_env))
    // local_env is dropped or returned. Original env untouched.
}
```

Note: returns `(LispVal, Env)` — the possibly-modified env (for `define`/`set!` inside lambda bodies). Actually for correctness, `define` inside a lambda should be local, so we can just return the result and drop local_env. But `set!` on closed-over variables... needs thought. The current code mutates caller_env and restores. With immutable, the mutation is local. This is actually MORE correct — `set!` inside a lambda shouldn't leak to caller.

**Wait** — `set!` IS supposed to mutate closed-over bindings. Let me check...

Actually the current `apply_lambda` pushes closed_env bindings, then pushes params. If the body does `(set! x 5)` where `x` was from closed_env, it mutates the local copy. After restore, the original is... unchanged. So the current behavior already doesn't propagate set! to outer scope. Immutable gives the same behavior. Good.

Return type: just `Result<LispVal, String>` — env doesn't leak.

### Task 5: Update all callers of `lisp_eval`

**Objective:** Update every file that calls `lisp_eval` or creates `Env`.

**Files:**
- Modify: `src/lib.rs` — update doc comments
- Modify: `src/bin/smoke_test.rs`, `src/bin/rlm.rs`, `src/bin/minimal.rs`, `src/bin/bench.rs`, `src/bin/test_runner.rs`
- Modify: `tests/norvig_tests.rs`, `tests/test_fast_path.rs`, `tests/test_lambda_hof.rs`, `tests/test_macros.rs`, `tests/fuzz_test.rs`, `tests/core_language.rs`, `tests/test_budget.rs`
- Modify: `src/eval/dispatch_collections.rs`
- Modify: `src/eval/dispatch_state.rs`
- Modify: `src/bytecode.rs`

**Pattern for all callers:**

Before:
```rust
let mut env = Env::new();
let result = lisp_eval(&expr, &mut env)?;
```

After:
```rust
let mut env = Env::new();
let mut state = EvalState::new();
let result = lisp_eval(&expr, &mut env, &mut state)?;
```

**Step 1:** Update all `dispatch_*` functions to accept `&mut EvalState` and pass it through.

**Step 2:** Update `dispatch_state.rs` — the `rlm_state`, `snapshots`, etc. move to `EvalState`:

```rust
// Before: env.rlm_state.insert(...)
// After:  state.rlm_state.insert(...)

// Before: env.take_snapshot() / env.restore_snapshot()
// After:  let snap = env.bindings.clone(); state.snapshots.push(snap);
//         env = Env { bindings: state.snapshots.remove(idx) };
```

**Step 3:** Update all test files — mechanical `&mut Env` → `&mut Env, &mut EvalState`.

**Step 4:** `cargo test` — all 235+ tests must pass.

**Step 5:** Commit

```bash
git commit -m "refactor: split Env into immutable bindings + EvalState

- Env.bindings: Vec -> im::HashMap (O(1) clone via structural sharing)
- Mutable counters (eval_count, rlm_state, tokens, etc.) -> EvalState
- apply_lambda: eliminate save/restore, use scoped env clone
- lisp_eval signature: (expr, &mut Env, &mut EvalState)
- Remove stacker dependency — still recursive, but Phase 2 will fix that
- All tests pass"
```

---

## Phase 2: Continuation Stack

### Task 6: Define the `Continuation` enum

**Objective:** Define all continuation variants that cover the ~27 recursive call sites.

**Files:**
- Create: `src/eval/continuation.rs`
- Modify: `src/eval/mod.rs` (add `mod continuation;`)

**Step 1:** Create the enum:

```rust
use crate::types::{Env, EvalState, LispVal};
use std::sync::Arc;

/// What to do after a sub-evaluation completes.
pub enum Cont {
    /// Top-level — return the result to the caller
    Done,

    // ── Function call ──
    /// Evaluate the head of a function call, then evaluate args
    EvalHead { args: Vec<LispVal>, env: Env },
    /// Evaluate args one-by-one, then dispatch
    EvalArg { remaining: Vec<LispVal>, done: Vec<LispVal>, head_val: LispVal, env: Env },

    // ── Special forms ──
    /// (if cond then else) — branch after cond eval
    IfBranch { then_branch: LispVal, else_branch: LispVal, env: Env },
    /// (cond (test1 expr1) (test2 expr2) ...) — try next arm
    CondArm { remaining_arms: Vec<Vec<LispVal>>, env: Env },
    /// (begin e1 e2 ... eN) — eval next expr in sequence
    BeginSeq { remaining: Vec<LispVal>, env: Env },
    /// (let ((x 1) (y 2)) body...) — eval next binding value
    LetBind { names: Vec<String>, remaining_pairs: Vec<(LispVal, LispVal)>, body: Vec<LispVal>, env: Env },
    /// (let* ...) same but sequential scope
    LetSeqBind { remaining_pairs: Vec<(String, LispVal)>, body: Vec<LispVal>, env: Env },
    /// (match expr (pattern body) ...) — after scrutinee eval
    MatchScrutinee { arms: Vec<(Vec<String>, LispVal)>, env: Env },
    /// (try expr (catch e body)) — evaluate expr, catch on error
    TryExpr { catch_var: String, catch_body: LispVal, env: Env },
    /// (define-macro ...) — after macro body eval
    MacroDefine { name: String, env: Env },
    /// Evaluate default value for optional arg
    DefaultArg { primary_expr: LispVal, env: Env },
    /// Module evaluation — import names after module body runs
    ModuleEval { prefix: String, exports: Vec<String>, module_env: Env },
    /// (set! name val) — after val eval
    SetVar { name: String, env: Env },
    /// (final expr) — after expr eval, store in rlm_state
    FinalExpr { env: Env },
    /// (assert cond msg) — after cond eval
    AssertCond { message: Option<LispVal>, env: Env },
    /// Quasiquote expansion then eval
    EvalExpanded { env: Env },
    /// for..in loop — evaluate body then check more items
    ForBody { var: String, rest: Vec<LispVal>, body: LispVal, env: Env },
    /// while loop — check condition after body eval
    WhileIter { cond: LispVal, body: LispVal, env: Env },
}
```

Note: `Cont` owns `Env` values — this is why immutable Env matters. Each continuation holds its own snapshot. With `im::HashMap`, cloning is O(1).

**Step 2:** `cargo check` — should compile (enum only, no usage yet).

### Task 7: Write the iterative eval loop

**Objective:** Replace `lisp_eval` + `lisp_eval_inner` with the continuation stack loop.

**Files:**
- Modify: `src/eval/mod.rs`

This is the big task. The approach:

1. `lisp_eval` becomes a `loop` that processes `eval_step()` results through the continuation stack
2. `eval_step()` is a pure function — takes an expr, returns either a value or (new_expr_to_eval + continuation to push)
3. `lisp_eval_inner` and the trampoline are deleted
4. All 27 recursive `lisp_eval()` calls become `push Cont + return next_expr`

```rust
pub fn lisp_eval(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    let mut stack: Vec<Cont> = vec![Cont::Done];
    let mut current = expr.clone();
    let mut current_env = env.clone();

    loop {
        // Budget check
        if state.eval_budget > 0 {
            state.eval_count += 1;
            if state.eval_count > state.eval_budget {
                return Err(format!("execution budget exceeded"));
            }
        }

        // Evaluate one step
        let step_result = eval_step(&current, &mut current_env, state)?;

        match step_result {
            Step::Done(value) => {
                // Pop continuations until we find work to do
                loop {
                    match stack.pop() {
                        Some(Cont::Done) => {
                            *env = current_env;  // propagate any top-level mutations
                            return Ok(value);
                        }
                        Some(Cont::EvalArg { mut remaining, mut done, head_val, env: arg_env }) => {
                            done.push(value);
                            if remaining.is_empty() {
                                // All args evaluated — dispatch the call
                                let result = dispatch_evaluated(&head_val, &done, &arg_env, state)?;
                                current = result.next_expr;
                                current_env = result.env;
                                // push any continuation from dispatch
                                if let Some(c) = result.cont {
                                    stack.push(c);
                                }
                                break;
                            } else {
                                current = remaining.remove(0);
                                current_env = arg_env;
                                stack.push(Cont::EvalArg { remaining, done, head_val, env: arg_env });
                                break;
                            }
                        }
                        Some(Cont::IfBranch { then_branch, .. }) if is_truthy(&value) => {
                            current = then_branch.clone();
                            break;
                        }
                        Some(Cont::IfBranch { else_branch, .. }) => {
                            current = else_branch.clone();
                            break;
                        }
                        Some(Cont::BeginSeq { remaining, env: seq_env }) => {
                            if remaining.is_empty() {
                                // value is the result, keep unwinding
                                continue;
                            } else {
                                current = remaining[0].clone();
                                current_env = seq_env.clone();
                                if remaining.len() > 1 {
                                    stack.push(Cont::BeginSeq {
                                        remaining: remaining[1..].to_vec(),
                                        env: seq_env,
                                    });
                                }
                                break;
                            }
                        }
                        // ... handle all other Cont variants
                        _ => todo!("handle continuation variant"),
                    }
                }
            }
            Step::EvalNext { expr, env, cont } => {
                if let Some(c) = cont {
                    stack.push(c);
                }
                current = expr;
                current_env = env;
            }
        }
    }
}
```

**Key insight:** The loop body is ~150 lines. Each `Cont` variant handler is 5-10 lines. Total ~300 lines for the loop + handlers, replacing the 540-line `lisp_eval_inner`.

### Task 8: Write `eval_step` — the non-recursive evaluator

**Objective:** Convert each branch of `lisp_eval_inner` into a non-recursive step.

**Files:**
- Modify: `src/eval/mod.rs`

The `eval_step` function handles atoms and special forms without recursion. For each special form:

- **Atoms** (Num, Str, Bool, Nil, Lambda, Macro, Map): return `Step::Done(value)`
- **Sym**: lookup in env → `Step::Done(value)` or error
- **quote**: `Step::Done(list[1])`
- **if**: push `Cont::IfBranch`, return `Step::EvalNext` with the condition
- **begin**: push `Cont::BeginSeq` with remaining, return first expr
- **define**: evaluate value (via Step), then bind — needs a `Cont::SetVar` or similar
- **lambda**: construct Lambda value → `Step::Done(Lambda{...})`
- **let**: push `Cont::LetBind`, return first binding value
- **List (function call)**: push `Cont::EvalHead`, return head expr

Every branch is a direct translation from the current `lisp_eval_inner` match arms. The logic is identical — just returns `Step` instead of making recursive calls.

### Task 9: Write `dispatch_evaluated` — call with already-evaluated args

**Objective:** After args are evaluated by the continuation stack, dispatch the call.

**Files:**
- Modify: `src/eval/mod.rs`

This replaces the current `dispatch_call` + `call_val` path. Args are already evaluated. Three cases:

1. Head is `Sym` → dispatch builtin or lookup + apply
2. Head is `Lambda` → push bindings, return body as next expr
3. Head is `List` → evaluate it first (shouldn't happen if EvalHead already ran, but fallback)

### Task 10: Update `apply_lambda` for continuation stack

**Objective:** `apply_lambda` no longer calls `lisp_eval`. It returns a body + env for the loop to process.

**Files:**
- Modify: `src/eval/mod.rs`

```rust
pub fn apply_lambda(...) -> Result<(LispVal, Env), String> {
    // Build local env with params + closures (same as Phase 1)
    // Return (body, local_env) — caller pushes as continuation
    Ok((body.clone(), local_env))
}
```

The caller (the main loop) then sets `current = body` and `current_env = local_env`.

### Task 11: Delete old code, clean up

**Objective:** Remove dead code from the refactor.

**Files:**
- Modify: `src/eval/mod.rs`

Delete:
- `lisp_eval_inner` function (540 lines)
- `'_trampoline` loop
- `stacker::maybe_grow` call
- All `push_saving!` macros
- All `env.truncate()` calls
- `env.len()` / `base_len` save-restore patterns

### Task 12: Full test suite + smoke test

**Objective:** Verify everything works.

**Step 1:** `cargo test` — all 235+ tests pass
**Step 2:** `cargo run --bin lisp-smoke` — Y1, Y2, Y3, foldl all work
**Step 3:** Test deep recursion — `(fact 1000)` should work (will still overflow i64 but shouldn't stack overflow)
**Step 4:** Commit

```bash
git commit -m "refactor: continuation stack — eliminate stack overflow

- Replace recursive lisp_eval with iterative loop + Vec<Cont>
- ~10 continuation variants cover all recursive patterns
- Native stack is always flat — one loop, one match
- Delete stacker dependency, trampoline, save/restore bookkeeping
- All tests pass, Y-combinators work at arbitrary depth"
```

---

## Summary

| Phase | Tasks | Net LOC change | Risk |
|-------|-------|---------------|------|
| Phase 1: Immutable Env | 1-5 | +80, -50 | Low — mechanical refactor |
| Phase 2: Continuation Stack | 6-12 | +400, -600 | Medium — large but mechanical |
| **Total** | **12** | **~480 net new, ~650 removed** | |

**Testing strategy:** After every task, `cargo test` must pass. Phase 1 is independently correct and shippable. Phase 2 builds on it.

**Key invariant:** `Cont` owns `Env` values. With `im::HashMap`, each `Env::clone()` is O(1) via structural sharing. No performance regression for normal code. Deep recursion gains unlimited depth.
