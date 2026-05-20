# lisp-rlm

A Lisp dialect that compiles to tiny NEAR smart contracts. Write Lisp, get sub-1KB WASM that runs on NEAR testnet/mainnet.

**→ Try it in your browser: [lisp-rlm.pages.dev](https://lisp-rlm.pages.dev)**

**903 bytes vs near-sdk's 35KB** for equivalent contracts.

## Quick Start

```bash
git clone https://github.com/Kampouse/lisp-rlm.git
cd lisp-rlm
cargo build --release
```

One binary: `near-compile`

```bash
# Scaffold a project
near-compile init my-contract
cd my-contract

# Build WASM
near-compile build

# Deploy to NEAR
near-compile deploy

# Call a contract method
near-compile call kampy.testnet get_count '{}'

# Create a subaccount
near-compile create myapp --account kampy.testnet

# Run tests
near-compile test

# Interactive REPL
near-compile --repl
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

## CLI Reference

```
near-compile init <name>              Scaffold a new project
near-compile build [dir]              Build project from near.json
near-compile build --target=outlayer   Build for OutLayer WASI
near-compile build --target=outlayer-p2 Build for WASI P2 Component

near-compile deploy [dir]             Build and deploy to NEAR
  --account <id>      Override account from near.json
  --network <net>     Override network (testnet|mainnet)
  --key-path <path>   Override key file path
  --seed-phrase        Read seed phrase from stdin (SLIP-0010)

near-compile call <contract> <method> [args.json|'{}'] [dir]
  --account, --network, --key-path, --seed-phrase
  --deposit <amount>   Attach NEAR deposit (e.g. "0.1")
  --gas <gas>          Gas limit (default 300 TGas)

near-compile create <account-id> [funder-account-id]
  --account, --network, --key-path, --seed-phrase
  --fund               Auto-fund from testnet faucet
  Saves credentials to ~/.near-credentials/

near-compile test [dir]               Build and run tests
near-compile --repl                   Interactive REPL
near-compile bench <file>             Benchmark with fuel metering
```

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

### Cross-Contract Calls
```lisp
(near/call "contract.near" "method" args_ptr args_len amount_ptr amount_len gas)
(near/promise_then promise_id "contract.near" "callback" args_ptr args_len amount_ptr amount_len gas)
(near/promise_return promise_id)
```

Full promise API: `promise_create`, `promise_then`, `promise_and`, `promise_return`, `promise_result`, `promise_yield_create`, `promise_yield_resume`, plus batch actions (`promise_batch_create`, `promise_batch_then`, `promise_batch_action_*`).

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

### NEAR Host Functions

92 host functions available, 20/20 verified on testnet:

**Core:** `input`, `return`, `return_str`, `log`, `log_num`, `abort`, `panic`

**Storage:** `storage_set`, `storage_get`, `storage_has`, `storage_remove`, `storage_usage`

**Account:** `current_account_id`, `signer_account_id`, `signer_account_pk`, `predecessor_account_id`

**Balances:** `account_balance`, `account_balance_high`, `account_locked_balance`, `account_locked_balance_high`, `attached_deposit`, `attached_deposit_high`

**Crypto:** `keccak256`, `sha256`, `ripemd160`, `ecrecover`, `ed25519_verify`, `random_seed`

**Promises:** `promise_create`, `promise_then`, `promise_and`, `promise_return`, `promise_result`, `promise_results_count`, `promise_batch_create`, `promise_batch_then`, `promise_batch_action_function_call`, `promise_batch_action_transfer`, `promise_batch_action_stake`, `promise_batch_action_add_key_with_full_access`, `promise_batch_action_delete_key`, `promise_batch_action_create_account`, `promise_batch_action_deploy_contract`, `promise_batch_action_delete_account`, `promise_yield_create`, `promise_yield_resume`, `promise_set_refund_to`

**Iteration:** `iter_prefix`, `iter_range`, `iter_next`

**JSON I/O:** `json_get_int`, `json_get_str`, `json_return_int`, `json_return_str`

**Blockchain:** `block_index`, `block_timestamp`, `epoch_height`, `prepaid_gas`, `used_gas`, `validator_stake`, `validator_total_stake`

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
$ near-compile test file.lisp
✅ double basic: 10
✅ double zero: 0
✅ square: 49
✅ sum 1..5: 15
4 passed, 0 failed
```

## Web Playground

Browser-based Lisp-to-WASM compiler with a NEAR mock runtime — test contracts in the browser without installing anything.

Live at: [lisp-rlm.pages.dev](https://lisp-rlm.pages.dev)

Features:
- Code editor with syntax highlighting
- Compile → WASM in-browser
- Mock NEAR runtime for testing storage, logging, and crypto
- Test runner with pass/fail reporting

## Project Configuration

`near.json` in the project root:

```json
{
  "account": "kampy.testnet",
  "network": "testnet",
  "key_path": "~/.near-credentials/testnet/kampy.testnet.json",
  "output": "target/contract.wasm"
}
```

All fields optional — override with CLI flags.

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

## Architecture

```
input.lisp
    ↓ resolve_modules() — text-level #include
    ↓ parse
LispVal AST
    ↓ typecheck (catches type errors)
    ↓ WasmEmitter::compile_near()
    ↓ tree_shake() — remove unused functions
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

## License

MIT
