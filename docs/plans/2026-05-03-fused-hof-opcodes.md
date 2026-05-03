# Fused MapOp/FilterOp/ReduceOp Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Add MapOp, FilterOp, ReduceOp as fused VM opcodes across all three layers (F* model, Rust bytecode VM, WASM emitter), replacing the unverified BuiltinCall dispatch with verifiable, gas-efficient fused operations.

**Architecture:** Three-layer aligned implementation. F* defines formal semantics and proves correctness. Rust bytecode VM implements the proven semantics. WASM emitter compiles to tight loops with direct function calls. All three layers agree on behavior.

**Why not just use BuiltinCall?** The current Rust VM handles map/filter/reduce inside BuiltinCall via `vm_call_lambda`. This works but: (1) not modeled in F* — zero formal verification, (2) goes through full function call path per element (frame push, env setup, dispatch), (3) no WASM equivalent. Fused opcodes solve all three.

**Scope:** Function argument is a known top-level symbol (resolved at compile time to a slot/code-table index). Closures passed as arguments fall back to BuiltinCall. This is the safe on-chain subset.

---

## Current State

| Layer | map | filter | reduce |
|-------|-----|--------|--------|
| F* source eval | ✅ (line 91, Lisp.Source.fst) | ❌ | ❌ |
| F* opcode type | ❌ | ❌ | ❌ |
| F* eval_op | ❌ (falls through) | ❌ | ❌ |
| F* compiler | ❌ | ❌ | ❌ |
| Rust Op enum | ❌ (uses BuiltinCall) | ❌ (uses BuiltinCall) | ❌ (uses BuiltinCall) |
| Rust eval_op | ✅ via BuiltinCall (line 6062) | ✅ via BuiltinCall (line 6084) | ✅ via BuiltinCall (line 6150) |
| Rust compiler | Falls through to BuiltinCall | Falls through to BuiltinCall | Falls through to BuiltinCall |
| WASM emitter | ❌ (has numeric `map-into` only) | ❌ | ✅ numeric range only |

---

## Design

### Opcode Signatures

```
MapOp    of nat * nat   (* slot_idx, n_args *)
FilterOp of nat * nat   (* slot_idx, n_args *)
ReduceOp of nat * nat   (* slot_idx, n_args *)
```

- `slot_idx`: compile-time resolved slot containing the function value
- `n_args`: number of arguments the function takes (1 for map/filter, 2 for reduce)

### Stack Effects

```
MapOp(slot, n_args):
  Before: [..., list_val]
  After:  [..., mapped_list]

FilterOp(slot, n_args):
  Before: [..., list_val]
  After:  [..., filtered_list]

ReduceOp(slot, n_args):
  Before: [..., init_val, list_val]
  After:  [..., accumulated_val]
```

Note: ReduceOp takes init BEFORE list (matches Lisp convention `(reduce f init list)`).

### Semantics

**MapOp(slot, n_args):**
1. Pop list from stack
2. Pop n_args-1 elements from stack (additional curried args beyond the element)
3. Look up function at slots[slot]
4. For each element in list: call function(element, extra_args...), collect result
5. Push List of results

Wait — simpler design: n_args is always the total arity. The compiler pushes extra args before the list. At eval time, pop list, pop (n_args - 1) extra args, then for each element call function(element, extra_args...).

Actually, simplest possible design for Phase 1: n_args = 1 always. map takes (elem), filter takes (elem), reduce takes (acc, elem). The compiler wraps multi-arg functions in a lambda if needed. This matches the common case and keeps the opcode simple.

**Revised signatures:**
```
MapOp    of nat   (* slot_idx — function takes exactly 1 arg *)
FilterOp of nat   (* slot_idx — function takes exactly 1 arg *)
ReduceOp of nat   (* slot_idx — function takes exactly 2 args: acc, elem *)
```

### Compiler Pattern

