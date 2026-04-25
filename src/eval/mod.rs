use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::helpers::*;
use crate::parser::parse_all;
use crate::types::{Env, EvalState, LispVal};
use continuation::{Cont, EvalResult, Step};
use cps_eval::{catch_error, eval_step, handle_cont};
pub mod crypto;
pub mod errors;
pub mod helpers;
pub mod llm_provider;
pub mod quasiquote;

// Tail-call elimination
pub mod continuation;
pub mod cps_eval;

// Domain-specific dispatch modules (v0.2 god-function split)
pub mod dispatch_arithmetic;
pub mod dispatch_collections;
pub mod dispatch_http;
pub mod dispatch_json;
pub mod dispatch_predicates;
pub mod dispatch_state;
pub mod dispatch_strings;
pub mod dispatch_types;

pub use llm_provider::*;

use crypto::{builtin_keccak256, builtin_sha256};
use helpers::{strip_markdown_fences, truncate_str};
use quasiquote::expand_quasiquote;

// ---------------------------------------------------------------------------
// JSON conversion
// ---------------------------------------------------------------------------

/// Convert a [`serde_json::Value`] into a [`LispVal`].
///
/// Mapping:
/// - `Null` → `Nil`
/// - `Bool` → `Bool`
/// - `Number` → `Num(i64)` or `Float(f64)`
/// - `String` → `Str`
/// - `Array` → `List`
/// - `Object` → `Map`
pub fn json_to_lisp(val: serde_json::Value) -> LispVal {
    match val {
        serde_json::Value::Null => LispVal::Nil,
        serde_json::Value::Bool(b) => LispVal::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                LispVal::Num(i)
            } else {
                // as_f64() always succeeds for JSON numbers, but use it as fallback
                // for values that don't fit in i64 (e.g. large u64)
                LispVal::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => LispVal::Str(s),
        serde_json::Value::Array(a) => LispVal::List(a.into_iter().map(json_to_lisp).collect()),
        serde_json::Value::Object(m) => {
            let map: im::HashMap<String, LispVal> =
                m.into_iter().map(|(k, v)| (k, json_to_lisp(v))).collect();
            LispVal::Map(map)
        }
    }
}

