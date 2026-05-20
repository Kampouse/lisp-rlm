# near-compile — Gap Tracker

## Status: Works on NEAR testnet ✅

Last verified: 2026-05-20 on kampy.testnet (92 host functions, cross-contract calls, native deploy CLI)

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
- [x] `(hof/map (lambda (x) body) start end [offset])` — map range
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
- [x] `near/log_num` — log i64 as decimal (i64→ASCII in WASM)
- [x] `near/panic` / `near/abort`
- [x] `near/input` — read call args
- [x] `near/block_index`

### Storage
- [x] `near/storage_set` — write key/value to NEAR storage
- [x] `near/storage_get` — read value from NEAR storage
- [x] `near/storage_has` — check key exists
- [x] `near/storage_remove` — delete key
- [x] Persistent storage in REPL (HashMap across calls)
- [x] Counter contract verified on testnet (state persists on-chain)

### u128
- [x] Memory-based u128 (16 bytes at offset, passed as i64 pointer)
- [x] `u128/from_yocto "amount" offset` — compile-time decimal parsing
- [x] `u128/new hi lo offset`, `u128/from_i64 n offset`, `u128/to_i64 offset`
- [x] `u128/add dst src`, `u128/sub dst src`, `u128/mul dst val` (in-place)
- [x] `u128/store addr lo hi`, `u128/load addr`, `u128/load_high addr`
- [x] `u128/eq a1 a2`, `u128/is_zero addr`, `u128/lt a1 a2`
- [x] `u128/store_storage "key" src`, `u128/load_storage "key" dst`

### FP64 Fixed-Point
- [x] `fp64/set_int`, `fp64/get_int`, `fp64/get_frac`
- [x] `fp64/mul`, `fp64/div`, `fp64/sqrt`
- [x] Q64.64 precision via 32-bit splits

### Tooling
- [x] WASM validation on compile (wasmparser + function-name error mapping)
- [x] Type checking (lightweight pre-pass: Num, Bool, Str, Void, Any)
- [x] Better error messages (Levenshtein suggestions, internal var mapping)
- [x] Inline tests: `(test "name" expr expected)` via `near-compile test`
- [x] REPL with wasmtime mock NEAR runtime
- [x] Live testnet: `:push`, `:call`, `:call!` in REPL
- [x] Persistent memory (256KB) + storage in REPL
- [x] Project system: `near.json` + `init`, `build`, `deploy`, `test`
- [x] Module imports: `(module name "path")` — C-style #include
- [x] Circular dependency detection
- [x] Tree-shaking: unused functions stripped from binary
- [x] REPL auto-loads project defines (including modules)

---

## ❌ Gaps — Not Yet Implemented

### Critical for Real Contracts

- [ ] **Cross-contract calls** — No `promise_create`, `promise_then`, `promise_results`
  - Need: `(near/call account_id method args deposit gas)`
  - Need: callback support for async patterns
  - **Priority: HIGH**

- [ ] **JSON input/output** — All contract calls use `{}` args
  - Can't pass parameters to contracts
  - Need: `near/json_get_int`, `near/json_get_str`, `near/json_return`
  - **Priority: HIGH** — without this, contracts can't receive arguments

- [ ] **AccountId type** — signer/predecessor returned as raw bytes
  - No string comparison for access control
  - **Priority: HIGH** — needed for any auth

### Important for Usability

- [ ] **u128/to_string** — Can't log full 128-bit values
  - Only `u128/to_i64` shows low 64 bits
  - Division-by-10 in raw WASM is complex
  - **Priority: MEDIUM**

- [ ] **Float values** — Only i64 supported
  - No `f64` WASM type
  - FP64 fixed-point works but is cumbersome
  - **Priority: MEDIUM**

- [ ] **Dynamic lists** — No runtime list data structure
  - `hof/map` writes to raw memory, returns count
  - No `car`, `cdr`, `cons`, `list` at runtime
  - **Priority: MEDIUM**

- [ ] **String operations** — Very limited
  - String literals exist (packed ptr+len)
  - No `string-append`, `substring`, `string-length`
  - **Priority: MEDIUM**

### Nice to Have

- [ ] **Closure support** — Lambdas only inlined at compile time
- [ ] **Recursion** — No self-calls in emitted WASM
- [ ] **Enums / tagged unions** — No ADTs
- [ ] **Borsh serialization** — For NEAR state
- [ ] **Vec/dynamic arrays**
- [ ] **Global definitions** — `(define x value)` only defines functions
  - Workaround: `(define (x) value)` and call `(x)`

---

## Known Bugs

- [ ] **Double value_return**: Functions using `near/return` AND the export wrapper both call `value_return`. The wrapper's call (with 0) wins. **Workaround**: don't use `near/return` inside exported functions — just return the value, the wrapper handles it.
- [ ] **Combined logging**: `(near/log "str" num)` causes WASM stack imbalance — can't combine string+number in single log line yet. Two separate log lines work fine.
- [ ] **REPL `:call!` return**: After mutable call, `:call` shows stale value (block cache). Wait a block and retry.

---

## Bytecode VM vs WASM Emitter

| Feature | Bytecode VM (Rust) | WASM Emitter (near-compile) |
|---------|--------------------|-----------------------------|
| Types | i64, f64, string, list, map, bool, nil | i64 only |
| map/filter/reduce | ✅ Runtime HOF | ✅ Inline only (hof/*) |
| Lambda closures | ✅ Captures env | ❌ Inlined body only |
| Pattern matching | 🐛 Broken | ❌ Not implemented |
| Cross-contract calls | ❌ | ❌ |
| Storage | ❌ | ✅ |
| u128 | ❌ | ✅ |
| Recursion | ✅ CallSelf | ❌ |
| String ops | ✅ Runtime | ❌ Minimal |
| FP64 math | ✅ Via builtins | ✅ Via builtins |
| Tree-shaking | ❌ | ✅ |
| Module system | ❌ | ✅ |
| Project files | ❌ | ✅ |
| Runs on NEAR | ❌ (native Rust) | ✅ (WASM contract) |

---

## Architecture

```
input.lisp
    ↓ resolve_modules() — text-level #include
    ↓ parse
LispVal AST
    ↓ typecheck_expr() — lightweight pre-pass
    ↓ WasmEmitter::compile_near()
    ↓ tree_shake() — remove unused functions
wasm-encoder Module
    ↓ .finish("_run")
raw WASM bytes
    ↓ wasmparser validation
output.wasm → deploy to NEAR
```

Memory layout:
- TEMP_MEM = 64 (return values)
- LOG_BUF = 4096 (64 bytes, string log buffer)
- NUM_BUF = 4160 (24 bytes, number-to-string buffer)
- STORAGE_BUF = 8192 (8 bytes, i64 storage temp)
- STORAGE_U128_BUF = 8208 (16 bytes, u128 storage temp)