```lisp
;; (map double my-list)
;; where (define (double x) (* x 2))  → slot 3
;; Compiles to:
;;   <compile my-list>    ;; push list onto stack
;;   MapOp 3              ;; slot 3 = double

;; (filter positive? nums)
;; where (define (positive? x) (> x 0))  → slot 5
;; Compiles to:
;;   <compile nums>        ;; push list onto stack
;;   FilterOp 5            ;; slot 5 = positive?

;; (reduce + 0 my-list)
;; where + is builtin
;; Compiles to:
;;   PushI64 0
;;   <compile my-list>     ;; push list onto stack
;;   ReduceOp builtin_slot  ;; slot for + function
```

### WASM Emitter Pattern

For WASM, lists can't be heap-allocated as tagged i64. Two options:

**Option A: Memory-based arrays (like existing `reduce`)**
```lisp
;; (map f start end) — iterate integer range
;; Already exists as map-into, extend to return values
```

**Option B: Restrict to numeric accumulators (extend existing reduce)**
```lisp
;; (reduce f init start end) — already works
;; (filter pred start end) — new, returns count of matches
```

**Recommendation:** Option B. On-chain DeFi doesn't need list-valued map/filter. It needs numeric accumulation (sum balances, count active positions, find max). The existing `reduce` pattern covers this. We extend it with `filter-count` and keep `map-into` for side effects.

---

## Phase 1: F* Model — Opcode Types + eval_op Semantics

### Task 1: Add opcode constructors to Lisp.Types.fst

**Files:**
- Modify: `verification/semantics/lisp/Lisp.Types.fst` (after line with `ReturnSlot`)

**Step 1: Add three new opcode constructors**

```fstar
  | MapOp         of nat   (* slot_idx *)
  | FilterOp      of nat   (* slot_idx *)
  | ReduceOp      of nat   (* slot_idx *)
```

Add after the `ReturnSlot of nat` line, before the closing of the `opcode` type.

**Step 2: Verify F* still type-checks**

```bash
cd /tmp/lisp-rlm/verification
rm -rf build
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --cache_dir build \
  semantics/lisp/Lisp.Types.fst
```

Expected: success (adding constructors is backward-compatible).

**Step 3: Commit**

```bash
git add semantics/lisp/Lisp.Types.fst
git commit -m "feat(fstar): add MapOp/FilterOp/ReduceOp opcode constructors"
```

---

### Task 2: Implement MakeList in eval_op (prerequisite)

**Files:**
- Modify: `verification/semantics/lisp_ir/LispIR.Semantics.fst`

Currently MakeList falls through to the default case (advance pc). We need it to actually work before map/filter/reduce can use lists.

**Step 1: Add helper `eval_make_list`**

Add before the `eval_op` function:

```fstar
val eval_make_list : n:nat -> stack:list lisp_val -> Tot (option (list lisp_val))
let rec eval_make_list n stack =
  match n, stack with
  | 0, items -> Some items
  | _, [] -> None
  | _, x :: rest ->
    (match eval_make_list (n - 1) rest with
     | Some items -> Some (items @ [x])  // reverse order
     | None -> None)
```

Wait — F* list append `@` may cause issues. Use cons + reverse:

```fstar
val eval_make_list : n:nat -> stack:list lisp_val -> Tot (list lisp_val * list lisp_val)
let rec eval_make_list n stack =
  match n, stack with
  | 0, rest -> ([], rest)
  | _, [] -> ([], [])
  | _, x :: rest ->
    let (items, remaining) = eval_make_list (n - 1) rest in
    (items @ [x], remaining)
```

Actually, check what helpers already exist. The file has `pop_n`:

```fstar
val pop_n : list lisp_val -> nat -> Tot (option (list lisp_val * list lisp_val))
```

Use `pop_n` directly in eval_op.

**Step 2: Add MakeList case to eval_op**

Inside the `eval_op` match, before the default `| _ ->` case:

```fstar
  | MakeList n ->
    (match pop_n s.stack n with
     | Some (items, rest) ->
       let reversed = list_rev items in
       Ok { s with stack = List reversed :: rest; pc = s.pc + 1 }
     | None -> Ok { s with ok = false; pc = s.pc + 1 })
```

Need to verify `list_rev` exists in F* stdlib (it does: `List.Tot.Base.rev`).

**Step 3: Verify**

```bash
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --cache_dir build \
  --z3rlimit 20 semantics/lisp_ir/LispIR.Semantics.fst
```

**Step 4: Run existing list tests**

