# lisp-rlm v0.2 — Fix All Five Issues

> **Goal:** Module system, proper errors, pluggable LLM backend, concurrency, split the god function.

---

## 1. Module System

**Problem:** No `(import ...)` / `(require ...)`. Everything in global env.

**Current state:** `require` exists (line 558) but only loads hardcoded stdlib snippets from `get_stdlib_code()`. No file-based loading.

**Design:**
- `(import "path/to/file.lisp")` — parses and evaluates a file, returns its env as a namespace
- `(import "path/to/file.lisp" as foo)` — makes definitions accessible as `(foo/bar ...)`
- `(export ...)` — inside a module, declares which bindings are public (everything public by default)
- Module caching — load each file once, store in a global `HashMap<String, Env>`
- Search path: current dir, then `LISP_RLM_PATH` env var

**Files:**
- New: `src/eval/modules.rs` — `ModuleRegistry` struct with `resolve()`, `load()`, cache
- Modify: `src/eval/mod.rs` — add `"import"` case to special forms, delegate to modules.rs
- Modify: `src/types.rs` — add `LispVal::Namespace(String, Env)` variant (or just use a BTreeMap)

**Tests:** `tests/test_modules.rs`

---

## 2. Error Messages with Source Locations

**Problem:** "not a function", "type error" — no line numbers, no context.

**Current state:** Parser (`src/parser.rs`) tracks line/col in `LispVal` via `Span` info? Let me check.

**Design:**
- Add `span: Option<(usize, usize)>` (line, col) to `LispVal` variants — or wrap in a `Spanned<T>` newtype
- Propagate span through eval — every error becomes `Err(format!("line {}:{} — not a function: {}", line, col, val))`
- Parser already has position info (pest/rowan or hand-written). Wire it through.

**Implementation:**
- Check if parser already tracks position → if yes, just thread it through eval
- If no, add position tracking to parser first
- Replace all `Err(format!("..."))` in dispatch_call with a helper `err(span, msg)` that prepends location
- Add an `EvalError` struct instead of bare `String` — `{ message, span, backtrace }`

**Files:**
- Modify: `src/types.rs` — add span to LispVal or use Spanned wrapper
- Modify: `src/parser.rs` — ensure position tracking
- Modify: `src/eval/mod.rs` — error helper, all Err() sites
- New: `src/eval/errors.rs` — `EvalError` struct, `err()` helper

**Tests:** verify error messages contain "line N" in test expectations

---

## 3. Pluggable LLM Provider

**Problem:** All LLM calls hardcode OpenAI chat/completions format. No way to swap providers.

**Design:**
```rust
trait LlmProvider: Send + Sync {
    fn complete(&self, messages: Vec<(String, String)>) -> Result<LlmResponse, String>;
}

struct LlmResponse {
    content: String,
    tokens: usize,
}
```

- Built-in providers: `OpenAiProvider`, `AnthropicProvider`, `GenericProvider` (any OpenAI-compatible endpoint)
- Provider selected via env var `RLM_PROVIDER` (default: "openai") 
- Config: `RLM_API_KEY`, `RLM_API_BASE`, `RLM_MODEL` already exist — just route them through the trait
- The trait impl handles the HTTP call + response parsing. The builtins in mod.rs just call `provider.complete(messages)`

**Files:**
- New: `src/eval/llm.rs` — `LlmProvider` trait, `OpenAiProvider`, `AnthropicProvider`, provider factory
- Modify: `src/eval/mod.rs` — extract 6 copy-pasted HTTP blocks into calls to `provider.complete()`

**Tests:** `tests/test_llm_provider.rs` — mock provider

---

## 4. Concurrency — Parallel LLM Calls

**Problem:** Single-threaded eval. `llm-batch` fires sequential HTTP calls.

**Design:**
- `SHARED_RUNTIME` already exists. Use `tokio::task::spawn` for parallel sub-calls.
- New builtins:
  - `(parallel (expr1) (expr2) ...)` — evals all expressions concurrently, returns list of results
  - `(llm-batch ...)` — already exists, make it actually parallel
- Implementation: `parallel` spawns each expr eval on the runtime, `join_all`, collect results
- Catch: eval takes `&mut Env` — need `Arc<Mutex<Env>>` or clone env per task

**Approach:** Clone env per parallel branch (same as sub-rlm already does). Merge results back.

**Files:**
- Modify: `src/eval/mod.rs` — add `"parallel"` builtin, fix `llm-batch`
- New: `src/eval/concurrency.rs` — `parallel_eval()` helper

**Tests:** `tests/test_concurrency.rs`

---

## 5. Split the God Function (dispatch_call)

**Problem:** `dispatch_call` is 2,441 lines in one match. Unmaintainable.

**Design:** Extract each category into its own function in its own file under `src/eval/`:

```
src/eval/
├── mod.rs          — lisp_eval, special forms, dispatch_call skeleton (delegates to category fns)
├── arithmetic.rs   — +, -, *, /, %, abs, min, max, ...
├── collections.rs  — length, cons, car, cdr, append, reverse, sort, zip, ...
├── strings.rs      — str-len, str-concat, str-upper, str-split, regex, ...
├── predicates.rs   — null?, list?, number?, string?, eq?, equal?, ...
├── io.rs           — file/read, file/write, file/append, file/exists?, file/list, shell
├── http.rs         — http-get, http-post, http-get-json
├── llm.rs          — llm, llm-code, rlm, sub-rlm, llm-batch, rlm-write (+ LlmProvider trait)
├── crypto.rs       — sha256, keccak256 (already extracted)
├── modules.rs      — import, require, module registry
├── concurrency.rs  — parallel, concurrent llm-batch
├── errors.rs       — EvalError struct, err() helper
├── helpers.rs      — truncate_str, strip_markdown_fences, extract_first_valid_expr
├── quasiquote.rs   — expand_quasiquote
```

Each file exports a `handle_*(name: &str, args: Vec<LispVal>, env: &mut Env) -> Result<LispVal, String>` function.

`dispatch_call` becomes a thin router:
```rust
"sha256" | "keccak256" => crypto::handle_builtin(name, args),
"+" | "-" | "*" | ... => arithmetic::handle(name, args),
"length" | "cons" | ... => collections::handle(name, args),
...
```

**Order matters:** Do this LAST because it's pure refactoring with no behavioral change. Everything else touches the same code — better to extract features first, then reorganize.

---

## Execution Order

1. **Errors + source locations** — foundational, everything benefits
2. **LLM provider trait** — unblocks concurrency, cleans up the biggest copy-paste
3. **Module system** — new feature, mostly additive
4. **Concurrency** — builds on shared runtime + provider trait
5. **Split god function** — pure refactor, do last when all features are in
