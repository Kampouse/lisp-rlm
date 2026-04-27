# lisp-rlm Verification — F* Formal Specification

## What This Proves

F* specifications for the lisp-rlm bytecode VM runtime. Following [vWasm](https://github.com/secure-foundations/vWasm)'s architecture:

| vWasm file | lisp-rlm equivalent |
|---|---|
| `semantics/wasm/Wasm.Types.fst` | `semantics/lisp/Lisp.Types.fst` |
| `semantics/wasm/Wasm.Eval_numeric.fst` | `semantics/lisp/Lisp.Values.fst` |
| `semantics/wasm/Wasm.Eval.fst` | `semantics/lisp_ir/LispIR.Semantics.fst` |
| `compiler/sandbox/Compiler.Sandbox.fsti` | `semantics/lisp_ir/LispIR.Correctness.fst` |

## Key Properties Verified

1. **Float comparison precision** — `num_cmp` never truncates floats to ints. For `Float(0.9) > Float(0.3)`, the result is `true`, not `false` (the bug we shipped).
2. **Type promotion in mixed arithmetic** — `Num(1) + Float(2.0)` produces `Float(3.0)`, not `Num(3)`.
3. **Stack discipline** — comparison ops pop 2, push 1 Bool. Arithmetic ops pop 2, push 1 result.
4. **Determinism** — same state + same op always produces same result (trivial in F* by purity).
5. **Slot bounds safety** — `LoadSlot`/`StoreSlot` with out-of-bounds index returns error, not UB.

## The Bug This Would Have Caught

The `num_val` truncation bug (fixed in commit `bf9481`):

```rust
// OLD (broken): cast Float(0.9) → i64(0)
Op::Gt => {
    let b = num_val(stack.pop());  // Float(0.3) → 0
    let a = num_val(stack.pop());  // Float(0.9) → 0
    stack.push(Bool(a > b));       // 0 > 0 → false. WRONG!
}

// NEW (fixed): float-aware comparison
Op::Gt => {
    let b = stack.pop();
    let a = stack.pop();
    stack.push(Bool(num_cmp(&a, &b, |x,y| x > y, |x,y| x > y)));
}
```

The F* spec in `Lisp.Values.num_cmp` makes the old implementation **impossible to write** — F* rejects `Float f → int_of_float f` in a comparison context because the type system tracks that the result type doesn't match the input type.

## Structure

```
verification/
├── semantics/
│   ├── lisp/
│   │   ├── Lisp.Types.fst       — LispVal, Op, BinOp, Ty, vm_state types
│   │   └── Lisp.Values.fst      — num_cmp, num_arith, lisp_eq, dict ops
│   └── lsp_ir/
│       ├── LispIR.Semantics.fst — eval_op (VM step), eval_steps (multi-step)
│       └── LispIR.Correctness.fst — top-level correctness properties
├── tests/
│   └── CompareSpec.fst          — unit tests for the exact bugs we found
├── Makefile                     — F* verification + OCaml extraction
└── README.md                    — this file
```

## Setup

```bash
# Install F* (requires .NET SDK)
dotnet tool install --global FStar

# Or use opam
opam install fstar

# Verify all specs
make verify

# Extract to OCaml for property-based testing against Rust
make extract
```

## 80/20 Assessment

| What | Effort | Value |
|---|---|---|
| Types + Values specs | 1 day | Catches all type confusion bugs |
| VM step semantics | 2 days | Catches stack underflow, bounds errors |
| Correctness lemmas | 1 day | Proves float precision, type preservation |
| **Total (80/20)** | **4 days** | **Catches the real bugs we shipped** |
| Full compiler correctness | 3-6 months | Proves bytecode ≡ tree-walking |

The 4-day investment catches bugs like `num_val` truncation. The 6-month investment proves the compiler is correct for all possible inputs. The 80/20 is clear.