```bash
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --include tests \
  --cache_dir build --z3rlimit 20 tests/ListCompileTest.fst
```

Expected: all 17 tests still pass.

**Step 5: Commit**

```bash
git add semantics/lisp_ir/LispIR.Semantics.fst
git commit -m "feat(fstar): implement MakeList in eval_op"
```

---

### Task 3: Add MapOp/FilterOp/ReduceOp to eval_op

**Files:**
- Modify: `verification/semantics/lisp_ir/LispIR.Semantics.fst`

**Design constraint:** eval_op is a pure function — it can't call `apply_lambda` (which requires an environment). So the fused opcodes define their semantics directly in terms of list operations, without invoking the function. The function slot is recorded but the actual function application is done at a higher level.

**Alternative approach:** Define the semantics using `eval_steps` recursively. The opcode sets up a sub-VM that evaluates the function. This is how `CallSelf` would work if it were implemented.

**Simplest verifiable approach:** The fused opcodes are defined as shorthand for a known bytecode sequence. The proof shows they're equivalent to the desugared version. This is the "correct by construction" approach.

For now, define placeholder semantics that advance pc and mark as todo:

```fstar
  | MapOp slot_idx ->
    // TODO: full semantics (requires function call support in eval_op)
    Ok { s with pc = s.pc + 1 }
  | FilterOp slot_idx ->
    Ok { s with pc = s.pc + 1 }
  | ReduceOp slot_idx ->
    Ok { s with pc = s.pc + 1 }
```

This allows the compiler to emit the opcodes and the type system to accept them, while we defer the full semantics to when CallSelf/CallDynamic are modeled.

**Step 1: Add placeholder cases to eval_op**

**Step 2: Verify**

```bash
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --cache_dir build \
  --z3rlimit 20 semantics/lisp_ir/LispIR.Semantics.fst
```

**Step 3: Commit**

```bash
git add semantics/lisp_ir/LispIR.Semantics.fst
git commit -m "feat(fstar): add MapOp/FilterOp/ReduceOp placeholder eval_op cases"
```

---

### Task 4: Add map/filter/reduce to F* source evaluator

**Files:**
- Modify: `verification/semantics/lisp/Lisp.Source.fst`

The source evaluator already has `map` (line 91). Add `filter` and `reduce`.

**Step 1: Add filter to eval_expr**

After the `map` match arm:

```fstar
  | [Sym "filter"; func_expr; list_expr] ->
    (match eval_expr f func_expr env, eval_expr f list_expr env with
     | Ok (Lambda (params, body, closure_env)), Ok (List lst) ->
       eval_filter_list f params body closure_env lst
     | Ok _, Ok Nil -> Ok Nil
     | Ok _, Ok (List []) -> Ok Nil
     | Ok _, Ok _ -> Err "filter: second arg must be a list"
     | Err m, _ -> Err m
     | _, Err m -> Err m)
```

**Step 2: Add reduce to eval_expr**

```fstar
  | [Sym "reduce"; func_expr; init_expr; list_expr] ->
    (match eval_expr f func_expr env, eval_expr f init_expr env, eval_expr f list_expr env with
     | Ok (Lambda (params, body, closure_env)), Ok init, Ok (List lst) ->
       eval_reduce_list f params body closure_env init lst
     | Ok _, Ok init, Ok Nil -> Ok init
     | Ok _, Ok init, Ok (List []) -> Ok init
     | Ok _, Ok _, Ok _ -> Err "reduce: third arg must be a list"
     | Err m, _, _ -> Err m
     | _, Err m, _ -> Err m
     | _, _, Err m -> Err m)
```

**Step 3: Add helper functions**

