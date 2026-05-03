# Vec Primitive Spec

## Motivation

Lists in lisp-rlm use structural induction for every property in F* — length, indexing, map/filter/reduce preservation. This is expensive and verbose. Native arrays backed by F*'s `FStar.Seq.seq` (which is axiomatized over `v:seq a`) give us free SMT array theory:

- `Seq.length` — O(1), no induction
- `Seq.index` / `Seq.update` — read-after-write, write-doesn't-corrupt axioms
- No structural induction needed for length/index properties

The `noeq` wall on `lisp_val` doesn't change — but we only prove things about indices and lengths (nat, eqtype), never about element equality.

## Design: Native Array, Immutable Semantics

Not Clojure persistent vectors (bit-partitioned trie — O(log32 n) but verification nightmare).
Not Rust `Vec` mutation (aliasing bugs in proofs).
Pure immutable array: `vec-assoc` returns a fresh copy. O(n) mutation, but vectors won't be huge on-chain.

## 1. Rust: LispVal Variant

```rust
// In src/types.rs, add after List variant:
/// Immutable native array. Backed by Vec<Val> at runtime.
/// Immutable semantics — vec-assoc/vec-update return fresh copies.
Vec(Vec<LispVal>),
```

## 2. Rust: Opcodes

```rust
// In src/bytecode.rs, Op enum — add after MakeList:

/// Pop n values, construct a Vec, push it
MakeVec(usize),
/// Pop index, pop vec, push vec[index] (or Nil if out of bounds)
VecNth,
/// Pop value, pop index, pop vec, push vec with value at index (fresh copy)
VecAssoc,
/// Pop vec, push its length as Num
VecLen,
/// Pop value, pop vec, push vec with value appended (fresh copy)
VecConj,
/// Pop vec, pop value, push true if value is in vec (structural equality)
VecContains,
/// Pop sub-vec, pop start-index, pop vec, push vec[start:] (fresh copy)
VecSlice,
```

**Bytecode VM implementation** (both `run()` and the check-based `run_checked()`):

```rust
Op::MakeVec(n) => {
    let mut items = Vec::with_capacity(*n);
    for _ in 0..*n {
        items.push(stack.pop().unwrap_or(LispVal::Nil));
    }
    items.reverse();
    stack.push(LispVal::Vec(items));
    pc += 1;
}

Op::VecNth => {
    let idx = stack.pop().unwrap_or(LispVal::Nil);
    let vec = stack.pop().unwrap_or(LispVal::Nil);
    match (&idx, &vec) {
        (Num(i), LispVal::Vec(v)) if *i >= 0 && (*i as usize) < v.len() => {
            stack.push(v[*i as usize].clone());
        }
        _ => stack.push(LispVal::Nil),
    }
    pc += 1;
}

Op::VecAssoc => {
    let val = stack.pop().unwrap_or(LispVal::Nil);
    let idx = stack.pop().unwrap_or(LispVal::Nil);
    let vec = stack.pop().unwrap_or(LispVal::Nil);
    match (&idx, &vec) {
        (Num(i), LispVal::Vec(v)) if *i >= 0 && (*i as usize) < v.len() => {
            let mut new_v = v.clone();
            new_v[*i as usize] = val;
            stack.push(LispVal::Vec(new_v));
        }
        _ => stack.push(LispVal::Nil),  // error → nil
    }
    pc += 1;
}

Op::VecLen => {
    match stack.pop() {
        Some(LispVal::Vec(v)) => stack.push(LispVal::Num(v.len() as i64)),
        _ => stack.push(LispVal::Num(0)),
    }
    pc += 1;
}

Op::VecConj => {
    let val = stack.pop().unwrap_or(LispVal::Nil);
    let vec = stack.pop().unwrap_or(LispVal::Nil);
    match vec {
        LispVal::Vec(mut v) => { v.push(val); stack.push(LispVal::Vec(v)); }
        LispVal::Nil => stack.push(LispVal::Vec(vec![val])),
        _ => stack.push(LispVal::Nil),
    }
    pc += 1;
}

Op::VecContains => {
    let target = stack.pop().unwrap_or(LispVal::Nil);
    let vec = stack.pop().unwrap_or(LispVal::Nil);
    match &vec {
        LispVal::Vec(v) => stack.push(LispVal::Bool(v.contains(&target))),
        _ => stack.push(LispVal::Bool(false)),
    }
    pc += 1;
}

Op::VecSlice => {
    let sub = stack.pop().unwrap_or(LispVal::Nil);
    let start = stack.pop().unwrap_or(LispVal::Nil);
    let vec = stack.pop().unwrap_or(LispVal::Nil);
    match (&start, &sub, &vec) {
        (Num(s), Num(e), LispVal::Vec(v)) => {
            let si = (*s as usize).max(0).min(v.len());
            let ei = (*e as usize).max(si).min(v.len());
            stack.push(LispVal::Vec(v[si..ei].to_vec()));
        }
        _ => stack.push(LispVal::Nil),
    }
    pc += 1;
}
```

