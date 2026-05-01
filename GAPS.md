# near-compile — Gap Tracker

## Status: Works on NEAR testnet ✅

Last verified: 2026-05-01 on kampy.testnet

---

## ✅ What Works

### Core
- [x] i64 integers (all values are i64)
- [x] Arithmetic: `+`, `-`, `*`, `/`, `mod`, `abs`
- [x] Comparison: `<`, `>`, `<=`, `>=`, `=`, `!=`
- [x] Logic: `and`, `or`, `not`
- [x] Control flow: `if`/`else`, `begin`, `while`, `for`
- [x] Bindings: `let`, `set!`
- [x] String literals (packed ptr|len<<32)

### Functions
- [x] `(define (name params...) body)` — function definitions
- [x] `(export "name" func true)` — NEAR view exports
- [x] Multi-expression bodies (implicit begin)
- [x] Gas metering (depth counter + eval budget)
- [x] Function args via `env.input` / `read_register`

### Memory
- [x] `(memory N)` — declare pages
- [x] `i64.load` / `i64.store` — direct memory access
- [x] `i32.store8` — byte-level writes

### Higher-Order Functions (inline)
- [x] `(hof/map (lambda (x) body) start end [offset])` — map range, writes to memory
- [x] `(hof/filter (lambda (x) pred) start end [offset])` — filter range
- [x] `(hof/reduce (lambda (acc x) body) init start end)` — fold range
- [x] Lambda body inlined at compile time (no runtime dispatch)

### Loop Macros
- [x] `(reduce init start end acc body)` — numeric accumulation
- [x] `(map-into offset start end body)` — write transformed values to memory
- [x] `(for var start end body...)` — counted loop

### NEAR Builtins
- [x] `near/return` — value_return
- [x] `near/return_str` — string return
- [x] `near/log` — log packed string
- [x] `near/log_num` — **log i64 as decimal** (i64→ASCII conversion in WASM)
- [x] `near/panic` / `near/abort`
- [x] `near/input` — read call args
- [x] `near/block_index`

### FP64 Fixed-Point
- [x] `fp64/set_int`, `fp64/get_int`, `fp64/get_frac`
- [x] `fp64/mul`, `fp64/div`, `fp64/sqrt`
- [x] Q64.64 precision via 32-bit splits

### Tooling
- [x] WASM validation on compile (catches type mismatches before deploy)
- [x] Deployed contracts verified on testnet (all functions return correct values)

---

## ❌ Gaps — Not Yet Implemented

### Critical for Real Contracts

- [ ] **Cross-contract calls** — No `promise_create`, `promise_then`, `promise_results`
  - Need: `(near/call account_id method args deposit gas)`
  - Need: callback support for async patterns
  - **Priority: HIGH** — without this, contracts can't interact with anything

- [ ] **Cross-contract call results** — No way to read async results
  - Need: callback functions that receive promise results

- [ ] **Storage** — No `storage_write`, `storage_read`, `storage_remove`
  - Need: persistent state between calls
  - Without this, contracts are pure functions (no state)
  - **Priority: HIGH** — state is fundamental for contracts

- [ ] **Balance/transfer** — No `balance`, `attached_deposit`, `transfer`
  - Can't write financial contracts

### Important for Usability

- [ ] **Float values** — Only i64 supported
  - No `f64` WASM type
  - FP64 fixed-point works but is cumbersome
  - **Priority: MEDIUM**

- [ ] **Dynamic lists** — No runtime list data structure
  - `hof/map` writes to raw memory, returns count
  - No way to pass lists between functions
  - No `car`, `cdr`, `cons`, `list` at runtime
  - **Priority: MEDIUM**

- [ ] **String operations** — Very limited
  - String literals exist (packed ptr+len)
  - No `string-append`, `substring`, `string-length`
  - No `number->string` (only via `near/log_num` which logs, doesn't return)
  - **Priority: MEDIUM**

- [ ] **Pattern matching** — No `match`
  - Bytecode VM has it (broken), WASM emitter doesn't
  - **Priority: LOW**

### Nice to Have

- [ ] **Closure support** — Lambdas only inlined at compile time
  - Can't pass lambdas to user-defined functions
  - Can't return lambdas from functions
  - Y combinator not supported in WASM emitter

- [ ] **Recursion** — No self-calls in emitted WASM
  - `CallSelf` / `PushSelf` only in bytecode VM
  - WASM emitter has no recursive function support

- [ ] **BigInt** — No arbitrary precision
  - NEAR uses u128 for balances, this compiler only does i64

- [ ] **Enums / tagged unions** — No ADTs
  - No way to represent Result<T, E> or Option<T>

- [ ] **`match` / pattern matching** — Not in WASM emitter

- [ ] **Global definitions** — `(define x value)` only defines functions
  - Can't define constants or computed values at module level
  - Workaround: use `(define (x) value)` and call `(x)`

- [ ] **Multi-file / imports** — Single file only
  - No `require` / `import` for WASM target
  - No stdlib loading

---

## Known Bugs

- [ ] **Double value_return**: Functions using `near/return` AND the export wrapper both call `value_return`. The wrapper's call (with 0) wins. **Workaround**: don't use `near/return` inside exported functions — just return the value, the wrapper handles it.
- [ ] **`near/log_num` buffer**: Uses memory at 4096..4120. If contract uses same area, data corruption. Should use a dedicated log buffer.

---

## Bytecode VM vs WASM Emitter

| Feature | Bytecode VM (Rust) | WASM Emitter (near-compile) |
|---------|--------------------|-----------------------------|
| Types | i64, f64, string, list, map, bool, nil | i64 only |
| map/filter/reduce | ✅ Runtime HOF | ✅ Inline only (hof/*) |
| Lambda closures | ✅ Captures env | ❌ Inlined body only |
| Pattern matching | 🐛 Broken | ❌ Not implemented |
| Cross-contract calls | ❌ | ❌ |
| Storage | ❌ | ❌ |
| Recursion | ✅ CallSelf | ❌ |
| String ops | ✅ Runtime | ❌ Minimal |
| FP64 math | ✅ Via builtins | ✅ Via builtins |
| Runs on NEAR | ❌ (native Rust) | ✅ (WASM contract) |

---

## Architecture

```
input.lisp
    ↓ parse
LispVal AST
    ↓ WasmEmitter::compile_near()
wasm-encoder Module
    ↓ .finish("_run")
raw WASM bytes
    ↓ wasmparser validation
output.wasm → deploy to NEAR
```

The WASM emitter is a one-pass compiler. No IR, no optimization passes.
Lambda bodies are inlined at the call site — no closures, no function pointers.
All locals are i64. Memory layout is manual (offsets specified by user).