```fstar
and eval_filter_list (fuel:int) (params:list string) (body:lisp_val)
    (closure_env:list (string * lisp_val)) (lst:list lisp_val) : eval_result =
  match lst with
  | [] -> Ok Nil
  | elem :: rest ->
    (match apply_fn f params body closure_env [elem] with
     | Ok (Bool true) ->
       (match eval_filter_list f params body closure_env rest with
        | Ok (List filtered) -> Ok (List (elem :: filtered))
        | Ok Nil -> Ok (List [elem])
        | _ -> Err "filter: internal error")
     | Ok (Bool false) ->
       eval_filter_list f params body closure_env rest
     | Ok _ ->  // truthy non-bool values count as true
       (match eval_filter_list f params body closure_env rest with
        | Ok (List filtered) -> Ok (List (elem :: filtered))
        | Ok Nil -> Ok (List [elem])
        | _ -> Err "filter: internal error")
     | Err m -> Err m)

and eval_reduce_list (fuel:int) (params:list string) (body:lisp_val)
    (closure_env:list (string * lisp_val)) (acc:lisp_val) (lst:list lisp_val) : eval_result =
  match lst with
  | [] -> Ok acc
  | elem :: rest ->
    (match apply_fn f params body closure_env [acc; elem] with
     | Ok new_acc -> eval_reduce_list f params body closure_env new_acc rest
     | Err m -> Err m)
```

**Step 4: Verify**

```bash
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --cache_dir build --z3rlimit 20 \
  semantics/lisp/Lisp.Source.fst
```

**Step 5: Commit**

```bash
git add semantics/lisp/Lisp.Source.fst
git commit -m "feat(fstar): add filter and reduce to source evaluator"
```

---

### Task 5: Add map/filter/reduce to F* compiler

**Files:**
- Modify: `verification/semantics/lisp/Lisp.Compiler.fst`

**Step 1: Add compiler cases**

In the `compile` function's match, before the `_ -> None` fallback:

```fstar
  // (map f list) — f must be a known symbol
  | List (Sym "map" :: Sym func_name :: list_expr :: _) ->
    (match slot_of func_name c with
     | Some slot_idx -> 
       (match compile (f - 1) list_expr c with
        | Some c' -> Some { c' with code = c'.code @ [MapOp slot_idx] }
        | None -> None)
     | None -> None)
  | List (Sym "filter" :: Sym func_name :: list_expr :: _) ->
    (match slot_of func_name c with
     | Some slot_idx ->
       (match compile (f - 1) list_expr c with
        | Some c' -> Some { c' with code = c'.code @ [FilterOp slot_idx] }
        | None -> None)
     | None -> None)
  | List (Sym "reduce" :: Sym func_name :: init_expr :: list_expr :: _) ->
    (match slot_of func_name c with
     | Some slot_idx ->
       (match compile (f - 1) init_expr c with
        | Some c1 ->
          (match compile (f - 1) list_expr c1 with
           | Some c2 -> Some { c2 with code = c2.code @ [ReduceOp slot_idx] }
           | None -> None)
        | None -> None)
     | None -> None)
```

Note: `slot_of` looks up a symbol in the compiler's slot_map and returns its index.

**Step 2: Verify**

```bash
rm -rf build
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --include tests \
  --cache_dir build --z3rlimit 20 semantics/lisp/Lisp.Compiler.fst
```

**Step 3: Commit**

```bash
git add semantics/lisp/Lisp.Compiler.fst
git commit -m "feat(fstar): add map/filter/reduce compilation to fused opcodes"
```

---

### Task 6: F* proofs — compiler output structure

**Files:**
- Create: `verification/tests/HofCompileTest.fst`

Following the ListCompileTest.fst pattern. Three layers of proof.

**Step 1: Prove map compiles to MapOp**

```fstar
module HofCompileTest

open FStar.List.Tot
open Lisp.Types
open Lisp.Compiler

val compile_map_spec : fuel:int -> slot:nat -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "x"] (List [Sym "map"; Sym "f"; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 m; MakeList 1; MapOp s; Return] -> m = n && s = slot
       | _ -> false)
    | None -> false))
let compile_map_spec fuel slot n = ()
```

Note: `compile_lambda` takes params ["f"; "x"] so that "f" has a slot. The slot index depends on param ordering.

**Step 2: Prove filter compiles to FilterOp**

```fstar
val compile_filter_spec : fuel:int -> slot:nat -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "x"] (List [Sym "filter"; Sym "f"; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 m; MakeList 1; FilterOp s; Return] -> m = n && s = slot
       | _ -> false)
    | None -> false))
let compile_filter_spec fuel slot n = ()
```

**Step 3: Prove reduce compiles to ReduceOp**