/// Convert a [`LispVal`] reference into a [`serde_json::Value`].
///
/// Mapping is the inverse of [`json_to_lisp`]:
/// - `Nil` → `Null`
/// - `Bool` → `Bool`
/// - `Num` → integer `Number`
/// - `Float` → float `Number` (non-finite values become `Null`)
/// - `Str` → `String`
/// - `List` → `Array`
/// - `Map` → `Object`
/// - All other variants (`Sym`, `Lambda`, `Macro`, `Recur`) → `String` (via [`Display`])
pub fn lisp_to_json(val: &LispVal) -> serde_json::Value {
    match val {
        LispVal::Nil => serde_json::Value::Null,
        LispVal::Bool(b) => serde_json::Value::Bool(*b),
        LispVal::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
        LispVal::Float(f) => {
            if let Some(n) = serde_json::Number::from_f64(*f) {
                serde_json::Value::Number(n)
            } else {
                serde_json::Value::Null
            }
        }
        LispVal::Str(s) => serde_json::Value::String(s.clone()),
        LispVal::List(items) => serde_json::Value::Array(items.iter().map(lisp_to_json).collect()),
        LispVal::Map(m) => {
            let obj: serde_json::Map<String, serde_json::Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), lisp_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        other => serde_json::Value::String(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// RLM System Prompt (aligned with MIT paper's reference implementation)
// ---------------------------------------------------------------------------

const RLM_SYSTEM_PROMPT: &str = r#"You are an autonomous agent with access to a Lisp REPL environment. Given a task, you write and execute Lisp code iteratively to accomplish it. You will be queried iteratively until you provide a final answer via (final "answer") or (final-var varname).

## REPL Environment

The REPL is initialized with:
1. A `context` variable containing important information about your query. Check it with (rlm-get context) or look at (show-vars).
2. `(llm "prompt")` — a single LLM completion call (no REPL, no iteration). Fast and lightweight. Use for simple extraction, summarization, or Q&A. The sub-LLM can handle large inputs.
3. `(llm-code "prompt")` — like llm but returns parsed/evaluated Lisp code.
4. `(sub-rlm "prompt")` — spawns a recursive RLM sub-call for deeper thinking subtasks. The child gets its own REPL and can iterate, just like you. Use when a subtask requires multi-step reasoning or its own iterative problem-solving — not just a simple one-shot answer.
5. `(llm-batch (list "prompt1" "prompt2" ...))` — runs multiple llm calls sequentially, returns a list of responses. Use for independent queries over different chunks.
6. `(show-vars)` — returns a string listing all variables in the current environment with their types.
7. `(show-context)` — returns metadata about the task context: prompt length, preview, iteration count, whether Final is set.
8. `(str-chunk str n)` — splits a string into n roughly equal chunks. Returns a list of strings.

## When to use llm vs sub-rlm

- Use `(llm "prompt")` for simple, one-shot tasks: extracting info from a chunk, summarizing text, answering a factual question, classifying content.
- Use `(sub-rlm "prompt")` when the subtask itself requires deeper thinking: multi-step reasoning, solving a sub-problem that needs its own REPL and iteration, or tasks where a single LLM call might not be enough.

## Breaking Down Problems

You MUST break problems into digestible components — whether that means chunking or summarizing a large context, or decomposing a hard task into easier sub-problems and delegating them via llm/sub-rlm. Write a PROGRAMMATIC STRATEGY that uses these LLM calls to solve the problem, as if you were building an agent: plan steps, branch on results, combine answers in code.

## Chunking Strategy

For large contexts, use str-chunk to split the input, then process per-chunk with llm or llm-batch:

```lisp
;; Split context into 5 chunks and process each
(define chunks (str-chunk context 5))
(define answers (map (lambda (chunk)
  (llm (str-concat "Extract relevant info from this text: " chunk)))
  chunks))
;; Aggregate answers
(define final-answer (llm (str-concat "Based on these partial answers, respond to the query: "
  (str-join "\n" answers))))
(final final-answer)
```

For batch processing (faster for independent queries):
```lisp
(define chunks (str-chunk context 10))
(define prompts (map (lambda (chunk)
  (str-concat "Answer the query based on this chunk. Only answer if confident: " chunk))
  chunks))
(define answers (llm-batch prompts))
```

## Iterative Book Analysis Pattern

```lisp
(define query "Did the protagonist win?")
(define chunks (str-chunk context 10))
(define buffers (list))

(loop ((i 0))
  (if (>= i (len chunks))
    (final (llm (str-concat "Based on gathered info: " (str-join "\n" buffers) "\nAnswer: " query)))
    (do
      (define chunk (nth i chunks))
      (define result (llm (str-concat "Gather info to answer: " query "\nText: " chunk)))
      (define buffers (append buffers (list result)))
      (recur (+ i 1)))))
```

## Using sub-rlm for Complex Sub-problems

```lisp
;; Child RLM solves a sub-problem in its own REPL
(define trend (sub-rlm "Analyze this dataset and conclude: up, down, or stable"))
(define recommendation
  (if (str-contains (str-downcase trend) "up")
    "Consider increasing exposure."
    (if (str-contains (str-downcase trend) "down")
      "Consider hedging."
      "Hold position.")))
(final (llm (str-concat "Given trend=" trend " and recommendation=" recommendation ", summarize.")))
```

## Providing Your Answer

When done, use one of:
- `(final "your answer string")` — provide the answer directly
- `(final-var myvar)` — return a variable you created in the REPL as the final answer

IMPORTANT: Create and assign the variable FIRST in code, then call final-var in a SEPARATE expression. If unsure what variables exist, use (show-vars).

## Key Rules

- Your outputs are TRUNCATED in the conversation history. Only constant-size metadata is kept. Store important data in variables — do not rely on seeing previous output.
- Use (show-vars) to check what state exists before referencing variables.
- Use (show-context) to understand the task before diving in.
- Think step by step. Plan, then execute. Output code and use sub-LLMs as much as possible.
- Do NOT provide a final answer on the first iteration — first explore the context and plan your approach.

## Available Builtins

Arithmetic: + - * / mod abs min max floor ceiling round sqrt number->string
Comparison: = < > <= >= not
Logic: and or
Numeric predicates: zero? positive? negative? even? odd?
Lists: list cons car cdr nth len append reverse map filter reduce sort range zip find some every member assoc partition fold-left fold-right for-each cons*
Predicates: nil? list? number? string? bool? map? macro? type? empty? procedure? symbol?
Equivalence: equal? eq? symbol=?
Conversion: symbol->string string->symbol to-int to-float to-string to-num
Strings: str-concat str-contains str-split str-split-exact str-trim str-upcase str-downcase str-length str-substring str-index-of str-starts-with str-ends-with str= str!= str-chunk str-join string->list list->string string<? string->number
Control: apply eval

IO — file and shell:
  (read-file "path.txt")           → string contents
  (write-file "path.txt" content)  → writes string to file
  (append-file "path.txt" content) → appends string to file
  (file-exists? "path.txt")        → bool
  (delete-file "path.txt")         → bool, deletes file
  (shell "ls -la")                 → stdout string (requires RLM_ALLOW_SHELL=1)
  (shell-bg "python3 server.py")   → spawn background process, returns PID (requires RLM_ALLOW_SHELL=1)
  (shell-kill pid)                 → kill background process by PID

HTTP — make HTTP requests:
  (http-get "https://example.com")                     → response body as string
  (http-post "https://api.com" "{\"key\":\"val\"}")    → POST with JSON body, returns response body
  (http-get-json "https://api.com/data")               → returns parsed JSON as Lisp map/list

JSON:
  (from-json "{\"a\":1}")      → Lisp map
  (to-json val)                → JSON string
  (json-get obj "key")         → value from map
  (json-get-in obj (list "a" "b"))  → nested access
  (json-build "key" val ...)   → build JSON object

LLM: llm llm-code sub-rlm llm-batch
Crypto: sha256 keccak256
Types: to-int to-float to-string to-num
State: rlm-set rlm-get
Final: final final-var
Introspection: show-vars show-context
Token tracking: rlm-tokens rlm-calls
Snapshot: snapshot rollback rollback-to
Fork: (fork expr) — evaluate expr in isolated env fork, parent unchanged. O(1) via persistent data structures. Use for speculative execution.
Types: type-of check check! matches? valid-type?
  Type primitives: :nil :bool :int :float :num :str :sym :list :map :fn :any
  Compound: (:list :int) (:map :str :int) (:tuple :int :str) (:or :int :nil)
  (check value :type) → value or error. (matches? value :type) → bool.
Contract: (contract ((x :int y :str) -> :bool) body) — runtime type-checked function.
Schema: (defschema :user "name" :str "age" :int "tags" (:list :str) :strict) then (validate data :user).
Special forms: define def let lambda if cond match quote quasiquote unquote unquote-splicing loop recur begin progn defmacro require try catch error fork contract"#;

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

/// Evaluate a single Lisp expression in the given environment.
///
/// This is the main entry point for the tree-walking evaluator.  It handles
/// all special forms (`quote`, `if`, `define`, `lambda`, `defmacro`, `let`,
/// `loop`, `recur`, `match`, `try`, `cond`, `progn`/`begin`, `and`, `or`,
/// `not`, `require`, `quasiquote`) and dispatches everything else to the
/// function-call machinery ([`apply_lambda`] / builtins).
///
/// # Execution budget
///
/// Each call increments `state.eval_count` and checks it against
/// `state.eval_budget`.  When the budget is exceeded an `Err` is returned.  A
/// budget of `0` disables the limit.
///
/// # Stack safety
///
/// The call is wrapped in [`stacker::maybe_grow`] so deeply-recursive
/// evaluation does not overflow the native stack.
///
/// # Errors
///
/// Returns `Err(String)` for:
/// - undefined symbols
/// - arity mismatches
/// - type errors in builtins
/// - execution budget exceeded
/// - errors propagated from user code (`(error ...)`)
pub fn lisp_eval(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    let mut stack: Vec<Cont> = Vec::new();
    let mut current = expr.clone();

    'main: loop {
        if state.eval_budget > 0 {
            state.eval_count += 1;
            if state.eval_count > state.eval_budget {
                return Err(format!(
                    "execution budget exceeded: {} iterations (limit: {})",
                    state.eval_count, state.eval_budget
                ));
            }
        }

        // Evaluate current expression
        let step = match eval_step(&current, env, state) {
            Ok(s) => s,
            Err(e) => match catch_error(&mut stack, e, env, state)? {
                Step::Done(v) => return Ok(v),
                Step::EvalNext {
                    expr,
                    conts,
                    new_env,
                } => {
                    stack.extend(conts);
                    current = expr;
                    if let Some(ne) = new_env {
                        *env = ne;
                    }
                    continue 'main;
                }
            },
        };

        match step {
            Step::Done(mut val) => {
                // Unwind continuation stack
                'unwind: loop {
                    match stack.pop() {
                        None => return Ok(val),
                        Some(cont) => {
                            let step = match handle_cont(cont, val, env, state) {
                                Ok(s) => s,
                                Err(e) => match catch_error(&mut stack, e, env, state)? {
                                    Step::Done(v) => {
                                        val = v;
                                        continue 'unwind;
                                    }
                                    Step::EvalNext {
                                        expr,
                                        conts,
                                        new_env,
                                    } => {
                                        stack.extend(conts);
                                        current = expr;
                                        if let Some(ne) = new_env {
                                            *env = ne;
                                        }
                                        continue 'main;
                                    }
                                },
                            };
                            match step {
                                Step::Done(v) => {
                                    val = v;
                                }
                                Step::EvalNext {
                                    expr,
                                    conts,
                                    new_env,
                                } => {
                                    stack.extend(conts);
                                    current = expr;
                                    if let Some(ne) = new_env {
                                        *env = ne;
                                    }
                                    continue 'main;
                                }
                            }
                        }
                    }
                }
            }
            Step::EvalNext {
                expr,
                conts,
                new_env,
            } => {
                stack.extend(conts);
                current = expr;
                if let Some(ne) = new_env {
                    *env = ne;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fractal RLM: try-solve → fail → binary split → recurse
// ---------------------------------------------------------------------------
//
// Each node:
//   1. TRY to solve the task in one LLM call (generate + eval Lisp code)
//   2. If solved (code runs + sets (final ...)) → BLACK node, return result
//   3. If failed (parse/eval error, or didn't call final) → RED node, decompose:
//      a. Ask LLM to split task into exactly 2 subtasks
//      b. Recurse on each (DFS: left first)
//      c. Synthesize children's results into final answer
//
// Properties:
//   - O(1) context per node (fresh messages, no history inheritance)
//   - O(log n) depth via max_depth guard
//   - "No two reds in a row": children always try-solve first
//   - Token budget per node prevents runaway
// ---------------------------------------------------------------------------

fn rlm_fractal(
    task: String,
    env: &mut Env,
    state: &mut EvalState,
    depth: usize,
    max_depth: usize,
) -> Result<LispVal, String> {
    // Compute deadline on first call, pass through recursion
    let deadline = if depth == 0 {
        let secs: u64 = std::env::var("RLM_TIME_BUDGET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        Instant::now() + std::time::Duration::from_secs(secs)
    } else {
        // Already set by parent — read from state
        state
            .rlm_state
            .get("__deadline")
            .and_then(|v| match v {
                LispVal::Float(f) => Some(
                    Instant::now()
                        + std::time::Duration::from_secs_f64(
                            *f - Instant::now().elapsed().as_secs_f64(),
                        ),
                ),
                _ => None,
            })
            .unwrap_or_else(|| Instant::now() + std::time::Duration::from_secs(300))
    };

    rlm_fractal_inner(task, env, state, depth, max_depth, deadline)
}

fn rlm_fractal_inner(
    task: String,
    env: &mut Env,
    state: &mut EvalState,
    depth: usize,
    max_depth: usize,
    deadline: Instant,
) -> Result<LispVal, String> {
    let max_retries: usize = 3;
    let do_verify = std::env::var("RLM_VERIFY").unwrap_or_default() == "1";

    // --- Budget checks (graceful degradation, not SIGKILL) ---
    let token_budget: usize = std::env::var("RLM_TOKEN_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);
    let call_budget: usize = std::env::var("RLM_CALL_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    // Time budget — return best effort instead of dying
    if Instant::now() >= deadline {
        eprintln!(
            "[rlm depth={}] ⚠ time budget exceeded, returning best effort",
            depth
        );
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str("Time budget exceeded".to_string()));
        return Ok(best);
    }
    let tokens_now = state.tokens_used.load(Ordering::Relaxed) as usize;
    if tokens_now >= token_budget {
        eprintln!(
            "[rlm depth={}] ⚠ token budget ({}/{})",
            depth, tokens_now, token_budget
        );
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str(format!(
                "Token budget exceeded ({} used)",
                tokens_now
            )));
        return Ok(best);
    }
    let calls_now = state.llm_calls.load(Ordering::Relaxed) as usize;
    if calls_now >= call_budget {
        eprintln!(
            "[rlm depth={}] ⚠ call budget ({}/{})",
            depth, calls_now, call_budget
        );
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str(format!(
                "Call budget exceeded ({} calls)",
                calls_now
            )));
        return Ok(best);
    }

    // Clear stale RLM state from parent/sibling — each node starts clean
    let saved_state = state.rlm_state.clone();
    state.rlm_state.clear();

    // --- Phase 1: TRY to solve in one shot ---
    let solve_result = rlm_try_solve(&task, env, state, max_retries);

    match solve_result {
        RlmNode::Black(result) => {
            // Generation (RED) → Execution succeeded → BLACK
            eprintln!(
                "[rlm depth={}] ■ BLACK: generation verified, result confirmed",
                depth
            );
            // Optional verification
            if do_verify {
                if let Some(verified) = rlm_verify(&task, &result, env, state)? {
                    // Restore parent state (preserve token counts)
                    merge_rlm_state(env, state, &saved_state);
                    return Ok(verified);
                }
                // Verification failed — fall through to split
                state.rlm_state.clear();
            } else {
                merge_rlm_state(env, state, &saved_state);
                return Ok(result);
            }
        }
        RlmNode::Red(reason) => {
            // Generation (RED) → Execution failed → stays RED → must split
            eprintln!(
                "[rlm depth={}] ■ RED: generation unverified — {}",
                depth, reason
            );
        }
    }

    // --- Phase 2: Can we split further? ---
    if depth >= max_depth {
        eprintln!(
            "[rlm depth={}] ⚠ max depth reached, returning best effort",
            depth
        );
        // Return whatever we have in state
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str(format!(
                "Could not solve after {} levels of decomposition",
                depth
            )));
        return Ok(best);
    }

    // --- Phase 3: DECOMPOSE into 2 subtasks ---
    if Instant::now() >= deadline {
        eprintln!(
            "[rlm depth={}] ⚠ time budget hit before decompose, returning best effort",
            depth
        );
        merge_rlm_state(env, state, &saved_state);
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str(
                "Time budget exceeded before decompose".to_string(),
            ));
        return Ok(best);
    }
    eprintln!("[rlm depth={}] ⟳ SPLITTING into 2 subtasks...", depth);

    let halves = rlm_decompose(&task, env, state)?;

    if halves.is_empty() {
        // Decomposition failed — best effort
        merge_rlm_state(env, state, &saved_state);
        let best = state
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str("Decomposition failed".to_string()));
        return Ok(best);
    }

    // --- Phase 4: Parallel execution of subtasks ---
    // Each branch gets its own Env fork (O(1) via im) and EvalState clone
    // (shares Arc<AtomicU64> counters). Run in parallel threads.
    let mut child_results: Vec<LispVal> = Vec::new();

    if halves.len() > 1 {
        // PARALLEL: spawn one thread per subtask
        let n = halves.len();
        eprintln!("[rlm depth={}] ⚡ PARALLEL {} subtasks", depth, n);

        // Pre-clone provider for each branch
        let provider_clones: Vec<Option<Box<dyn crate::eval::llm_provider::LlmProvider>>> = (0..n)
            .map(|_| state.llm_provider.as_ref().map(|p| p.box_clone()))
            .collect();

        let handles: Vec<std::thread::JoinHandle<Result<LispVal, String>>> = halves
            .into_iter()
            .zip(provider_clones.into_iter())
            .enumerate()
            .map(|(i, (subtask, prov))| {
                let mut env_fork = env.clone();
                let mut state_fork = state.clone();
                state_fork.llm_provider = prov;
                state_fork.rlm_state.clear();
                let total = n;
                std::thread::spawn(move || {
                    eprintln!(
                        "[rlm depth={}] → child {}/{}: {}",
                        depth + 1,
                        i + 1,
                        total,
                        truncate_str(&subtask, 80)
                    );
                    rlm_fractal_inner(
                        subtask,
                        &mut env_fork,
                        &mut state_fork,
                        depth + 1,
                        max_depth,
                        deadline,
                    )
                })
            })
            .collect();

        // Collect results in order
        for (i, handle) in handles.into_iter().enumerate() {
            match handle.join() {
                Ok(Ok(v)) => child_results.push(v),
                Ok(Err(e)) => {
                    eprintln!("[rlm depth={}] child {} error: {}", depth, i + 1, e);
                    child_results.push(LispVal::Str(format!("error: {}", e)));
                }
                Err(_) => {
                    eprintln!("[rlm depth={}] child {} panicked", depth, i + 1);
                    child_results.push(LispVal::Str(format!("error: child {} panicked", i + 1)));
                }
            }
        }
    } else {
        // SEQUENTIAL: single subtask, no thread overhead
        for (i, subtask) in halves.iter().enumerate() {
            eprintln!(
                "[rlm depth={}] → child {}/{}: {}",
                depth,
                i + 1,
                halves.len(),
                truncate_str(subtask, 80)
            );
            let pre_child_state = state.rlm_state.clone();
            state.rlm_state.clear();

            let child_result =
                rlm_fractal_inner(subtask.clone(), env, state, depth + 1, max_depth, deadline);

            // Restore parent rlm_state (tokens/calls already shared via Arc<AtomicU64>)
            state.rlm_state = pre_child_state;

            match child_result {
                Ok(v) => child_results.push(v),
                Err(e) => {
                    eprintln!(
                        "[rlm depth={}] child {}/{} error: {}",
                        depth,
                        i + 1,
                        halves.len(),
                        e
                    );
                    child_results.push(LispVal::Str(format!("error: {}", e)));
                }
            }
        }
    }

    // --- Phase 5: SYNTHESIZE ---
    eprintln!(
        "[rlm depth={}] ★ SYNTHESIZING {} child results",
        depth,
        child_results.len()
    );

    let combined = rlm_synthesize(&task, &child_results, env, state)?;

    // Restore parent state (preserve token counts)
    merge_rlm_state(env, state, &saved_state);

    // Optional verification of synthesized result
    if do_verify {
        if let Some(verified) = rlm_verify(&task, &combined, env, state)? {
            return Ok(verified);
        }
    }

    Ok(combined)
}

/// Result of a single try-solve attempt
/// RED = generation (LLM produced output, needs verification)
/// BLACK = success (output verified via execution)
enum RlmNode {
    Black(LispVal), // Generated (RED) → Executed → Success → BLACK
    Red(String),    // Generated (RED) → Execution failed → stays RED → trigger split
}

/// Try to solve a task in one shot: generate Lisp code, eval it, check for (final ...)
fn rlm_try_solve(task: &str, env: &mut Env, state: &mut EvalState, max_retries: usize) -> RlmNode {
    let sys_prompt =
        std::env::var("RLM_SYSTEM_PROMPT").unwrap_or_else(|_| RLM_SYSTEM_PROMPT.to_string());

    // Fresh context — no history from parent or siblings
    let mut messages: Vec<(String, String)> = vec![
        ("system".to_string(), sys_prompt),
        (
            "user".to_string(),
            format!(
                "Your task: {}\n\n\
                 CRITICAL: Return ONLY Lisp code. NO English text, NO explanations, NO markdown.\n\
                 Every line must be valid Lisp (start with ( or be a comment).\n\
                 \n\
                 Steps:\n\
                 1. Compute the result\n\
                 2. Verify it with (assert <condition>) — e.g. (assert (= result expected)), (assert (> len 0))\n\
                 3. Return it with (final <value>)\n\
                 \n\
                 Without (assert ...), your answer is UNVERIFIED and will be rejected.\n\
                 Available: llm, read-file, write-file, str-*, json-*, http-*, show-vars, rlm-set, rlm-get, assert, filter, map, reduce, loop, nth, len.\n\
                 Available state: {}",
                task,
                rlm_state_summary(env, state)
            ),
        ),
    ];

    for attempt in 0..=max_retries {
        // Clear stale AssertPassed from previous iteration so each attempt starts clean
        state.rlm_state.remove("AssertPassed");

        // Call LLM
        let resp = match state
            .llm_provider
            .as_ref()
            .unwrap()
            .complete(&messages, Some(8192))
        {
            Ok(r) => r,
            Err(e) => return RlmNode::Red(format!("LLM error: {}", e)),
        };
        state
            .tokens_used
            .fetch_add(resp.tokens as u64, Ordering::Relaxed);
        state.llm_calls.fetch_add(1, Ordering::Relaxed);

        let code_str = strip_markdown_fences(&resp.content);
        messages.push(("assistant".to_string(), truncate_str(&resp.content, 500)));

        eprintln!(
            "[rlm try {}] code:\n{}",
            attempt,
            truncate_str(&code_str, 300)
        );

        // Parse
        let exprs = match parse_all(&code_str) {
            Ok(e) => e,
            Err(e) => {
                if attempt < max_retries {
                    messages.push((
                        "user".to_string(),
                        format!("Parse error: {}. Return ONLY valid Lisp code, no English text, no markdown.", truncate_str(&e, 200)),
                    ));
                    continue;
                }
                return RlmNode::Red(format!("Parse error after {} retries: {}", max_retries, e));
            }
        };

        // Eval with snapshot rollback
        let snap = env.snapshot();
        let mut eval_ok = true;
        let mut err_msg = String::new();
        let mut result = LispVal::Nil;

        for expr in &exprs {
            match lisp_eval(expr, env, state) {
                Ok(v) => result = v,
                Err(e) => {
                    env.restore(snap);
                    err_msg = e;
                    eval_ok = false;
                    break;
                }
            }
        }

        if !eval_ok {
            if attempt < max_retries {
                messages.push((
                    "user".to_string(),
                    format!(
                        "Runtime error (retry {}/{}): {}. Return ONLY Lisp code, no English.",
                        attempt + 1,
                        max_retries,
                        truncate_str(&err_msg, 200)
                    ),
                ));
                continue;
            }
            return RlmNode::Red(format!("Runtime error: {}", err_msg));
        }

        // Check if (final ...) AND (assert ...) were called
        // RED→BLACK requires both: generation produced output AND verified it
        let is_final = state
            .rlm_state
            .get("Final")
            .map(|v| is_truthy(v))
            .unwrap_or(false);
        let is_asserted = state
            .rlm_state
            .get("AssertPassed")
            .map(|v| is_truthy(v))
            .unwrap_or(false);

        if is_final && is_asserted {
            // Generation (RED) → Execution OK + Assertion passed → BLACK
            if let Some(r) = state.rlm_state.get("result") {
                return RlmNode::Black(r.clone());
            }
            return RlmNode::Black(result);
        }

        if is_final && !is_asserted {
            // (final ...) called but no (assert ...) — stays RED
            if attempt < max_retries {
                messages.push((
                    "user".to_string(),
                    "You called (final ...) but didn't verify your result. \
                     Add (assert <condition>) before (final ...) to confirm correctness. \
                     Example: (assert (> result 0)) then (final result)"
                        .to_string(),
                ));
                continue;
            }
            return RlmNode::Red(
                "Generation not verified — no (assert ...) before (final ...)".to_string(),
            );
        }

        // Code ran but didn't call (final ...) — not done yet
        if attempt < max_retries {
            let result_preview = truncate_str(&result.to_string(), 200);
            messages.push((
                "user".to_string(),
                format!(
                    "Code ran but no (assert ...) and (final ...). Result: {}. \
                     Return ONLY Lisp code: add (assert ...) then (final <value>).",
                    result_preview
                ),
            ));
            continue;
        }

        // Used all retries without final — treat as fail (trigger split)
        return RlmNode::Red("Did not produce (final ...) after all retries".to_string());
    }

    RlmNode::Red("Exhausted all retries".to_string())
}

/// Decompose a failed task into exactly 2 subtasks
fn rlm_decompose(task: &str, _env: &mut Env, state: &mut EvalState) -> Result<Vec<String>, String> {
    let decompose_prompt = format!(
        "You need to split this task into exactly 2 independent subtasks.\n\
         Each subtask should be solvable in one shot by writing Lisp code.\n\
         Return ONLY a JSON array of exactly 2 strings, nothing else.\n\
         Example: [\"subtask one description\", \"subtask two description\"]\n\n\
         Task: {}",
        task
    );

    let messages = vec![
        (
            "system".to_string(),
            "You decompose tasks. Return ONLY a JSON array of exactly 2 strings. No explanation."
                .to_string(),
        ),
        ("user".to_string(), decompose_prompt),
    ];

    let resp = state
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(1024))?;
    state
        .tokens_used
        .fetch_add(resp.tokens as u64, Ordering::Relaxed);
    state.llm_calls.fetch_add(1, Ordering::Relaxed);

    // Parse the JSON array
    let content = resp.content.trim();

    // Try to find a JSON array in the response
    let json_str = if content.starts_with('[') {
        content.to_string()
    } else if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            content[start..=end].to_string()
        } else {
            return Ok(vec![]);
        }
    } else {
        return Ok(vec![]);
    };

    // Simple JSON array parsing — split by comma, strip quotes
    let inner = json_str.trim_start_matches('[').trim_end_matches(']');
    let subtasks: Vec<String> = inner
        .split("\",")
        .map(|s| s.trim().trim_matches('"').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if subtasks.len() < 2 {
        return Ok(vec![]);
    }

    // Take exactly 2
    Ok(vec![subtasks[0].clone(), subtasks[1].clone()])
}

/// Synthesize child results into a final answer
fn rlm_synthesize(
    task: &str,
    child_results: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    let child_summaries: Vec<String> = child_results
        .iter()
        .enumerate()
        .map(|(i, v)| format!("Child {}: {}", i + 1, truncate_str(&v.to_string(), 500)))
        .collect();

    let synth_prompt = format!(
        "You had this task: {}\n\n\
         You split it into subtasks and got these results:\n{}\n\n\
         Synthesize these into a single final answer. Write Lisp code that \
         returns the combined result via (final \"answer\").",
        task,
        child_summaries.join("\n")
    );

    // One-shot synthesis — fresh context
    let messages = vec![
        (
            "system".to_string(),
            "You combine subtask results into a final answer. Return Lisp code ending with (final ...).".to_string(),
        ),
        ("user".to_string(), synth_prompt),
    ];

    let resp = state
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(4096))?;
    state
        .tokens_used
        .fetch_add(resp.tokens as u64, Ordering::Relaxed);
    state.llm_calls.fetch_add(1, Ordering::Relaxed);

    let code_str = strip_markdown_fences(&resp.content);

    let exprs = parse_all(&code_str).map_err(|e| format!("Synthesis parse error: {}", e))?;

    let snap = env.snapshot();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        match lisp_eval(expr, env, state) {
            Ok(v) => result = v,
            Err(_e) => {
                env.restore(snap);
                // Fallback: join child results as string
                let combined = child_results
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(LispVal::Str(combined));
            }
        }
    }

    // Check if synthesis set a final result
    if let Some(r) = state.rlm_state.get("result") {
        Ok(r.clone())
    } else {
        Ok(result)
    }
}