## 3. Lisp Surface Syntax

```
(vec 1 2 3)           → MakeVec(3) — push 1, push 2, push 3, construct
(vec-nth v 0)         → VecNth
(vec-assoc v 0 42)    → VecAssoc — v with index 0 set to 42
(vec-len v)           → VecLen
(vec-conj v 42)       → VecConj — append 42
(vec-contains v x)    → VecContains
(vec-slice v 0 3)     → VecSlice — v[0:3]
```

**Compiler wiring** — same pattern as `list`:
```rust
// In compile_expr, add after the "list" fast path:
Sym("vec") => {
    if args.is_empty() {
        self.code.push(Op::MakeVec(0));
    } else {
        for arg in args { self.compile_expr(arg)?; }
        self.code.push(Op::MakeVec(args.len()));
    }
}
```

The builtins `vec-nth`, `vec-assoc`, `vec-len`, `vec-conj`, `vec-contains`, `vec-slice` get special-form compilation (like `list` does), lowering to the opcodes directly instead of `BuiltinCall`.

## 4. WASM Emission

Same pattern as `MakeList`. Each opcode maps to a WASM helper function:
- `make_vec(n)` — pop n, reverse, construct array
- `vec_nth` — bounds check, index
- `vec_assoc` — clone + index write
- `vec_len` — array length
- `vec_conj` — push
- `vec_contains` — linear scan
- `vec_slice` — bounds-checked subarray

WASM side: stored as an arrayref on the linear memory stack. Tag byte distinguishes `List` from `Vec`.

## 5. F* Model

### Types (Lisp.Types.fst)

```fstar
// Add to lisp_val:
  | Vec    of seq lisp_val    (* FStar.Seq.seq — axiomatized, SMT-friendly *)

// Add to opcode:
  | MakeVec       of nat
  | VecNth
  | VecAssoc
  | VecLen
  | VecConj
  | VecContains
  | VecSlice
```

Why `seq` not `list`: `FStar.Seq` is axiomatized over SMT's array theory. Length, index, update, sub — all come with pre-proved lemmas. No induction needed for structural properties.

### VM Step (Lisp.Source.fst)

```fstar
| MakeVec n ->
    let items, stack = take_stack n s.stack in
    { s with stack = Vec (Seq.of_list items) :: stack; pc = s.pc + 1 }

| VecNth ->
    match s.stack with
    | Num i :: Vec v :: rest ->
        if 0 <= i && i < Seq.length v
        then { s with stack = Seq.index v (i + 0) :: rest; pc = s.pc + 1 }  (* +0 for type: int → nat *)
        else { s with stack = Nil :: rest; pc = s.pc + 1 }
    | _ -> { s with ok = false }

| VecAssoc ->
    match s.stack with
    | val :: Num i :: Vec v :: rest ->
        if 0 <= i && i < Seq.length v
        then { s with stack = Vec (Seq.update v (i + 0) val) :: rest; pc = s.pc + 1 }
        else { s with stack = Nil :: rest; pc = s.pc + 1 }
    | _ -> { s with ok = false }

| VecLen ->
    match s.stack with
    | Vec v :: rest ->
        { s with stack = Num (Seq.length v + 0) :: rest; pc = s.pc + 1 }  (* nat → int *)
    | _ -> { s with stack = Num 0 :: s.stack; pc = s.pc + 1 }

| VecConj ->
    match s.stack with
    | val :: Vec v :: rest ->
        { s with stack = Vec (Seq.append v (Seq.singleton val)) :: rest; pc = s.pc + 1 }
    | val :: Nil :: rest ->
        { s with stack = Vec (Seq.singleton val) :: rest; pc = s.pc + 1 }
    | _ -> { s with ok = false }

| VecContains ->
    match s.stack with
    | target :: Vec v :: rest ->
        { s with stack = Bool (Seq.exists (fun x -> x = target) v) :: rest; pc = s.pc + 1 }
    | _ -> { s with stack = Bool false :: s.stack; pc = s.pc + 1 }

| VecSlice ->
    match s.stack with
    | Num e :: Num s :: Vec v :: rest ->
        let si = max 0 (min s (Seq.length v + 0)) in
        let ei = max si (min e (Seq.length v + 0)) in
        { s with stack = Vec (Seq.slice v si ei) :: rest; pc = s.pc + 1 }
    | _ -> { s with ok = false }
```