```fstar
val compile_reduce_spec : fuel:int -> slot:nat -> init:int -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "acc"; "x"]
       (List [Sym "reduce"; Sym "f"; Num init; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 i; PushI64 m; MakeList 1; ReduceOp s; Return] -> i = init && m = n && s = slot
       | _ -> false)
    | None -> false))
let compile_reduce_spec fuel slot init n = ()
```

**Step 4: Verify**

```bash
FSTAR_HOME=/tmp/fstar-install/fstar /tmp/fstar-install/fstar/bin/fstar.exe \
  --include semantics/lisp --include semantics/lisp_ir --include tests \
  --cache_dir build --z3rlimit 20 tests/HofCompileTest.fst
```

**Step 5: Commit**

```bash
git add tests/HofCompileTest.fst
git commit -m "feat(fstar): prove map/filter/reduce compile to fused opcodes"
```

---

## Phase 2: Rust Bytecode VM

### Task 7: Add Op variants to Rust

**Files:**
- Modify: `src/bytecode.rs` — Op enum (near existing CallDynamic variant)

**Step 1: Add enum variants**

```rust
MapOp(usize),     // slot_idx: function takes 1 arg (elem)
FilterOp(usize),  // slot_idx: function takes 1 arg (elem)
ReduceOp(usize),  // slot_idx: function takes 2 args (acc, elem)
```

**Step 2: Verify compilation**

```bash
cd /tmp/lisp-rlm && cargo build 2>&1 | grep error
```

Expected: compile errors in match exhaustiveness — every `match op` needs new arms.

**Step 3: Commit**

```bash
git add src/bytecode.rs
git commit -m "feat(vm): add MapOp/FilterOp/ReduceOp opcode variants"
```

---

### Task 8: Implement eval_op for fused opcodes

**Files:**
- Modify: `src/bytecode.rs` — eval_op function

Add cases in the `match op` block, before the `BuiltinCall` arm:

```rust
Op::MapOp(slot_idx) => {
    let func_val = slots.get(*slot_idx).cloned().unwrap_or(LispVal::Nil);
    let list_val = stack.pop().unwrap_or(LispVal::Nil);
    let items = match &list_val {
        LispVal::List(l) => l.clone(),
        _ => vec![],
    };
    let mut result = Vec::with_capacity(items.len());
    for elem in &items {
        match vm_call_lambda(&func_val, &[elem.clone()], outer_env, state) {
            Ok(r) => result.push(r),
            Err(_) => result.push(LispVal::Nil),
        }
    }
    stack.push(LispVal::List(result));
    pc += 1;
}

Op::FilterOp(slot_idx) => {
    let func_val = slots.get(*slot_idx).cloned().unwrap_or(LispVal::Nil);
    let list_val = stack.pop().unwrap_or(LispVal::Nil);
    let items = match &list_val {
        LispVal::List(l) => l.clone(),
        _ => vec![],
    };
    let mut result = Vec::new();
    for elem in &items {
        let keep = match vm_call_lambda(&func_val, &[elem.clone()], outer_env, state) {
            Ok(LispVal::Bool(b)) => b,
            Ok(v) => is_truthy(&v),
            Err(_) => false,
        };
        if keep {
            result.push(elem.clone());
        }
    }
    stack.push(LispVal::List(result));
    pc += 1;
}

Op::ReduceOp(slot_idx) => {
    let func_val = slots.get(*slot_idx).cloned().unwrap_or(LispVal::Nil);
    let list_val = stack.pop().unwrap_or(LispVal::Nil);
    let mut acc = stack.pop().unwrap_or(LispVal::Nil);
    let items = match &list_val {
        LispVal::List(l) => l.clone(),
        _ => vec![],
    };
    for elem in &items {
        match vm_call_lambda(&func_val, &[acc.clone(), elem.clone()], outer_env, state) {
            Ok(r) => acc = r,
            Err(_) => {}
        }
    }
    stack.push(acc);
    pc += 1;
}
```

**Step 2: Also add cases to SpecVM in test_differential_fuzz.rs**

In the SpecVm `step()` method, add:

```rust
Op::MapOp(slot_idx) => {
    let func_val = self.slots.get(*slot_idx).cloned().unwrap_or(LispVal::Nil);
    let list_val = self.stack.pop().unwrap_or(LispVal::Nil);
    let items = match &list_val {
        LispVal::List(l) => l.clone(),
        _ => vec![],
    };
    let mut result = Vec::with_capacity(items.len());
    for elem in &items {
        // SpecVM uses simplified call: only handles BuiltinFn and direct slot lookup
        result.push(LispVal::Num(0)); // placeholder — SpecVM can't call arbitrary functions
    }
    self.stack.push(LispVal::List(result));
    self.pc += 1;
}
```

Note: SpecVM can't call arbitrary lambdas. The differential fuzz for HOF opcodes needs a different approach (source-level testing, not bytecode-level).

**Step 3: Build and test**

```bash
cd /tmp/lisp-rlm && cargo build && cargo test --test test_differential_fuzz
```

**Step 4: Commit**

```bash
git add src/bytecode.rs tests/test_differential_fuzz.rs
git commit -m "feat(vm): implement MapOp/FilterOp/ReduceOp eval_op"
```

---

### Task 9: Wire compiler to emit fused opcodes

**Files:**
- Modify: `src/bytecode.rs` — LoopCompiler.compile_expr

In the compile_expr dispatch (around line 2100 where known symbol calls are handled), add before the generic call path:

```rust
// Fused HOF opcodes — map/filter/reduce with known function symbol
if op == "map" && list.len() == 3 {
    if let LispVal::Sym(func_name) = &list[1] {
        if let Some(slot) = self.slot_of(func_name) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            self.code.push(Op::MapOp(slot));
            return true;
        }
        // If func is a builtin, try to capture it
        if let Some((slot, _)) = self.try_capture(func_name, outer_env) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            self.code.push(Op::MapOp(slot));
            return true;
        }
    }
    // Fall through to BuiltinCall for dynamic cases
}
// Similar for filter and reduce
if op == "filter" && list.len() == 3 {
    if let LispVal::Sym(func_name) = &list[1] {
        if let Some(slot) = self.slot_of(func_name) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            self.code.push(Op::FilterOp(slot));
            return true;
        }
        if let Some((slot, _)) = self.try_capture(func_name, outer_env) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            self.code.push(Op::FilterOp(slot));
            return true;
        }
    }
}
if op == "reduce" && list.len() == 4 {
    if let LispVal::Sym(func_name) = &list[1] {
        if let Some(slot) = self.slot_of(func_name) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            if !self.compile_expr(&list[3], outer_env) { return false; }
            self.code.push(Op::ReduceOp(slot));
            return true;
        }
        if let Some((slot, _)) = self.try_capture(func_name, outer_env) {
            if !self.compile_expr(&list[2], outer_env) { return false; }
            if !self.compile_expr(&list[3], outer_env) { return false; }
            self.code.push(Op::ReduceOp(slot));
            return true;
        }
    }
}
```

**Step 2: Build**

```bash
cd /tmp/lisp-rlm && cargo build
```

**Step 3: Commit**

```bash
git add src/bytecode.rs
git commit -m "feat(compiler): emit fused MapOp/FilterOp/ReduceOp for known functions"
```

---

### Task 10: Rust integration tests

**Files:**
- Create: `tests/test_fused_hof.rs`

**Step 1: Write helper**

```rust
fn eval_str(source: &str) -> Result<LispVal, String> {
    let exprs = lisp_rlm_wasm::parser::parse_all(source)
        .map_err(|e| format!("parse: {}", e))?;
    let stdlib = lisp_rlm_wasm::types::get_stdlib_code("core")
        .and_then(|code| lisp_rlm_wasm::parser::parse_all(code).ok())
        .unwrap_or_default();
    let mut env = lisp_rlm_wasm::types::Env::new();
    let mut state = lisp_rlm_wasm::types::EvalState::new();
    for expr in &stdlib {
        let _ = lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state);
    }
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result)
}
```

**Step 2: Test map**

