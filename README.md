# lisp-rlm

Standalone Lisp runtime for **Recursive Language Model (RLM)** orchestration — a Lisp interpreter where code is data, macros enable emergent self-harnessing, and the bytecode VM makes it fast enough for real-time LLM tool-use loops.

Forked from [near-lisp](https://github.com/Jemartel/near-lisp) (NEAR Protocol on-chain Lisp smart contract). Gas system, chain dependencies, and all NEAR-specific code removed. Replaced with a native binary, file I/O, HTTP support, and a runtime execution budget.

---

## Quick Start

```bash
git clone https://github.com/Kampouse/lisp-rlm.git
cd lisp-rlm
cargo run --bin rlm
```

```
rlm> (+ 1 2)
3
rlm> (define square (lambda (x) (* x x)))
rlm> (square 7)
49
rlm> (map square (list 1 2 3 4 5))
(1 4 9 16 25)
```

---

## Architecture

```
src/
├── eval.rs       Main eval engine — special forms, builtins, macro expansion,
│                 dispatch, map/filter fast path, budget enforcement
├── bytecode.rs   Bytecode compiler + stack-based VM — compiles loops and
│                 simple lambdas to skip per-element env overhead
├── parser.rs     S-expression parser (lists, atoms, strings, maps, quotes)
├── types.rs      LispVal enum, Env (scoped bindings + execution budget),
│                 stdlib modules (math, list, string, crypto)
├── helpers.rs    Utility functions — parse_params, is_builtin_name, type predicates
├── lib.rs        Public API re-exports
└── bin/
    └── rlm.rs    REPL with rustyline, stdlib auto-loading
```

---

## Language Reference

### Types

| Type | Example | Notes |
|------|---------|-------|
| Nil | `nil` | Null/empty |
| Bool | `true`, `false` | |
| Num | `42`, `-7` | i64 integer |
| Float | `3.14`, `-0.5` | f64 |
| Str | `"hello"` | Double-quoted |
| Sym | `foo`, `+` | Symbols / identifiers |
| List | `(1 2 3)` | Heterogeneous |
| Map | `{"a" 1 "b" 2}` | String keys |
| Lambda | `(lambda (x) (* x x))` | First-class functions |
| Macro | `(defmacro ...)`, | Code-as-data transforms |

### Special Forms

| Form | Description |
|------|-------------|
| `(define name expr)` | Bind a value in the current scope |
| `(lambda (params...) body)` | Create a function |
| `(if cond then else?)` | Conditional |
| `(cond (test1 val1) (test2 val2) ...)` | Multi-branch conditional |
| `(let ((var1 val1) ...) body)` | Local bindings |
| `(begin expr1 expr2 ...)` / `(progn ...)` | Sequential evaluation |
| `(loop ((var1 init1) ...) body)` | Named let loop with `(recur ...)` |
| `(recur val1 val2 ...)` | Re-enter loop with new bindings |
| `(set! var expr)` | Mutate an existing binding |
| `(quote expr)` / `'expr` | Prevent evaluation |
| `(match expr (pattern1 body1) ...)` | Pattern matching |
| `(try expr (catch error-var body))` | Error handling |
| `(defmacro name (params...) body)` | Define a macro |
| `(quasiquote expr)` | Template with `(unquote ...)` and `(unquote-splicing ...)` |

### Builtins

**Arithmetic:** `+`, `-`, `*`, `/`, `mod`, `abs`, `min`, `max`

**Comparison:** `=`, `!=`, `<`, `>`, `<=`, `>=`

**Boolean:** `and`, `or`, `not`

**List ops:** `car`, `cdr`, `cons`, `list`, `len`, `append`, `nth`, `reverse`, `sort`, `range`, `zip`

**Higher-order:** `map`, `filter`, `reduce`, `find`, `some`, `every`

**String ops:** `str-concat`, `str-length`, `str-split`, `str-contains`, `str-substring`, `str-index-of`, `str-trim`, `str-upcase`, `str-downcase`, `str-starts-with`, `str-ends-with`

**Type predicates:** `nil?`, `list?`, `number?`, `string?`, `boolean?`, `symbol?`, `integer?`, `float?`

**Type conversions:** `to-string`, `to-int`, `to-float`, `to-num`, `to-json`, `from-json`

**JSON:** `json-parse`, `json-build`, `json-get`, `json-get-in`

**Hashing:** `sha256`, `keccak256`

**I/O:** `file/read`, `file/write`, `file/exists?`, `file/list`

**Debugging:** `print`, `println`, `debug`, `inspect`, `trace`

**Other:** `dict`, `error`, `require`, `match`, `try`/`catch`

### Stdlib Modules

Loaded via `require` or at REPL startup:

- **math** — `abs`, `min`, `max`, `even?`, `odd?`, `gcd`, `square`, `identity`, `pow`, `sqrt`, `lcm`
- **list** — `empty?`, `map`, `filter`, `reduce`, `find`, `some`, `every`, `reverse`, `sort`, `range`, `zip`
- **string** — `str-join`, `str-replace`, `str-repeat`, `str-pad-left`, `str-pad-right`
- **crypto** — `hash/sha256-bytes`, `hash/keccak256-bytes`

---

## RLM-Specific Features

These builtins make lisp-rlm an orchestration layer for LLM workflows:

### `rlm/signature`

Creates a typed signature — describes what inputs and outputs a step expects.

```lisp
(rlm/signature "translate" (list "text" "target_lang") (list "translated_text"))
```

### `rlm/format-prompt`

Formats a signature into a structured prompt for an LLM.

```lisp
(rlm/format-prompt sig)
;; => "Inputs:\n- text\n- target_lang\nOutputs:\n- translated_text\n"
```

### `rlm/trace`

Appends trace entries to `__rlm_trace__` in the environment — logs execution flow.

### `rlm/config`

Stores key-value configuration in the environment under `__rlm_<key>__`.

---

## Bytecode & Performance

The bytecode compiler translates loop bodies and simple lambdas into stack-based bytecode, skipping the per-element env overhead of the tree-walking evaluator.

**Fast path:** `map`, `filter`, and `reduce` automatically use the bytecode VM when:
1. The function is a `Lambda` (not a builtin or macro)
2. It has exactly 1 parameter (for map/filter) or 2 (for reduce)
3. The body compiles successfully

If compilation fails (e.g. macro-generated code), it falls back to the eval path — zero correctness risk.

**Bytecode operations:** 29 ops including arithmetic, comparison, list ops, string ops, and control flow. Unknown builtins trigger graceful fallback.

---

## Execution Budget

Gas was removed (it's a standalone runtime, not on-chain). Instead, an iteration counter in `Env` prevents runaway infinite loops:

```rust
let mut env = Env::new();           // budget = 10,000,000 by default
env.eval_budget = 1_000_000;        // or set your own
env.eval_budget = 0;                // 0 = unlimited
```

Every `lisp_eval()` call increments `env.eval_count`. When it exceeds `env.eval_budget`, evaluation aborts with:

```
ERROR: execution budget exceeded: 1000001 iterations (limit: 1000000)
```

This catches:
- Infinite tail recursion (`(define f (lambda () (f)))`)
- Mutual recursion loops (`f` calls `g` calls `f`)
- Runaway loops outside `loop/recur`

Stack overflow from deep recursion is caught by `stacker::maybe_grow()` — a separate, complementary mechanism.

---

## Macro System

lisp-rlm has a full macro system — the key differentiator for the thesis. Because Lisp is homoiconic (code IS data), macros let the runtime generate new control flow and abstractions at runtime:

```lisp
(defmacro when (cond body)
  (quasiquote (if (unquote cond) (unquote body) nil)))

(when (> 5 3) (print "yes"))
;; expands to: (if (> 5 3) (print "yes") nil)
```

This enables **emergent self-harnessing** — the model writes new macros and control structures at runtime, composing behaviors that no static tool system can match. This is the conceptual bridge between predict-rlm, nullclaw, and the NEAR Lisp VM.

---

## Testing

```bash
cargo test                    # All 235 tests
cargo test --test core        # 160 core language tests
cargo test --test macros      # 31 macro tests
cargo test --test budget      # 12 execution budget tests
cargo test --test fast_path   # 14 bytecode fast path tests
cargo test --test loop        # 11 loop/recur tests
```

---

## Comparison with near-lisp

| Feature | lisp-rlm | near-lisp |
|---------|----------|-----------|
| **Purpose** | LLM orchestration runtime | On-chain smart contract |
| **Runtime** | Native binary + REPL | NEAR wasm cdylib |
| **Gas** | Removed — execution budget instead | Full gas tracking |
| **Chain deps** | None | near-sdk, borsh |
| **File I/O** | file/read, file/write, file/exists?, file/list | None |
| **HTTP** | reqwest + tokio | None |
| **Macros** | defmacro, quasiquote, unquote, unquote-splicing | Not included |
| **Bytecode** | 29 ops + map/filter fast path | 18 ops |
| **RLM builtins** | rlm/signature, rlm/format-prompt, rlm/trace, rlm/config | None |
| **Cross-contract calls** | None | yield/resume via vm.rs |
| **Chain state** | None | near/* builtins (block-height, signer, etc.) |
| **Crypto** | sha256, keccak256 | sha256, keccak256, ed25519-verify, ecrecover |
| **LOC** | ~5,500 | ~12,800 |
| **Tests** | 235 | 390+ |

---

## License

MIT