/// Verify a result — returns Some(verified_result) if OK, None if failed verification
fn rlm_verify(
    task: &str,
    result: &LispVal,
    _env: &mut Env,
    state: &mut EvalState,
) -> Result<Option<LispVal>, String> {
    let verify_prompt = format!(
        "Given the task: {}\nAnd the result: {}\n\nIs this correct? Answer YES or NO and explain briefly.",
        task,
        truncate_str(&result.to_string(), 300)
    );

    let messages = vec![
        (
            "system".to_string(),
            "You are a verification assistant. Answer YES or NO.".to_string(),
        ),
        ("user".to_string(), verify_prompt),
    ];

    let resp = state
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(512))?;
    state
        .tokens_used
        .fetch_add(resp.tokens as u64, Ordering::Relaxed);
    state.llm_calls.fetch_add(1, Ordering::Relaxed);

    if resp.content.to_uppercase().starts_with("NO") {
        eprintln!("[rlm verify] FAILED: {}", truncate_str(&resp.content, 200));
        Ok(None) // Verification failed — caller can split
    } else {
        Ok(Some(result.clone()))
    }
}

/// Compact summary of rlm_state for context injection
fn rlm_state_summary(_env: &Env, state: &EvalState) -> String {
    if state.rlm_state.is_empty() {
        return "(empty)".to_string();
    }
    let entries: Vec<String> = state
        .rlm_state
        .iter()
        .map(|(k, v)| format!("{} = {}", k, truncate_str(&v.to_string(), 60)))
        .collect();
    truncate_str(&entries.join(", "), 300).to_string()
}