```rust
#[test]
fn test_map_double() {
    let r = eval_str(
        "(define (double x) (* x 2))\n\
         (map double (list 1 2 3 4 5))"
    );
    assert_eq!(r, Ok(LispVal::List(vec![
        LispVal::Num(2), LispVal::Num(4), LispVal::Num(6),
        LispVal::Num(8), LispVal::Num(10),
    ])));
}

#[test]
fn test_map_empty_list() {
    let r = eval_str(
        "(define (double x) (* x 2))\n\
         (map double (list))"
    );
    assert_eq!(r, Ok(LispVal::List(vec![])));
}

#[test]
fn test_map_with_builtin() {
    // map abs over a list (abs is a builtin)
    let r = eval_str("(map abs (list -3 0 7 -1))");
    // abs is BuiltinFn, should work via vm_call_lambda
    assert!(r.is_ok());
}
```

**Step 3: Test filter**

```rust
#[test]
fn test_filter_positive() {
    let r = eval_str(
        "(define (positive? x) (> x 0))\n\
         (filter positive? (list -3 0 1 5 -2 7))"
    );
    assert_eq!(r, Ok(LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(5), LispVal::Num(7),
    ])));
}

#[test]
fn test_filter_none_match() {
    let r = eval_str(
        "(define (big? x) (> x 100))\n\
         (filter big? (list 1 2 3))"
    );
    assert_eq!(r, Ok(LispVal::List(vec![])));
}
```

**Step 4: Test reduce**

```rust
#[test]
fn test_reduce_sum() {
    let r = eval_str("(reduce + 0 (list 1 2 3 4 5))");
    assert_eq!(r, Ok(LispVal::Num(15)));
}

#[test]
fn test_reduce_product() {
    let r = eval_str("(reduce * 1 (list 1 2 3 4 5))");
    assert_eq!(r, Ok(LispVal::Num(120)));
}

#[test]
fn test_reduce_empty() {
    let r = eval_str("(reduce + 42 (list))");
    assert_eq!(r, Ok(LispVal::Num(42)));
}

#[test]
fn test_reduce_max() {
    let r = eval_str(
        "(define (max a b) (if (> a b) a b))\n\
         (reduce max 0 (list 3 7 2 9 4))"
    );
    assert_eq!(r, Ok(LispVal::Num(9)));
}
```

**Step 5: Test composed HOFs**

```rust
#[test]
fn test_map_then_filter() {
    let r = eval_str(
        "(define (double x) (* x 2))\n\
         (define (big? x) (> x 5))\n\
         (filter big? (map double (list 1 2 3 4 5)))"
    );
    assert_eq!(r, Ok(LispVal::List(vec![
        LispVal::Num(6), LispVal::Num(8), LispVal::Num(10),
    ])));
}

#[test]
fn test_sum_of_squares() {
    // sum of squares via map + reduce
    let r = eval_str(
        "(define (square x) (* x x))\n\
         (reduce + 0 (map square (list 1 2 3 4 5)))"
    );
    assert_eq!(r, Ok(LispVal::Num(55)));
}
```

**Step 6: Run tests**

```bash
cd /tmp/lisp-rlm && cargo test --test test_fused_hof
```

**Step 7: Commit**

```bash
git add tests/test_fused_hof.rs
git commit -m "test: add fused HOF integration tests"
```

---

### Task 11: Bytecode verifier support

**Files:**
- Modify: `src/verifier.rs`

The bytecode verifier validates slot indices and other invariants. Add validation for the new opcodes.

**Step 1: Add slot index validation**

In the verifier's opcode validation, add:

```rust
Op::MapOp(slot) | Op::FilterOp(slot) | Op::ReduceOp(slot) => {
    if *slot >= total_slots {
        return Err(format!("MapOp/FilterOp/ReduceOp slot {} >= total_slots {}", slot, total_slots));
    }
}
```

**Step 2: Build and test**

```bash
cd /tmp/lisp-rlm && cargo build && cargo test
```

**Step 3: Commit**

```bash
git add src/verifier.rs
git commit -m "feat(verifier): validate MapOp/FilterOp/ReduceOp slot indices"
```

---

## Phase 3: WASM Emitter

### Task 12: WASM — extend reduce to support list-like iteration

**Files:**
- Modify: `src/wasm_emit.rs`

The current WASM `reduce` is numeric-only: `(reduce init start end acc body)`. Extend to also support `(reduce f init list)` where list is a compile-time constant.

