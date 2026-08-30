# lisp-interp

A Lisp interpreter **written in lisp-rlm** (its own S-expression language), compiled to WebAssembly for NEAR Protocol.

The interpreter reads a string program at runtime, parses it into an arena-allocated AST, evaluates it with an environment-based evaluator, and returns the result as a string.

## Deployed Contract

- **Account:** `lisp6.kampy.testnet` (NEAR testnet)
- **Wasm size:** 23,403 bytes
- **Source:** [src/main.lisp](src/main.lisp)

## Calling the Contract

The contract exposes a single `eval` method that accepts a JSON-wrapped program string.

### View Call (free, read-only)

```bash
near-compile view --account lisp6.kampy.testnet eval '{"program":"(+ 2 3)"}' deploy/lisp-interp
# → 5
```

Or via RPC directly:

```bash
curl -s -X POST https://rpc.testnet.fastnear.com \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":1,"method":"query",
    "params":["call_function","{
      \"account_id\": \"lisp6.kampy.testnet\",
      \"method_name\": \"eval\",
      \"args_base64\": \"'$(echo -n '{"program":"(+ 2 3)"}' | base64)'\",
      \"gas\": 300000000000000
    }",""]
  }' | jq -r '.result.result' | base64 -d
```

### Supported Programs

```lisp
(+ 2 3)                        ; → 5
(* 10 5)                       ; → 50
(- 0 47)                       ; → -47
(/ 100 3)                     ; → 33
(define (sq x) (* x x))       ; define function
(sq 5)                        ; → 25
(define (fact n)              ; recursion
  (if (< n 2) 1 (* n (fact (- n 1)))))
(fact 5)                      ; → 120
(define (fib n)              ; fibonacci
  (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(fib 20)                      ; → 6765
```

### Input Format

The interpreter accepts both formats:
- Raw: `(+ 2 3)`
- JSON-wrapped: `{"program":"(+ 2 3)"}` (what `near-cli` sends)

## Architecture

### Interpreter (49 functions)

| Component | Description |
|-----------|-------------|
| **Reader** | `read-form`, `read-list`, `read-atom`, `skip-ws`, `atom-end`, `delim?`, `char-at`, `ws?` |
| **Arena** | Bump allocator at 2 MiB offset, 24 B/cell (type/val/cdr). `cell-alloc`, `heap-init` |
| **Eval** | `my-eval`, `eval-form`, `eval-special`, `do-if`, `do-begin`, `evargs` |
| **Apply** | `apply-proc`, `bind-params`, `dispatch`, `do-add/sub/mul/div/lt/gt/eq` |
| **Fast path** | `apply-op2`, `do-add2/sub2/mul2/div2/lt2/gt2/eq2` — 2-arg arith skips cons allocation |
| **Env** | `lookup`, `ext`, `setup-env`, `make-env-cell` — linked-list env cells |
| **Predicates** | `null?`, `num?`, `pair?`, `sym?`, `truthy` |
| **IO** | `json-prog`, `do-eval` (reads `near/input`, returns via `near/return_str`) |

### Gas Optimizations

- **2-arg fast path**: Arithmetic operators with exactly 2 args skip `evargs` (which allocates a cons cell per arg) and the `dispatch`/`apply-proc` re-unboxing chain. This is the hot path in `fib` and most numeric programs.
- **Depth guard disabled**: NEAR's runtime already bounds the native wasm stack. The guard's 4 memory-ops per function call burned ~8% gas without ever firing first.
- **No GC**: NEAR zeros contract memory per call. The arena dies with the instance.

### Cell Types

| Type | Tag | Layout |
|------|-----|--------|
| Number | 1 | `[1, value, 0]` |
| Cons/Pair | 2 | `[2, car, cdr]` |
| Symbol | 3 | `[3, hash_code, 0]` |
| Env cell | 4 | `[4, binding_pair, 0]` |

### Special Form Codes

| Code | Form | Code | Builtin |
|------|------|------|---------|
| 1001 | `quote` | 1006 | `=` |
| 1002 | `if` | 1007 | `+` |
| 1003 | `begin` | 1008 | `-` |
| 1004 | `define` | 1009 | `*` |
| 1005 | `lambda` | 1010 | `/` |
| | | 1011 | `<` |
| | | 1012 | `>` |
| | | 1013 | `cons` |
| | | 1014 | `car` |
| | | 1015 | `cdr` |
| | | 1016 | `null?` |
| | | 1017 | `pair?` |

## On-Chain Limits (measured)

| Metric | Limit | Notes |
|--------|-------|-------|
| Gas (view) | ~300 TGas fixed | `fib 20` ✓ (6765), `fib 21` OOG |
| Stack depth | ~150-180 interpreted frames | ~11 wasm frames per `eval` call. `countd 150` ✓, `countd 200` OOB |
| Memory | 256 MiB allocated | Never the limiter on-chain (gas binds first) |

**Stack depth is a hard NEAR constraint**: `return_call` (tail call) is disabled in nearcore's VM (`TAIL_CALL: bool = false` in `features.rs`), alongside SIMD, threads, and multi-memory. Trampolining at the lisp level doesn't help — the trampoline loop itself is recursive and grows the native stack identically.

## Error Handling

- **Unbound symbols** → `GuestPanic("unbound symbol: <hash>")` — the hash is deterministic (h31 polynomial), invertible by re-hashing candidate names.
- **Division by zero** → wasm trap (native `i64.div_u` by zero).
- **Stack overflow** → `Accessed memory outside the bounds` (NEAR's native wasm stack limit).
- **Gas exceeded** → `Exceeded the prepaid gas`.

## Building

```bash
# From the lisp-rlm repo root
cd deploy/lisp-interp
lisp-rlm compile          # → target/lisp-interp.wasm
lisp-rlm deploy --account lisp6.kampy.testnet --network testnet
```

## Testing Locally

```bash
# Compile + run in the local wasmtime-based mock
lisp-rlm compile deploy/lisp-interp
lisp-rlm mock deploy/lisp-interp eval '(+ 2 3)'   # → 5
lisp-rlm mock deploy/lisp-interp eval '(fib 10)'   # → 55
lisp-rlm mock deploy/lisp-interp eval '(fact 5)'   # → 120
```

The mock uses a 64 MiB wasm stack and has no gas limit, so `fib 27` → 196418 works locally (but not on-chain).

## What This Demonstrates

lisp-rlm compiling a non-trivial program (a full Lisp interpreter) to WebAssembly that runs on NEAR's smart contract runtime. The interpreter is entirely self-hosted — every function in `main.lisp` is written in lisp-rlm's S-expression language and compiled by lisp-rlm's own compiler.
