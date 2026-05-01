# lisp-rlm

A Lisp runtime that compiles to NEAR WASM smart contracts. Includes a Clojure frontend.

**Write smart contracts in Lisp or Clojure. Run them locally with mock NEAR storage. Compile to WASM for deployment.**

## Quick Start

```bash
cargo build
# Binaries: target/debug/rlm, target/debug/clj-rlm, target/debug/clj-compile

# Run Lisp
./target/debug/rlm -e '(+ 1 2)'
# → 3

# Run Clojure
./target/debug/clj-rlm -e '(-> 5 (+ 3) (* 2))'
# → 16
```

## Features

### Two Languages, One Runtime

| | Lisp (`rlm`) | Clojure (`clj-rlm`) |
|---|---|---|
| Syntax | `(define (f x) ...)` | `(defn f [x] ...)` |
| Lambda | `(lambda (x) ...)` | `(fn [x] ...)` or `#(* % 2)` |
| Let | `(let ((x 1)) ...)` | `(let [x 1] ...)` |
| Threading | — | `(-> 5 (+ 3) (* 2))` |
| Conditionals | `(cond ...)` | `(cond ... :else val)` |
| Maps/Sets | — | `{:a 1}` `#{1 2 3}` |

Both compile to the same bytecode VM. Same performance.

### Mock NEAR Builtins (Local Testing)

Test smart contracts locally with in-memory storage — no deployment needed:

```clojure
;; counter.clj — runs locally, same code compiles to WASM
(defn initialize []
  (storage-write "count" 0)
  (storage-write "owner" (signer-account-id)))

(defn increment []
  (let [c (or (storage-read "count") 0)]
    (storage-write "count" (+ c 1))
    (storage-read "count")))

(defn get-count []
  (or (storage-read "count") 0))

;; Test it
$ clj-rlm counter.clj
$ clj-rlm -e '(initialize) (increment) (increment) (get-count)'
;; → 2
```

**Available mock builtins:**
- `storage-write` / `storage-read` / `storage-remove` / `storage-has-key`
- `signer-account-id` / `predecessor-account-id` / `current-account-id`
- `block-height` / `block-timestamp`
- `account-balance` / `attached-deposit`
- `log` / `log-utf8`
- `near-config` (set mock values) / `near-reset` (clear storage)

### Compile to NEAR WASM

```bash
# Compile Clojure → WASM
$ clj-compile contract.clj -o contract.wasm
Compiled 172 bytes → contract.wasm

# View WAT
$ clj-compile contract.clj --wat

# Deploy
$ near deploy --accountId mycontract.near --wasmFile contract.wasm
```

172 bytes for a basic contract. No near-sdk bloat (35KB+).

### Stack Traces

Errors show the full call chain:

```
Error: division by zero
    (fn [] 42 . .)
    deep1
    deep2
  → deep3            ← crash point
```

### Inline Eval

```bash
$ rlm -e '(define (fib n) (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2))))) (fib 10)'
55

$ clj-rlm -e '(defn sq [x] (* x x)) (map #(sq %) (list 1 2 3 4 5))'
(1 4 9 16 25)
```

### Parse Errors with Location

```
Parse error: unexpected `)` at line 3, col 12
```

## Architecture

```
clojure-rlm/          Clojure frontend (parser + desugar → Lisp AST)
src/                  Core Lisp runtime
  bytecode.rs         Bytecode VM with JIT compilation
  wasm_emit.rs        WASM binary emitter (no WAT strings)
  program.rs          Top-level eval pipeline
  parser.rs           Lisp parser with line/col tracking
  types.rs            Types + EvalState (storage, trace, context)
  eval/               LLM dispatch, JSON, HTTP builtins
```

The VM compiles Lisp to bytecode at define-time. Hot loops use a separate loop compiler with fused ops (e.g., `SlotAddImm`, `RecurIncAccum`). Functions get inlined when small.

The WASM emitter produces binary WASM directly via `wasm-encoder` — no string-based WAT, impossible to emit invalid modules.

## Performance

- 1M tail-recursive iterations in a single view call
- 10M while-loop iterations
- 903 bytes vs near-sdk's 35KB for equivalent contracts
- Bytecode JIT: `map`/`filter`/`reduce` compile to tight loops

## Clojure Frontend (`clojure-rlm/`)

Thin layer over the Lisp VM:

| Feature | Status |
|---------|--------|
| `defn` / `fn` with `[]` params | ✅ |
| `#(* % %2)` anonymous functions | ✅ |
| `->` / `->>` threading macros | ✅ |
| `let` with `[]` bindings | ✅ |
| `when` / `cond` / `if-not` | ✅ |
| `:keywords` | ✅ |
| `[vec]` / `{map}` / `#{set}` literals | ✅ |
| File execution + REPL | ✅ |
| `-e` inline eval | ✅ |
| WASM compilation (`clj-compile`) | ✅ |

## Build

```bash
git clone https://github.com/Kampouse/lisp-rlm.git
cd lisp-rlm
cargo build
```

Binaries:
- `rlm` — Lisp runner (REPL + file + `-e`)
- `clj-rlm` — Clojure runner
- `clj-compile` — Clojure → WASM compiler
- `near-compile` — Lisp → WASM compiler

## License

MIT
