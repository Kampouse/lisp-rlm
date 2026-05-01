# lisp-rlm

A Lisp dialect that compiles to tiny NEAR smart contracts. Write Lisp, get sub-1KB WASM that runs on NEAR testnet/mainnet.

**903 bytes vs near-sdk's 35KB** for equivalent contracts.

## Quick Start

```bash
git clone https://github.com/Kampouse/lisp-rlm.git
cd lisp-rlm
cargo build
```

One binary: `near-compile`

```bash
# Compile to WASM
cargo run --bin near-compile -- contract.lisp contract.wasm
# ✅ contract.wasm (767 bytes) — validated

# Interactive REPL (local, instant)
cargo run --bin near-compile -- --repl
# ⚡ NEAR Lisp REPL (WASM + wasmtime, mock NEAR runtime)

# Run inline tests
cargo run --bin near-compile -- test contract.lisp
```

## Your First Contract

```lisp
(memory 4)

(define (increment)
  (let ((n (+ (near/storage_get "counter") 1)))
    (begin
      (near/storage_set "counter" n)
      n)))

(define (get_count)
  (near/storage_get "counter"))

(define (reset)
  (begin
    (near/storage_remove "counter")
    0))

(export "get_count" get_count true)
(export "increment" increment false)
(export "reset" reset false)
```

**767 bytes.** Compiles, validates, deploys to NEAR testnet.

## The REPL

The REPL compiles every expression through the **same WASM emitter** as `near-compile`. It runs locally via wasmtime with a mock NEAR environment — catches type mismatches, i32/i64 bugs, and all WASM-level issues before you deploy.

```
⚡ NEAR Lisp REPL (WASM + wasmtime, mock NEAR runtime)
   Storage persists between expressions ✨

> (+ 1 2)
3
> (hof/reduce (lambda (acc x) (+ acc x)) 0 1 6)
15
> (near/log "sum=" 15)
  LOG: sum=15
> (near/storage_set "counter" 42)
0
> (near/storage_get "counter")
42
> (near/storage_remove "counter")
1
> (near/storage_get "counter")
0
```

### Live Testnet from the REPL

Write code, test locally, deploy, and interact with the live contract — all from one terminal:

```
> (define (increment) ...)
✓ (383 bytes)
> (define (get_count) ...)
✓ (383 bytes)
> (near/storage_set "counter" 0)     ← test locally
0
> (near/storage_get "counter")
0
> :push                               ← deploy to testnet
✅ https://explorer.testnet.near.org/transactions/...
> :call get_count                     ← view on-chain (free)
📋 get_count → 0
> :call! increment                    ← mutate on-chain (costs gas)
📋 1
> :call get_count
📋 1
```

### REPL Commands

| Command | Description |
|---------|-------------|
| `:help` | Show available commands |
| `:quit` | Exit |
| `:defs` | Show all defined functions |
| `:reset` | Clear definitions and storage |
| `:wat` | Show compiled WASM (WAT format) |
| `:size` | Show WASM byte size |
| `:push` | Deploy all definitions to testnet |
| `:call fn` | View call on testnet (free) |
| `:call! fn` | Mutable call on testnet (costs gas) |
| `:near` | Deploy last compiled WASM |

## Language Reference

### Types
Everything is `i64`. Numbers, booleans (0/1), nil = 0.

### Arithmetic & Comparison
```
+  -  *  /  mod  abs
<  >  <=  >=  =  !=
```

### Logic & Control Flow
```
and  or  not
if  let  begin  set!
while  for
```

### Higher-Order Functions (inline)
```lisp
(hof/map    (lambda (x) (* x 2)) 1 6)     ; → count=5, writes to memory
(hof/filter (lambda (x) (= (mod x 2) 0)) 1 11)  ; → 5
(hof/reduce (lambda (acc x) (+ acc x)) 0 1 6)    ; → 15
```

Lambdas are inlined at compile time — no runtime closures, no function pointers.

### Storage
```lisp
(near/storage_set "key" value)    ; Store i64 under string key
(near/storage_get "key")          ; Get i64 (0 if not found)
(near/storage_has "key")          ; 1 if exists, 0 if not
(near/storage_remove "key")       ; Remove, returns 1 if existed
```

Storage persists between contract calls on-chain. In the REPL, storage persists within the session.

### Logging
```lisp
(near/log "hello")                ; Log string
(near/log "count=" 42)            ; Log "count=42" as single line
(near/log_num 99)                 ; Log number as decimal
```

### Function Definitions & Exports
```lisp
(define (name params...) body)
(export "name" func view_only)
```

- `true` = view function (read-only, free to call)
- `false` = mutable function (can modify state, costs gas)

### FP64 Fixed-Point
```lisp
(fp64/set_int val)
(fp64/mul a b)
(fp64/div a b)
(fp64/sqrt val)
```

Q64.64 precision for fixed-point arithmetic.

## Inline Tests

Write tests directly in your source:

```lisp
(memory 4)

(define (double x) (* x 2))
(define (square x) (* x x))

(test "double basic" (double 5) 10)
(test "double zero" (double 0) 0)
(test "square" (square 7) 49)
(test "sum 1..5" (hof/reduce (lambda (acc x) (+ acc x)) 0 1 6) 15)
```

```bash
$ cargo run --bin near-compile -- test file.lisp
✅ double basic: 10
✅ double zero: 0
✅ square: 49
✅ sum 1..5: 15
4 passed, 0 failed
```

## WASM Validation

The compiler validates WASM structure before writing. If there's a type mismatch, you get:

```
❌ WASM error in function `my_func`: type mismatch at offset 0xd6
```

Not a cryptic deserialization error on testnet.

## Error Messages

The compiler catches common mistakes at compile time:

```
Type error: (+) expects numeric arguments, got string at argument 1
Error: function 'foo' is not defined. Did you mean 'for'?
Error: '__hof_it' is an internal variable used by hof/map — not accessible from user code
```

## Project Configuration

Create a `.near-config.json` (optional):
```json
{
  "account": "kampy.testnet",
  "network": "testnet",
  "key_path": "~/.near-credentials/testnet/kampy.testnet.json"
}
```

## Architecture

```
input.lisp
    ↓ parse
LispVal AST
    ↓ typecheck (catches type errors)
    ↓ WasmEmitter::compile_near()
wasm-encoder Module
    ↓ wasmparser validation
output.wasm (binary, no WAT strings)
    ↓ deploy to NEAR
live contract
```

The WASM emitter produces binary directly via `wasm-encoder` — no string-based WAT generation, impossible to emit structurally invalid modules.

## Performance

- 1M tail-recursive iterations in a single view call
- 10M while-loop iterations
- **767 bytes** for a counter contract with 3 methods
- Lambda bodies inlined at compile time — zero runtime overhead

## Current Limitations

See [GAPS.md](GAPS.md) for the full feature tracker.

**Not yet supported:**
- Cross-contract calls (promises, callbacks)
- u128 / BigInt (only i64)
- Dynamic lists at runtime (only memory-backed arrays)
- Borsh serialization / JSON I/O
- Pattern matching

**What works:**
- ✅ Arithmetic, logic, control flow
- ✅ Storage (read/write/remove/has)
- ✅ Higher-order functions (map/filter/reduce, inlined)
- ✅ Logging (string + number, single-line)
- ✅ FP64 fixed-point math
- ✅ REPL with mock NEAR runtime + live testnet calls
- ✅ Inline tests
- ✅ WASM validation + type checking

## License

MIT