**Note on `VecContains`:** Uses `Seq.exists (fun x -> x = target)` which hits `noeq` — `lisp_val` equality isn't structural. Two options:
1. **Skip it in F*** — keep it as a builtin-only, don't model in the formal spec. On-chain it works via Rust's `PartialEq`.
2. **Use a custom `lisp_val_eq`** function (already exists in `Lisp.Values.fst`) instead of `=`.

Recommended: option 2 for consistency, but mark it `assume` to avoid the `noeq` issue in proofs.

### Value Operations (Lisp.Values.fst)

```fstar
val vec_len : lisp_val -> Tot int
let vec_len v =
  match v with
  | Vec s -> Seq.length s + 0  (* nat → int *)
  | _ -> 0

val vec_nth : v:lisp_val -> i:int -> Tot (option lisp_val)
let vec_nth v i =
  match v with
  | Vec s ->
    if 0 <= i && i < Seq.length s + 0
    then Some (Seq.index s (i + 0))
    else None
  | _ -> None

val vec_is_vec : lisp_val -> Tot bool
let vec_is_vec v = match v with Vec _ -> true | _ -> false
```

## 6. Properties We Get For Free

With `seq` in F*, these are **axiomatically true** — no induction, no custom lemmas:

### Length properties
```
vec_len (Vec []) = 0
vec_len (vec-conj (Vec v) x) = vec_len (Vec v) + 1
vec_len (vec-assoc (Vec v) i x) = vec_len (Vec v)
```

### Index properties (SMT array theory)
```
vec_nth (Vec v) i = Some (Seq.index v i)          when in bounds
vec_nth (vec-assoc (Vec v) i x) i = Some x         when in bounds
vec_nth (vec-assoc (Vec v) i x) j = vec_nth (Vec v) j  when i ≠ j and both in bounds
```

### Map preserves length
```
vec_len (vec-map f (Vec v)) = vec_len (Vec v)
```
Proof: by `Seq.length (Seq.map f v) = Seq.length v` — axiom.

### Slice properties
```
vec_len (vec-slice (Vec v) a b) = b - a              when 0 ≤ a ≤ b ≤ len v
vec_nth (vec-slice (Vec v) a b) i = vec_nth (Vec v) (a + i)  when in bounds
```

## 7. Implementation Order

1. **Rust types** — add `Vec` variant to `LispVal`, update `PartialEq`, `Display`, `Clone`
2. **Rust opcodes** — add 7 opcodes to `Op`, implement in both `run()` and `run_checked()`
3. **Compiler** — wire `vec`, `vec-nth`, `vec-assoc`, `vec-len`, `vec-conj`, `vec-contains`, `vec-slice`
4. **Tests** — basic roundtrip tests (construct, index, assoc, len, conj, slice)
5. **WASM emission** — add opcode handlers in `wasm_emit.rs`
6. **F* model** — add `Vec` to `lisp_val`, opcodes to `opcode`, VM step rules
7. **F* proofs** — length preservation, index-after-assoc, map-preserves-length

## 8. Open Questions

- **Tag byte in WASM:** Need to distinguish `List` from `Vec` on the WASM side. Currently `List` has tag N. `Vec` gets tag N+1. Need to check wasm_emit.rs for the tag scheme.
- **Equality semantics:** `VecContains` needs structural equality. In F* we model it with `assume` or a custom `lisp_val_eq`. In Rust it just works via `PartialEq`.
- **Clojure interop:** Should `(vec [1 2 3])` convert a list to a vec? Should `(into [])` convert back? Low priority — can add later.
- **HOF fusion:** Should `MapOp`/`FilterOp`/`ReduceOp` also work on `Vec`? Yes — the fused HOF opcodes should dispatch on both `List` and `Vec` variants.
