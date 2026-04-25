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

- 268 tests, 0 failed, 2 ignored (doc tests), 0 warnings from source
- All fib/fibonacci tests pass (fib(15) = 610) — no stack overflow
- All budget tests pass — infinite loops caught by budget, not stack overflow

---

## Auto-Parallelism in RLM Fractal Decomposition

When the RLM fractal loop decomposes a task into 2+ subtasks (Phase 4), subtasks now run **in parallel** via `std::thread::spawn`.

**How it works:**

- Each branch gets its own `Env` fork (O(1) via `im::HashMap` structural sharing)
- Each branch gets its own `EvalState` clone with **shared** `Arc<AtomicU64>` counters for `tokens_used` and `llm_calls`
- Each branch gets its own `LlmProvider` clone (shares `SHARED_CLIENT` HTTP connection pool)
- Threads are joined in order; results collected for Phase 5 (synthesize)
- Single subtask (len=1) runs sequentially to avoid thread overhead

**Shared budget semantics:**

- `tokens_used` and `llm_calls` are `Arc<AtomicU64>` — all branches share the same counters
- If branch A burns 80% of the token budget, branch B gets the remaining 20%
- Budget checks (`state.tokens_used.load() >= token_budget`) are automatically cross-branch
- No manual save/restore needed — the atomics accumulate correctly

**Why `std::thread::spawn` (not rayon/tokio):**

- The evaluator is synchronous; each branch calls `SHARED_RUNTIME.block_on()` for LLM HTTP
- `tokio::spawn` would create nested `block_on` panics
- `std::thread::spawn` gives each branch its own OS thread — `block_on` works from any thread
- No new dependency required

**Enabled automatically — no model changes needed:**

The model writes `(rlm "solve this")`, the fractal decomposes, subtasks run in parallel, synthesize merges results. Zero new Lisp syntax. The concurrency is an implementation detail of the evaluator.

### `Arc<AtomicU64>` Migration

`EvalState.tokens_used` and `llm_calls` changed from `usize` to `Arc<AtomicU64>`:

- `new()` → `Arc::new(AtomicU64::new(0))`
- `Clone` → `Arc::clone(&self.tokens_used)` (shared reference, not copied value)
- Reads → `.load(Ordering::Relaxed) as usize`
- Writes → `.fetch_add(n as u64, Ordering::Relaxed)`
- `merge_rlm_state()` simplified — no more token/call save/restore

---

## Persistent Data Structures for Lisp

### Persistent Maps (`LispVal::Map`)

Changed from `BTreeMap<String, LispVal>` to `im::HashMap<String, LispVal>`:

- `dict/set` and `dict/remove` now return a new version via structural sharing — O(1) instead of O(n) clone
- All dict operations (`dict`, `dict/get`, `dict/set`, `dict/remove`, `dict/merge`, `dict/has?`, `dict/keys`, `dict/vals`) unchanged at the Lisp level
- `sort` builtin now handles strings (lexicographic) in addition to numbers
- JSON interop (`json_to_lisp` / `lisp_to_json`) transparently uses `im::HashMap`

### Speculative Evaluation (`fork`)

New special form: `(fork expr)`

- Evaluates `expr` in an isolated environment fork — O(1) via `im::HashMap` structural sharing
- Returns the result without affecting the parent's bindings
- Shares `Arc<AtomicU64>` token/call counters — budget is consumed but env is isolated
- Gets its own `LlmProvider` clone (shared HTTP client pool)

Usage:
```lisp
;; Try a risky computation without polluting the env
(define result (fork (begin (define x 42) (compute x))))
;; x is NOT defined here — fork's env was isolated

;; Compare two approaches speculatively
(define a (fork (approach-1 data)))
(define b (fork (approach-2 data)))
(if (> (score a) (score b)) a b)
```

This is the key primitive for self-harnessing RLM: the model can evaluate generated code speculatively, keep the result, and discard the side effects.