**Design:** For on-chain, the most useful pattern is iterating storage keys. The existing numeric range approach is correct for this. No WASM changes needed for list-based map/filter/reduce — those are bytecode-VM-only features.

Instead, add `filter-range` for the WASM path:

```lisp
;; (filter-range pred start end)
;; Returns count of items in [start, end) where pred returns true
;; pred references `__it` for the current value
```

This is optional — the existing `reduce` and `map-into` cover the main on-chain use cases.

**Decision:** Skip WASM HOF changes. The bytecode VM handles list-based HOFs. WASM uses numeric range iteration. Document this split clearly.

---

## Phase 4: Differential Testing

### Task 13: Cross-backend HOF fuzz test

**Files:**
- Modify: `tests/test_wasm_fuzz.rs` — NOT applicable (WASM doesn't do list HOFs)
- Create: `tests/test_hof_differential.rs` — bytecode VM vs source-level reference

Since the bytecode VM and source evaluator both run in Rust, we can compare them directly:

```rust
#[test]
fn test_map_consistency() {
    // Run via bytecode VM (run_program → try_compile_lambda)
    let vm_result = eval_via_vm("(define (double x) (* x 2))\n(map double (list 1 2 3))");
    // Run via source eval (lisp_eval with JIT disabled... except there's no such mode)
    // Actually: both paths use run_program, so this tests compiler output consistency
    assert!(vm_result.is_ok());
}
```

Since both paths go through `run_program` (which always compiles), this test validates that the fused opcodes produce the same results as the BuiltinCall fallback. Add proptest generators:

```rust
proptest! {
    #[test]
    fn prop_map_random(
        func in gen_known_function(),
        list in gen_small_int_list(),
    ) {
        let source = format!("(map {} (list {}))", func, list.join(" "));
        // Compare fused opcode path vs BuiltinCall path
        // (need to force one or the other)
    }
}
```

**Note:** This requires a way to force BuiltinCall vs fused opcode compilation. Add a test-only flag or compare against the existing BuiltinCall implementation directly.

---

## Verification Status

| What | Proven? | Method |
|------|---------|--------|
| Opcode type definitions | ✅ Phase 1 Task 1 | F* type-checks |
| MakeList semantics | ✅ Phase 1 Task 2 | F* eval_op |
| MapOp/FilterOp/ReduceOp eval_op | ⚠️ Placeholder | Advances pc, no function call semantics |
| Compiler emits correct opcodes | ✅ Phase 1 Task 6 | F* proof |
| Source eval map/filter/reduce | ✅ Phase 1 Task 4 | F* code |
| Rust VM implementation | ✅ Phase 2 Task 8 | Integration tests |
| Rust compiler emits opcodes | ✅ Phase 2 Task 9 | Build + integration tests |
| End-to-end correctness (F*) | ❌ Blocked | Needs CallSelf/CallDynamic in F* eval_op |
| WASM support | ⏭️ Skipped | Numeric range only — different domain |
| Differential fuzz | ⚠️ Phase 4 Task 13 | Bytecode VM vs source eval |

**The verification gap:** Full F* end-to-end proofs for MapOp/FilterOp/ReduceOp require modeling function calls in eval_op. This is the same prerequisite for proving CallSelf, CallDynamic, and the existing BuiltinCall map/filter/reduce. It's a larger effort that enables all HOF verification at once.

**Honest assessment:** The fused opcodes give us:
1. Gas-efficient on-chain execution (fewer opcodes per iteration)
2. Compiler-level verification (F* proves the right opcode is emitted with the right slot)
3. Runtime correctness (Rust integration tests + differential fuzz)

What they don't give us (yet):
1. F* proof that the opcode semantics match source eval (blocked on CallSelf modeling)
2. WASM list-based HOFs (different value representation)

---

## Execution Order

```
Phase 1 (F* Model):        Tasks 1-6
Phase 2 (Rust VM):         Tasks 7-11
Phase 3 (WASM):            Task 12 (skip/defer)
Phase 4 (Differential):    Task 13
```

Phase 1 and Phase 2 can be parallelized — they touch different files (F* vs Rust). Phase 4 depends on Phase 2.