/// Merge saved parent state back into env, preserving cumulative token/call counts.
/// With shared atomics, tokens/calls are already accumulated — just restore rlm_state.
fn merge_rlm_state(_env: &mut Env, state: &mut EvalState, saved: &im::OrdMap<String, LispVal>) {
    state.rlm_state = saved.clone();
}

// ---------------------------------------------------------------------------
// Lambda application
// ---------------------------------------------------------------------------

/// Apply a lambda (or macro) to a set of arguments.
///
/// Creates a temporary scope in `caller_env` by extending it with:
/// 1. The `closed_env` bindings (captured closure variables),
/// 2. The `params` bound positionally from `args` (missing args default to
///    [`LispVal::Nil`]),
/// 3. An optional `rest_param` that collects leftover arguments into a
///    [`LispVal::List`].
///
/// The body is then evaluated via [`lisp_eval`].  After evaluation the
/// environment is truncated back to its original size, restoring lexical
/// scoping.
///
/// # Errors
///
/// Propagates any evaluation error from the body.
pub fn apply_lambda(
    params: &[String],
    rest_param: &Option<String>,
    body: &LispVal,
    closed_env: &std::sync::Arc<std::sync::RwLock<im::HashMap<String, LispVal>>>,
    args: &[LispVal],
    caller_env: &mut Env,
    _state: &mut EvalState,
) -> Result<EvalResult, String> {
    // Clone the caller's env (O(1) via structural sharing), add bindings.
    // Return TailCall — the trampoline evaluates the body iteratively.
    let mut local_env = caller_env.clone();
    local_env.set_shared_env(closed_env.clone());

    // Overlay closed_env bindings (lexical scope)
    for (k, v) in closed_env.read().unwrap().iter() {
        local_env.push(k.to_string(), v.clone());
    }
    for (i, p) in params.iter().enumerate() {
        local_env.push(p.to_string(), args.get(i).cloned().unwrap_or(LispVal::Nil));
    }
    if let Some(rest_name) = rest_param {
        let rest_args: Vec<LispVal> = args.get(params.len()..).unwrap_or(&[]).to_vec();
        local_env.push(rest_name.to_string(), LispVal::List(rest_args));
    }

    Ok(EvalResult::TailCall {
        expr: body.clone(),
        env: local_env,
    })
}