## Runtime Type System

Three layers, all checked at runtime — no compile-time phase.

### Layer 1: Predicate-style (`check`, `type-of`, `matches?`)

```lisp
(type-of 42)              ;; → :int
(check 42 :int)           ;; → 42
(check "hello" :int)      ;; → Error: expected :int, got :str
(matches? nil (:or :int :nil))  ;; → true
```

Type language:
- Primitives: `:nil :bool :int :float :num :str :sym :list :map :fn :any`
- Parameterized: `(:list :int)`, `(:map :str :int)`, `(:tuple :int :str :bool)`
- Union: `(:or :int :nil)`
- `:num` = int or float

### Layer 2: Contracts (`contract` special form)

```lisp
(define add1
  (contract (x :int -> :int)
    (+ x 1)))
(add1 5)         ;; → 6
(add1 "hello")   ;; → Error: contract violation: param 1 expected :int, got :str
```

Two signature formats:
- Flat: `(x :int y :str -> :ret)`
- Grouped: `((x :int) (y :str) -> :ret)`

Checks param types on entry, return type on exit. Resolves TailCalls before return check.

### Layer 3: Schemas (`defschema`, `validate`)

```lisp
(defschema :user "name" :str "age" :int "tags" (:list :str) :strict)
(validate (dict "name" "Jean" "age" 30 "tags" (list "dev")) :user)  ;; → ok
(validate (dict "name" 42) :user)  ;; → Error: missing field 'age'
(validate (dict "name" "J" "age" 30 "extra" "x") :user)  ;; → Error: unexpected field 'extra' (strict)
```

Strict mode rejects unknown keys. Compound types work in schemas.

### Design decisions

- Keywords (symbols starting with `:`) are self-evaluating — `:int` evaluates to itself
- Lists starting with a keyword are self-evaluating — `(:list :int)` passes through unchanged
- `parse_type` accepts both `Sym` and `Str` (contracts store types as strings internally)
- Contract return check force-evaluates TailCalls to check actual values, not unevaluated expressions

## Architecture: Persistent Data Structures

`Env` wraps `im::HashMap` — a Hash Array Mapped Trie (HAMT). This is a persistent (immutable) data structure: every mutation creates a new version that shares unchanged nodes with the old one via structural sharing.

**What this means concretely:**

- `env.clone()` / `env.snapshot()` is O(1) — just a pointer bump, not a deep copy
- Old versions remain valid and usable as long as someone holds a reference
- Only the path from root to the changed key is allocated per edit (2-3 nodes, regardless of map size)
- Lookup is O(log32 n) — effectively constant for any realistic env size

**What this enables for the evaluator:**

1. **Time-travel debugging** — Every eval step can snapshot env in O(1). Full timeline of environment state for auditing agent execution without memory blowup.

2. **Speculative evaluation** — Fork env, try risky code (macro expansion, generated code), commit or discard. The fork is O(1) because the parent version never changed. No rollback machinery needed.

3. **Safe recursive branching** — `(+ (fib (- n 1)) (fib (- n 2)))` evaluates two recursive calls that share the parent's bindings via structural sharing, not copying. Each branch mutates independently without clobbering the other.

**What it does NOT enable (yet):**

Concurrency. The eval loop is synchronous and single-threaded. `im` gives the data structure for parallel evaluation, but the evaluator doesn't use threads. The persistent map is currently doing defensive work (preventing clobber) rather than enabling parallelism.

**Design origin:** Okasaki's "Purely Functional Data Structures" (1999). Clojure made it mainstream. `im` is Rust's implementation.

---

## Future (not planned)

### Full Continuation Stack for deeper patterns

If mutually recursive functions not in tail position ever overflow, the architecture supports it. The `Cont` enum already has ~15 variants. All that's needed is converting more `lisp_eval` recursive call sites into `push Cont + return next_expr`.
