# lisp-rlm

A **Recursive Language Model** runtime — a Lisp interpreter where the LLM writes Lisp code, the runtime executes it, and the loop continues until the task is done.

Based on the concepts from ["Recursive Language Models"](https://arxiv.org/html/2512.24601v2) (Zhang, Kraska, Khattab — MIT 2025).

Forked from [near-lisp](https://github.com/Jemartel/near-lisp) (NEAR Protocol on-chain Lisp smart contract). Gas system, chain dependencies, and all NEAR-specific code removed. Replaced with file I/O, HTTP, LLM integration, and a runtime execution budget.

---

## Quick Start

```bash
git clone https://github.com/Kampouse/lisp-rlm.git
cd lisp-rlm
cargo run --bin rlm
```

```
rlm> (+ 1 2 3)
6
rlm> (define (square x) (* x x))
rlm> (square 7)
49
rlm> (map square (list 1 2 3 4 5))
(1 4 9 16 25)
rlm> 'hello
hello
```

### Run a script

```bash
cargo run --bin rlm -- script.lisp
```

### With LLM (set API key)

```bash
export GLM_API_KEY=your_key_here    # or OPENAI_API_KEY
cargo run --bin rlm
```

```lisp
rlm> (llm "What is 2+2?")
4

rlm> (llm-code "Write a function that computes fibonacci of 10")
```

---

## Language Features

lisp-rlm implements a large subset of **R7RS Scheme** plus extensions for LLM integration, file I/O, HTTP, JSON, and runtime state management.

### R7RS Conformance

**433/515 tests passing** (84%) against the official chibi-scheme R7RS test suite.

| Category | Status |
|----------|--------|
| Special forms (if, cond, let, let\*, letrec, case, when, unless, do, begin, set!) | ✅ |
| Quote shorthand (`'x`) | ✅ |
| lambda, closures, recursion | ✅ |
| case-lambda (arg-count dispatch) | ✅ |
| Macros (defmacro, quasiquote) | ✅ |
| define-values, let-values, let\*-values | ✅ |
| delay/force (lazy evaluation) | ✅ |
| cond `=>` arrow syntax | ✅ |
| Arithmetic (incl. expt, atan, sin, cos, tan, log, truncate/) | ✅ |
| String operations (30+ builtins, case-insensitive variants) | ✅ |
| Character operations (literals, predicates, comparisons, case) | ✅ |
| Type predicates (number types, finite?, infinite?, nan?) | ✅ |
| Fraction literals (3/4 → 0.75) | ✅ |
| Float literals (+nan.0, +inf.0, -inf.0) | ✅ |
| Multiple values (values, call-with-values) | ✅ |
| Not yet: syntax-rules, vectors, complex numbers, bignums, full char+string libraries | — |

### Special Forms

| Form | Description |
|------|-------------|
| `(define name expr)` | Bind a value |
| `(define (f x) body)` | Function shorthand |
| `(lambda (params...) body)` | Anonymous function |
| `(case-lambda (() e0) ((x) e1) (args e2))` | Dispatch by arg count |
| `(if cond then else?)` | Conditional |
| `(cond (t1 v1) (t2 => proc) ...)` | Multi-branch with `=>` support |
| `(case key ((d1 d2) body) ...)` | Value dispatch |
| `(let ((v1 e1) ...) body)` | Parallel local bindings |
| `(let* ((v1 e1) ...) body)` | Sequential local bindings |
| `(letrec ((f1 e1) ...) body)` | Recursive local bindings |
| `(let-values (((a b) expr)) body)` | Destructure multiple values |
| `(define-values (a b c) expr)` | Destructure at top level |
| `(when test body...)` | Conditional execution |
| `(unless test body...)` | Inverse conditional |
| `(do ((v i step) ...) (test result...) body)` | Imperative loop |
| `(begin e1 e2 ...)` | Sequential evaluation |
| `(loop ((v1 i1) ...) body)` | Named let with `(recur ...)` |
| `(set! var expr)` | Mutate existing binding |
| `(quote x)` / `'x` | Prevent evaluation |
| `(try expr (catch e body))` | Error handling |
| `(defmacro name (ps...) body)` | Define a macro |
| `(delay expr)` | Create a promise (lazy) |
| `(force promise)` | Evaluate a promise |

### Builtins

**Arithmetic:** `+`, `-`, `*`, `/`, `mod`, `abs`, `min`, `max`, `sqrt`, `floor`, `ceiling`, `round`, `expt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `log`, `truncate`, `truncate/`, `floor/`, `exact-integer-sqrt`

**Comparison:** `=`, `<`, `>`, `<=`, `>=`, `not`, `equal?`, `eqv?`

**Logic:** `and`, `or`

**Lists:** `list`, `cons`, `car`, `cdr`, `nth`, `len`, `append`, `reverse`, `map`, `filter`, `reduce`, `sort`, `range`, `member`, `assoc`, `for-each`, `list-copy`, `list-tail`, `make-list`, `cadr`

**Strings (30+):** `str-concat`, `str-split`, `str-contains`, `str-trim`, `str-upcase`, `str-downcase`, `str-length`, `str-substring`, `str-index-of`, `str-starts-with`, `str-ends-with`, `string=?`, `string<?`, `string>?`, `string<=?`, `string>=?`, `string-ci=?`, `string-ci<?`, `string-ci>?`, `string-ci<=?`, `string-ci>=?`, `string-foldcase`, `make-string`, `string-ref`

**Characters:** `char?`, `char=?`, `char<?`, `char>?`, `char<=?`, `char>=?`, `char-ci=?` (and all variants), `char-alphabetic?`, `char-numeric?`, `char-whitespace?`, `char-upper-case?`, `char-lower-case?`, `char-upcase`, `char-downcase`, `char-foldcase`, `char->integer`, `integer->char`, `digit-value`

**Type predicates:** `nil?`, `bool?`, `list?`, `cons?`, `number?`, `string?`, `boolean=?`, `symbol?`, `procedure?`, `map?`, `integer?`, `rational?`, `real?`, `complex?`, `exact?`, `inexact?`, `exact-integer?`, `finite?`, `infinite?`, `nan?`, `promise?`

**I/O:** `print`, `println`, `read-file`, `write-file`, `append-file`, `file-exists?`, `load-file`, `shell`

**HTTP:** `http-get`, `http-post`, `http-get-json`

**JSON:** `json-parse`, `json-get`, `json-get-in`, `json-build`, `from-json`, `to-json`

**Conversions:** `to-string`, `to-int`, `to-float`, `to-num`, `string->number`, `number->string`

**Parsing:** `read` (parse string → LispVal), `read-all` (parse all expressions → list)

**Multiple values:** `values`, `call-with-values`

**Crypto:** `sha256`, `keccak256`

---

## RLM Built-in Agent Loop

The built-in RLM system implements Algorithm 1 from the [MIT RLM paper](https://arxiv.org/html/2512.24601v2):

```
┌─────────┐     ┌──────────┐     ┌─────────┐     ┌──────────┐
│  Task    │────▶│  Build    │────▶│  LLM    │────▶│  Parse   │
│  Prompt  │     │  Context  │     │  Call   │     │  Code    │
└─────────┘     └──────────┘     └─────────┘     └────┬─────┘
                                                         │
                     ┌──────────┐                         │
                     │  Update  │◀────────────────────────┤
                     │  State   │                         │
                     └────┬─────┘                         │
                          │                               │
                    ┌─────▼──────┐     ┌───────────┐     │
                    │  Final?    │─YES▶│  Return   │     │
                    └─────┬──────┘     │  Result   │     │
                          │ NO         └───────────┘     │
                          │                               │
                     ┌────▼─────┐     ┌──────────┐       │
                     │ Snapshot  │────▶│  Eval    │───────┘
                     │ Env      │     │  Code    │  (retry on error
                     └──────────┘     └──────────┘   with rollback)
```

### Core Agent Loop

| Builtin | Description |
|---------|-------------|
| `(rlm "task")` | Full agent loop — calls LLM, parses code, evals with retry, returns result |
| `(rlm-write "task")` | Generate Lisp code, return as string |
| `(rlm-write "task" "path")` | Generate code, save to file, return as string |
| `(llm "prompt")` | Call LLM, return text response |
| `(llm-code "prompt")` | Call LLM, parse response as Lisp, eval it |

### State Management

| Builtin | Description |
|---------|-------------|
| `(rlm-set key value)` | Store state |
| `(rlm-get key)` | Retrieve state |
| `(rlm-tokens)` | Total LLM tokens used |
| `(rlm-calls)` | Number of LLM API calls made |

### Snapshot & Rollback

| Builtin | Description |
|---------|-------------|
| `(snapshot)` | Save current environment state |
| `(rollback)` | Restore to last snapshot |
| `(rollback-to n)` | Restore to specific snapshot |

### Sub-RLM

| Builtin | Description |
|---------|-------------|
| `(sub-rlm "sub-task")` | Spawn a sub-computation with isolated state (max depth 5) |

---

## Example: Self-Implementing RLM

```lisp
;; The RLM read the MIT paper and generated its own implementation
(load-file "/tmp/rlm_algo1.lisp")
(run-rlm "What is 2 plus 2?")
;; → 4
```

## Example: Generate & Run Programs

```lisp
(rlm-write "Write a Lisp program with sum-list and average functions, then test them" "/tmp/stats.lisp")
(load-file "/tmp/stats.lisp")
;; → Testing sum-list:
;; →   sum of (1 2 3 4 5) = 15
;; → Testing average:
;; →   average of (1 2 3 4 5) = 3
;; → All tests passed!
```

---

## Architecture

The runtime uses **CPS (Continuation-Passing Style)** evaluation with a trampoline loop — no recursive `eval_step` calls, so deep recursion and infinite loops are caught by the execution budget instead of causing stack overflows.

```
src/
├── eval/
│   ├── mod.rs              Eval engine, dispatch, LLM/RLM integration
│   ├── cps_eval.rs         CPS evaluator — special forms, continuation handling
│   ├── continuation.rs     Step/Cont enums for the trampoline
│   ├── bytecode.rs         Bytecode compiler + stack-based VM
│   ├── dispatch_arithmetic.rs
│   ├── dispatch_collections.rs
│   ├── dispatch_strings.rs
│   ├── dispatch_predicates.rs
│   ├── dispatch_types.rs
│   ├── dispatch_json.rs
│   ├── dispatch_http.rs
│   └── dispatch_state.rs
├── parser.rs               S-expression parser with quote/char/fraction literals
├── types.rs                LispVal enum, Env, EvalState
├── helpers.rs              Utility functions, builtin dispatch table
├── lib.rs                  Public API re-exports
└── bin/
    └── rlm.rs              REPL + file execution
```

### Key design decisions

- **CPS trampoline** — all special forms return `Step` values, never recurse. Function arg evaluation uses `ArgCollect` continuation. No stack overflows.
- **Eval budget** — 1M iterations default. Catches infinite loops.
- **Persistent data structures** — `im::HashMap` for environments. Snapshot/rollback is O(1).
- **Env isolation** — function arg evaluation saves/restores env snapshot to prevent cross-contamination between args.

---

## Testing

```bash
cargo test                                          # 288/290 tests passing
cargo test --test core_language                     # 160 tests
cargo test --test test_macros                       # 31 tests
cargo test --test test_stdlib_tier1                 # 26 tests
cargo test --test test_lambda_hof                   # 8 tests
cargo test --test test_types                        # 18 tests
cargo test --test norvig_tests                      # 1 test (Norvig's Lis.py suite)
cargo test --test test_harness                      # 3 tests
```

2 tests require LLM API keys (not code bugs).

---

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `GLM_API_KEY` or `OPENAI_API_KEY` | (required for LLM) | LLM API key |
| `RLM_API_BASE` | `https://api.z.ai/api/coding/paas/v4` | API endpoint |
| `RLM_MODEL` | `glm-5.1` | Model name |
| `RLM_MAX_ITERATIONS` | `10` | Max iterations for `(rlm ...)` loop |
| `RLM_VERIFY` | (off) | Set to `1` to enable self-verification |
| `RLM_ALLOW_SHELL` | (off) | Set to `1` to enable `(shell ...)` |

---

## References

- **Recursive Language Models** — Alex L. Zhang, Tim Kraska, Omar Khattab (MIT, 2025)
  [arXiv:2512.24601v2](https://arxiv.org/html/2512.24601v2)

- **near-lisp** — [github.com/Jemartel/near-lisp](https://github.com/Jemartel/near-lisp)

---

## License

MIT