// ---------------------------------------------------------------------------
// Function dispatch
// ---------------------------------------------------------------------------

/// Dispatch a builtin by name with already-evaluated args.
/// Used by call_val when a builtin symbol is passed as a first-class value.
fn dispatch_call_with_args(
    name: &str,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<EvalResult, String> {
    if let Some(result) = dispatch_arithmetic::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_collections::handle(name, args, env, state)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_strings::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_predicates::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_json::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_http::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_state::handle(name, args, env, state)? {
        return Ok(EvalResult::Value(result));
    }
    if let Some(result) = dispatch_types::handle(name, args)? {
        return Ok(EvalResult::Value(result));
    }
    match name {
        "sha256" => Ok(EvalResult::Value(builtin_sha256(args)?)),
        "keccak256" => Ok(EvalResult::Value(builtin_keccak256(args)?)),
        _ => Err(format!("{}: not a dispatchable builtin", name)),
    }
}

fn dispatch_call(
    list: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<EvalResult, String> {
    let head = &list[0];
    let raw_args: Vec<LispVal> = list[1..].to_vec();

    // Check if head resolves to a Macro — macros get unevaluated args
    if let LispVal::Sym(name) = head {
        if let Some(func) = env.get(name) {
            if matches!(func, LispVal::Macro { .. }) {
                let func_clone = func.clone();
                return call_val(&func_clone, &raw_args, env, state);
            }
        }
    }

    // Normal path: evaluate args
    // Save/restore env around EACH arg — inner lisp_eval may replace env
    // via TailCall (e.g. recursive fib), corrupting the view for subsequent args.
    let saved_env = env.snapshot();
    let mut args: Vec<LispVal> = Vec::with_capacity(raw_args.len());
    for a in &raw_args {
        let val = lisp_eval(a, env, state)?;
        env.restore(saved_env.clone());
        args.push(val);
    }

    if let LispVal::Sym(name) = head {
        // ── Dispatch chain: delegate to domain modules ──
        if let Some(result) = dispatch_arithmetic::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_collections::handle(name, &args, env, state)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_strings::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_predicates::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_json::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_http::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_state::handle(name, &args, env, state)? {
            return Ok(EvalResult::Value(result));
        }
        if let Some(result) = dispatch_types::handle(name, &args)? {
            return Ok(EvalResult::Value(result));
        }

        // ── Inline builtins: crypto + LLM/RLM ──
        match name.as_str() {
            "sha256" => Ok(EvalResult::Value(builtin_sha256(&args)?)),
            "keccak256" => Ok(EvalResult::Value(builtin_keccak256(&args)?)),

            // --- LLM builtins ---

            // --- LLM builtins ---
            "llm" => {
                let prompt = as_str(&args[0])?;
                let messages = vec![
                    ("system".to_string(), "You are a helpful assistant with access to a Lisp runtime called lisp-rlm.".to_string()),
                    ("user".to_string(), prompt),
                ];
                let resp = state
                    .llm_provider
                    .as_ref()
                    .ok_or("llm: no LLM provider configured")?
                    .complete(&messages, Some(2048))?;
                state
                    .tokens_used
                    .fetch_add(resp.tokens as u64, Ordering::Relaxed);
                state.llm_calls.fetch_add(1, Ordering::Relaxed);
                Ok(EvalResult::Value(LispVal::Str(resp.content)))
            }
            "llm-code" => {
                let prompt = as_str(&args[0])?;
                let messages = vec![
                    ("system".to_string(), RLM_SYSTEM_PROMPT.to_string()),
                    ("user".to_string(), prompt),
                ];
                let resp = state
                    .llm_provider
                    .as_ref()
                    .ok_or("llm-code: no LLM provider configured")?
                    .complete(&messages, Some(2048))?;

                state
                    .tokens_used
                    .fetch_add(resp.tokens as u64, Ordering::Relaxed);
                state.llm_calls.fetch_add(1, Ordering::Relaxed);

                let code_str = strip_markdown_fences(&resp.content);

                // Parse and eval the LLM-generated Lisp code
                let exprs = parse_all(&code_str)?;
                let mut result = LispVal::Nil;
                for expr in &exprs {
                    result = lisp_eval(expr, env, state)?;
                }
                Ok(EvalResult::Value(result))
            }

            // --- Fractal RLM: try-solve → fail → binary split → recurse ---
            // Self-similar at every scale. O(1) context per node. O(log n) depth.
            // Red-black inspired: "no two reds in a row" = must try-solve before splitting again
            "rlm" => {
                let task = as_str(&args[0])?;
                let max_depth: usize = std::env::var("RLM_MAX_DEPTH")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(6);
                rlm_fractal(task, env, state, 0, max_depth).map(EvalResult::Value)
            }

            // --- Sub-RLM: delegates to the same fractal loop ---
            "sub-rlm" => {
                let sub_task = as_str(&args[0])?;
                if state.rlm_depth >= 5 {
                    return Err("sub-rlm: max depth (5) exceeded".into());
                }
                state.rlm_depth += 1;
                let max_depth: usize = std::env::var("RLM_MAX_DEPTH")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(6);
                let result = rlm_fractal(sub_task, env, state, 0, max_depth);
                state.rlm_depth -= 1;
                match &result {
                    Ok(v) => Ok(EvalResult::Value(LispVal::Str(v.to_string()))),
                    Err(e) => Ok(EvalResult::Value(LispVal::Str(format!("error: {}", e)))),
                }
            }

            "show-vars" => {
                let mut entries: Vec<String> = Vec::new();
                for (name, val) in env.iter() {
                    let type_str = match val {
                        LispVal::Nil => "nil",
                        LispVal::Bool(_) => "bool",
                        LispVal::Num(_) => "int",
                        LispVal::Float(_) => "float",
                        LispVal::Str(_) => "string",
                        LispVal::List(l) => &format!("list[{}]", l.len()),
                        LispVal::Map(m) => &format!("map[{}]", m.len()),
                        LispVal::Lambda { params, .. } => &format!("lambda({})", params.len()),
                        LispVal::Macro { .. } => "macro",
                        LispVal::Sym(_) => "symbol",
                        LispVal::Recur(_) => "recur",
                    };
                    // Truncate value display
                    let val_preview = truncate_str(&val.to_string(), 80);
                    entries.push(format!("  {} : {} = {}", name, type_str, val_preview));
                }
                Ok(EvalResult::Value(LispVal::Str(entries.join("\n"))))
            }
            "llm-batch" => {
                let prompts = match &args[0] {
                    LispVal::List(l) => l.clone(),
                    _ => return Err("llm-batch: need list of prompt strings".into()),
                };
                let mut results: Vec<LispVal> = Vec::new();
                for p in &prompts {
                    let prompt_str = as_str(p)?;
                    // Call llm on each prompt
                    let call = LispVal::List(vec![
                        LispVal::Sym("llm".to_string()),
                        LispVal::Str(prompt_str),
                    ]);
                    let result = lisp_eval(&call, env, state)?;
                    results.push(result);
                }
                Ok(EvalResult::Value(LispVal::List(results)))
            }
            "show-context" => {
                let context_val = state
                    .rlm_state
                    .get("context")
                    .cloned()
                    .unwrap_or(LispVal::Nil);
                let context_str = context_val.to_string();
                let preview = truncate_str(&context_str, 200);
                let final_set = state
                    .rlm_state
                    .get("Final")
                    .map(|v| is_truthy(v))
                    .unwrap_or(false);
                Ok(EvalResult::Value(LispVal::Str(format!(
                    "Context length: {} chars\nPreview: {}\nIteration: {}\nFinal set: {}",
                    context_str.len(),
                    preview,
                    state.rlm_iteration,
                    final_set
                ))))
            }

            // --- Token tracking ---
            "rlm-tokens" => Ok(EvalResult::Value(LispVal::Num(
                state.tokens_used.load(Ordering::Relaxed) as i64,
            ))),
            "rlm-calls" => Ok(EvalResult::Value(LispVal::Num(
                state.llm_calls.load(Ordering::Relaxed) as i64,
            ))),
            "rlm-write" => {
                // Like (rlm "task") but returns the generated code as a string
                // Also saves to file if path is provided as second arg
                let task = as_str(&args[0])?;
                let save_path = args.get(1).map(|v| as_str(v)).transpose()?;

                if state.llm_provider.is_none() {
                    return Err("rlm-write: no LLM provider configured".to_string());
                }

                let sys = r#"You are a Lisp code generator for lisp-rlm. Return ONLY raw Lisp code — no markdown fences, no explanations, no backticks.

SYNTAX RULES:
- Use \n for newlines in strings: (str-concat "line1\n" "line2") — NOT literal line breaks
- Check empty list with (= (len lst) 0) — NOT null?
- Use (str-join sep list) to join strings — NOT reduce with str-concat
- Use (reduce + 0 nums) for sum, (reduce * 1 nums) for product — the lambda gets (accumulator element)
- Define functions: (define (name args) body)
- Update variables: (set! var val)
- Iterate: (loop () body... (recur)) or (define i 0) + (loop () (if cond (begin ... (set! i (+ i 1)) (recur))))
- Comments: ;;
- NO write-file wrappers — code is saved automatically

WORKING EXAMPLES:

;; Example 1: Define and test a function
(define (square x) (* x x))
(println (str-concat "5 squared = " (to-string (square 5))))

;; Example 2: Process a list with map/filter/reduce
(define nums (list 1 2 3 4 5 6 7 8 9 10))
(define evens (filter (lambda (n) (= (mod n 2) 0)) nums))
(define doubled (map (lambda (n) (* n 2)) evens))
(println (str-concat "Sum of doubled evens: " (to-string (reduce + 0 doubled))))

;; Example 3: Read file, count words
(define content (read-file "/tmp/data.txt"))
(define lines (str-split content "\n"))
(define (count-words line) (len (str-split line " ")))
(define total (reduce + 0 (map count-words lines)))
(println (str-concat "Total words: " (to-string total)))

;; Example 4: Join strings properly
(define names (list "Alice" "Bob" "Charlie"))
(println (str-join ", " names))
;; → "Alice, Bob, Charlie"

;; Example 5: Chunk a document and batch analyze
(define text (read-file "/tmp/paper.txt"))
(define chunks (str-chunk text 5))
(define prompts (map (lambda (c) (str-concat "Summarize the key ideas:\n" c)) chunks))
(define summaries (llm-batch prompts))
(define all-summaries (str-join "\n\n" summaries))
(define answer (llm (str-concat "Synthesize into one paragraph:\n\n" all-summaries)))
(final answer)

;; Example 6: Recursive function
(define (factorial n)
  (if (= n 0) 1
    (* n (factorial (- n 1)))))
(println (str-concat "10! = " (to-string (factorial 10))))

;; Example 7: Loop with iteration
(define i 0)
(define total 0)
(loop ()
  (if (> i 100)
    (println (str-concat "Sum 1-100 = " (to-string total)))
    (begin
      (set! total (+ total i))
      (set! i (+ i 1))
      (recur))))

;; Example 8: Error handling
(try
  (begin
    (define data (read-file "/tmp/optional.txt"))
    (println data))
  (lambda (e) (println "File not found, skipping")))

AVAILABLE BUILTINS:
define def let lambda if cond begin loop recur set!
print println read-file write-file append-file load-file
str-concat str-join str-split str-length str-substring str-trim str-contains str-chunk
list cons car cdr nth len append reverse map filter reduce sort range
+ - * / mod = < > >= <= not and or
to-string to-int to-float number? string? list? empty? nil? bool?
llm llm-batch sub-rlm rlm-set rlm-get
show-vars show-context final final-var snapshot rollback
try catch error fork
type-of check check! matches? valid-type?
defschema validate schema

DO NOT wrap code in markdown fences. DO NOT add explanations."#;

                // First call: initial code generation
                let gen_messages = vec![
                    ("system".to_string(), sys.to_string()),
                    ("user".to_string(), task.clone()),
                ];
                let gen_resp = state
                    .llm_provider
                    .as_ref()
                    .unwrap()
                    .complete(&gen_messages, Some(8192))?;
                state
                    .tokens_used
                    .fetch_add(gen_resp.tokens as u64, Ordering::Relaxed);
                state.llm_calls.fetch_add(1, Ordering::Relaxed);
                let code = strip_markdown_fences(&gen_resp.content);

                // Verify parse, retry once if broken
                let final_code = if crate::parser::parse_all(&code).is_err() {
                    let fix_messages = vec![
                        ("system".to_string(), sys.to_string()),
                        ("assistant".to_string(), code.clone()),
                        ("user".to_string(), "The previous code had a parse error. Write it again, fixed. Return ONLY valid raw Lisp code, no markdown, no explanations.".to_string()),
                    ];
                    let fix_resp = state
                        .llm_provider
                        .as_ref()
                        .unwrap()
                        .complete(&fix_messages, Some(8192))?;
                    state
                        .tokens_used
                        .fetch_add(fix_resp.tokens as u64, Ordering::Relaxed);
                    state.llm_calls.fetch_add(1, Ordering::Relaxed);
                    let fixed = strip_markdown_fences(&fix_resp.content);
                    if crate::parser::parse_all(&fixed).is_ok() {
                        fixed
                    } else {
                        code
                    }
                } else {
                    code.clone()
                };

                // Strip trailing write-file calls (rlm-write saves automatically)
                let final_code = final_code.trim_end().to_string();
                let final_code = if final_code.ends_with(")") {
                    // Remove last top-level (write-file ...) if present
                    let trimmed = final_code.trim_end();
                    if trimmed
                        .rfind("(write-file")
                        .map(|i| {
                            // Check it's a top-level form (count parens before it)
                            let before = &trimmed[..i];
                            let open = before.chars().filter(|c| *c == '(').count();
                            let close = before.chars().filter(|c| *c == ')').count();
                            open == close // top-level if parens are balanced before it
                        })
                        .unwrap_or(false)
                    {
                        trimmed[..trimmed.rfind("(write-file").unwrap()]
                            .trim_end()
                            .to_string()
                    } else {
                        final_code
                    }
                } else {
                    final_code
                };

                // Save to file if path provided (no unescaping — this is source code, \n should stay as \n)
                if let Some(ref path) = save_path {
                    std::fs::write(path, &final_code).map_err(|e| format!("rlm-write: {}", e))?;
                }

                Ok(EvalResult::Value(LispVal::Str(final_code)))
            } // --- RLM builtins ---
            "rlm/signature" => {
                let sig_name = as_str(&args[0])?;
                let inputs = match &args[1] {
                    LispVal::List(l) => {
                        l.iter().map(|v| as_str(v)).collect::<Result<Vec<_>, _>>()?
                    }
                    _ => return Err("rlm/signature: inputs must be list".into()),
                };
                let outputs = match &args[2] {
                    LispVal::List(l) => {
                        l.iter().map(|v| as_str(v)).collect::<Result<Vec<_>, _>>()?
                    }
                    _ => return Err("rlm/signature: outputs must be list".into()),
                };
                let mut m = im::HashMap::new();
                m.insert("name".to_string(), LispVal::Str(sig_name));
                m.insert(
                    "inputs".to_string(),
                    LispVal::List(inputs.into_iter().map(LispVal::Str).collect()),
                );
                m.insert(
                    "outputs".to_string(),
                    LispVal::List(outputs.into_iter().map(LispVal::Str).collect()),
                );
                Ok(EvalResult::Value(LispVal::Map(m)))
            }
            "rlm/format-prompt" => {
                let sig = &args[0];
                let input_dict = &args[1];
                let sig_name = match sig {
                    LispVal::Map(m) => m
                        .get("name")
                        .and_then(|v| as_str(v).ok())
                        .unwrap_or_default(),
                    _ => "unknown".to_string(),
                };
                let inputs = match sig {
                    LispVal::Map(m) => match m.get("inputs") {
                        Some(LispVal::List(l)) => l
                            .iter()
                            .map(|v| as_str(v).unwrap_or_default())
                            .collect::<Vec<_>>(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                let outputs = match sig {
                    LispVal::Map(m) => match m.get("outputs") {
                        Some(LispVal::List(l)) => l
                            .iter()
                            .map(|v| as_str(v).unwrap_or_default())
                            .collect::<Vec<_>>(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                let mut prompt = format!("You are a {} function.\n\nInputs:\n", sig_name);
                for inp in &inputs {
                    let val = match input_dict {
                        LispVal::Map(m) => m
                            .get(inp)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "nil".to_string()),
                        _ => "nil".to_string(),
                    };
                    prompt.push_str(&format!("- {}: {}\n", inp, val));
                }
                prompt.push_str("\nOutputs:\n");
                for out in &outputs {
                    prompt.push_str(&format!("- {}\n", out));
                }
                prompt.push_str("\nRespond with a JSON object containing the output fields.");
                Ok(EvalResult::Value(LispVal::Str(prompt)))
            }
            "rlm/trace" => {
                let step = as_str(&args[0])?;
                let data = &args[1];
                eprintln!("[RLM] {}: {}", step, data);
                Ok(EvalResult::Value(LispVal::Bool(true)))
            }
            "rlm/config" => {
                let key = as_str(&args[0])?;
                let val = args[1].clone();
                env.push(format!("__rlm_{}__", key), val);
                Ok(EvalResult::Value(LispVal::Bool(true)))
            }

            // -- Tier 1: Control --
            "apply" => {
                // (apply f arg1 arg2 ... arglist)
                // Last arg must be a list, prepend any preceding args
                if args.len() < 2 {
                    return Err("apply: need (f ... arglist)".into());
                }
                let func = args[0].clone();
                let mut apply_args = args[1..args.len() - 1].to_vec();
                match args.last() {
                    Some(LispVal::List(lst)) => apply_args.extend(lst.iter().cloned()),
                    Some(LispVal::Nil) => {}
                    _ => return Err("apply: last arg must be list".into()),
                }
                call_val(&func, &apply_args, env, state)
            }
            "eval" => {
                // (eval datum) — evaluate datum as code in current env
                let datum = args.first().ok_or("eval: need 1 arg")?;
                lisp_eval(datum, env, state).map(EvalResult::Value)
            }

            // -- Tier 1: IO --
            "delete-file" => {
                let path = as_str(args.first().ok_or("delete-file: need path")?)?;
                match std::fs::remove_file(&path) {
                    Ok(()) => Ok(EvalResult::Value(LispVal::Bool(true))),
                    Err(e) => Err(format!("delete-file: {}", e)),
                }
            }

            _ => {
                let func = env
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("undefined: {}", name))?;
                call_val(&func, &args, env, state)
            }
        }
    } else if let LispVal::Lambda {
        params,
        rest_param,
        body,
        closed_env,
    } = head
    {
        apply_lambda(params, &rest_param, body, closed_env, &args, env, state)
    } else {
        // Head is a compound expression — evaluate it, then call the result
        let func = lisp_eval(head, env, state)?;
        call_val(&func, &args, env, state)
    }
}

fn call_val(
    func: &LispVal,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<EvalResult, String> {
    match func {
        LispVal::Lambda {
            params,
            rest_param,
            body,
            closed_env,
        } => apply_lambda(params, rest_param, body, closed_env, args, env, state),
        LispVal::Macro {
            params,
            rest_param,
            body,
            closed_env,
        } => {
            // Macros receive UNEVALUATED args, return code to be evaluated
            let expanded = apply_lambda(params, rest_param, body, closed_env, args, env, state)?;
            let expanded_val = match expanded {
                EvalResult::Value(v) => v,
                EvalResult::TailCall {
                    expr,
                    env: tail_env,
                } => {
                    // Evaluate the expansion in the macro's env, but don't
                    // overwrite caller's env — the expansion is just code.
                    let mut tmp_env = tail_env;
                    lisp_eval(&expr, &mut tmp_env, state)?
                }
            };
            // Now eval the expanded code in the CALLER's env (preserved)
            let result = lisp_eval(&expanded_val, env, state)?;
            Ok(EvalResult::Value(result))
        }
        LispVal::List(ll) if ll.len() >= 3 => {
            let (params, rest_param) = parse_params(&ll[1])?;
            apply_lambda(
                &params,
                &rest_param,
                &ll[2],
                &std::sync::Arc::new(std::sync::RwLock::new(im::HashMap::new())),
                args,
                env,
                state,
            )
        }
        LispVal::Map(m) if m.contains_key("__contract") => {
            // Contract-wrapped function — check param types, call, check return type
            let inner_fn = m.get("fn").ok_or("contract: missing fn")?;
            let param_type_strs = match m.get("param_types") {
                Some(LispVal::List(ts)) => ts.clone(),
                _ => vec![],
            };
            let ret_type_str = match m.get("return_type") {
                Some(LispVal::Str(s)) => Some(s.clone()),
                _ => None,
            };

            // Check argument types
            for (i, (arg, type_str)) in args.iter().zip(param_type_strs.iter()).enumerate() {
                let t = dispatch_types::parse_type(type_str)
                    .map_err(|e| format!("contract: invalid param type: {}", e))?;
                if !dispatch_types::type_matches(arg, &t) {
                    return Err(format!(
                        "contract violation: param {} expected {}, got {} — {}",
                        i + 1,
                        dispatch_types::format_type(&t),
                        dispatch_types::type_of(arg),
                        match arg {
                            LispVal::Str(s) => format!("\"{}\"", s),
                            other => other.to_string(),
                        }
                    ));
                }
            }

            // Call the inner function
            let result = call_val(inner_fn, args, env, state)?;

            // Resolve TailCall to get the actual value for return type check
            let resolved = match &result {
                EvalResult::Value(v) => v.clone(),
                EvalResult::TailCall {
                    expr, env: tc_env, ..
                } => {
                    let mut tc_env = tc_env.clone();
                    lisp_eval(expr, &mut tc_env, state)?
                }
            };

            // Check return type
            if let Some(ref ret_str) = ret_type_str {
                let ret_type_sym = LispVal::Sym(ret_str.clone());
                let rt = dispatch_types::parse_type(&ret_type_sym)
                    .map_err(|e| format!("contract: invalid return type: {}", e))?;
                if !dispatch_types::type_matches(&resolved, &rt) {
                    return Err(format!(
                        "contract violation: return expected {}, got {} — {}",
                        dispatch_types::format_type(&rt),
                        dispatch_types::type_of(&resolved),
                        resolved.to_string()
                    ));
                }
            }

            // Return the original result (TailCall or Value) so the trampoline continues
            Ok(result)
        }
        LispVal::Sym(name) => {
            // Resolve symbol in env first. If not found (builtin), dispatch by name.
            if let Some(resolved) = env.get(name).cloned() {
                call_val(&resolved, args, env, state)
            } else if is_builtin_name(name) {
                dispatch_call_with_args(name, args, env, state)
            } else {
                Err(format!("undefined: {}", name))
            }
        }
        _ => Err(format!("not callable: {}", func)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_all;
    use std::io::Write;

    fn eval_str(code: &str) -> Result<LispVal, String> {
        let exprs = parse_all(code).expect("parse failed");
        let mut env = Env::new();
        let mut state = EvalState::new();
        let mut result = LispVal::Nil;
        for expr in &exprs {
            result = lisp_eval(expr, &mut env, &mut state)?;
        }
        Ok(result)
    }

    // --- Phase 1: File I/O ---

    #[test]
    fn test_write_and_read_file() {
        let path = "/tmp/lisp_rlm_test_io.txt";
        let _ = std::fs::remove_file(path);
        let r = eval_str(&format!(r#"(write-file "{}" "hello world")"#, path));
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), LispVal::Bool(true));
        let r = eval_str(&format!(r#"(read-file "{}")"#, path));
        assert_eq!(r.unwrap(), LispVal::Str("hello world".to_string()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_append_file() {
        let path = "/tmp/lisp_rlm_test_append.txt";
        let _ = std::fs::remove_file(path);
        eval_str(&format!(r#"(write-file "{}" "abc")"#, path)).unwrap();
        eval_str(&format!(r#"(append-file "{}" "def")"#, path)).unwrap();
        let r = eval_str(&format!(r#"(read-file "{}")"#, path));
        assert_eq!(r.unwrap(), LispVal::Str("abcdef".to_string()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_file_exists() {
        let path = "/tmp/lisp_rlm_test_exists.txt";
        let _ = std::fs::remove_file(path);
        let r = eval_str(&format!(r#"(file-exists? "{}")"#, path));
        assert_eq!(r.unwrap(), LispVal::Bool(false));
        std::fs::write(path, "x").unwrap();
        let r = eval_str(&format!(r#"(file-exists? "{}")"#, path));
        assert_eq!(r.unwrap(), LispVal::Bool(true));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_shell_blocked_and_allowed() {
        // Test blocked
        std::env::remove_var("RLM_ALLOW_SHELL");
        let r = eval_str(r#"(shell "echo hi")"#);
        assert!(r.is_err(), "expected shell to be blocked, got {:?}", r);
        assert!(r.unwrap_err().contains("blocked"));

        // Test allowed
        std::env::set_var("RLM_ALLOW_SHELL", "1");
        let r = eval_str(r#"(shell "echo hello")"#);
        std::env::remove_var("RLM_ALLOW_SHELL");
        assert!(r.is_ok());
        let s = match r.unwrap() {
            LispVal::Str(s) => s,
            _ => panic!("expected string"),
        };
        assert_eq!(s.trim(), "hello");
    }

    #[test]
    fn test_read_file_not_found() {
        let r = eval_str(r#"(read-file "/tmp/lisp_rlm_nonexistent_12345.txt")"#);
        assert!(r.is_err());
    }

    // --- Phase 2: HTTP builtins ---
    // These are integration tests that require network access.

    #[test]
    fn test_http_get() {
        let r = eval_str(r#"(http-get "https://httpbin.org/get")"#);
        assert!(r.is_ok(), "http-get failed: {:?}", r);
        let body = match r.unwrap() {
            LispVal::Str(s) => s,
            _ => panic!("expected string"),
        };
        assert!(body.contains("httpbin.org"));
    }

    #[test]
    fn test_http_post() {
        let r =
            eval_str(r#"(http-post "https://httpbin.org/post" (to-json (dict "hello" "world")))"#);
        assert!(r.is_ok(), "http-post failed: {:?}", r);
        let body = match r.unwrap() {
            LispVal::Str(s) => s,
            _ => panic!("expected string"),
        };
        assert!(body.contains("hello"));
    }

    #[test]
    fn test_http_get_json() {
        let r = eval_str(r#"(http-get-json "https://httpbin.org/json")"#);
        assert!(r.is_ok(), "http-get-json failed: {:?}", r);
        // Should return a LispVal::Map (parsed JSON)
        match r.unwrap() {
            LispVal::Map(_) => {}
            other => panic!("expected map, got {}", other),
        }
    }

    // --- Phase 3: LLM builtins ---
    // These tests check error handling without an API key set.

    #[test]
    fn test_llm_no_api_key() {
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        let r = eval_str(r#"(llm "hello")"#);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(err.contains("provider") || err.contains("API_KEY"));
    }

    #[test]
    fn test_llm_code_no_api_key() {
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        let r = eval_str(r#"(llm-code "compute 2+2")"#);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(err.contains("provider") || err.contains("API_KEY"));
    }
}
