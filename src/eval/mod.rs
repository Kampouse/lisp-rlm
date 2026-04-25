use std::collections::BTreeMap;
use std::time::Instant;

use std::sync::LazyLock;

use crate::helpers::*;
use crate::parser::parse_all;
use crate::types::{get_stdlib_code, Env, LispVal};

pub mod crypto;
pub mod errors;
pub mod helpers;
pub mod llm_provider;
pub mod quasiquote;

// Domain-specific dispatch modules (v0.2 god-function split)
pub mod dispatch_arithmetic;
pub mod dispatch_collections;
pub mod dispatch_http;
pub mod dispatch_json;
pub mod dispatch_predicates;
pub mod dispatch_state;
pub mod dispatch_strings;

pub use llm_provider::*;

use crypto::{builtin_keccak256, builtin_sha256};
use helpers::{extract_first_valid_expr, strip_markdown_fences, truncate_str};
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
            let map: BTreeMap<String, LispVal> =
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

Arithmetic: + - * / mod
Comparison: = < > <= >= not
Logic: and or
Lists: list cons car cdr nth len append reverse map filter reduce sort range zip find some every
Predicates: nil? list? number? string? bool? map? macro? type? empty?
Strings: str-concat str-contains str-split str-split-exact str-trim str-upcase str-downcase str-length str-substring str-index-of str-starts-with str-ends-with str= str!= str-chunk str-join

IO — file and shell:
  (read-file "path.txt")           → string contents
  (write-file "path.txt" content)  → writes string to file
  (append-file "path.txt" content) → appends string to file
  (file-exists? "path.txt")        → bool
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
Special forms: define def let lambda if cond match quote quasiquote unquote unquote-splicing loop recur begin progn defmacro require try catch error"#;

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
/// Each call increments `env.eval_count` and checks it against
/// `env.eval_budget`.  When the budget is exceeded an `Err` is returned.  A
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
pub fn lisp_eval(expr: &LispVal, env: &mut Env) -> Result<LispVal, String> {
    // Execution budget check
    if env.eval_budget > 0 {
        env.eval_count += 1;
        if env.eval_count > env.eval_budget {
            return Err(format!(
                "execution budget exceeded: {} iterations (limit: {})",
                env.eval_count, env.eval_budget
            ));
        }
    }
    stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || lisp_eval_inner(expr, env))
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
        env.rlm_state
            .get("__deadline")
            .and_then(|v| match v {
                LispVal::Float(f) => Some(Instant::now() + std::time::Duration::from_secs_f64(*f - Instant::now().elapsed().as_secs_f64())),
                _ => None,
            })
            .unwrap_or_else(|| Instant::now() + std::time::Duration::from_secs(300))
    };

    rlm_fractal_inner(task, env, depth, max_depth, deadline)
}

fn rlm_fractal_inner(
    task: String,
    env: &mut Env,
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
        eprintln!("[rlm depth={}] ⚠ time budget exceeded, returning best effort", depth);
        let best = env.rlm_state.get("result").cloned()
            .unwrap_or(LispVal::Str("Time budget exceeded".to_string()));
        return Ok(best);
    }
    if env.tokens_used >= token_budget {
        eprintln!("[rlm depth={}] ⚠ token budget ({}/{})", depth, env.tokens_used, token_budget);
        let best = env.rlm_state.get("result").cloned()
            .unwrap_or(LispVal::Str(format!("Token budget exceeded ({} used)", env.tokens_used)));
        return Ok(best);
    }
    if env.llm_calls >= call_budget {
        eprintln!("[rlm depth={}] ⚠ call budget ({}/{})", depth, env.llm_calls, call_budget);
        let best = env.rlm_state.get("result").cloned()
            .unwrap_or(LispVal::Str(format!("Call budget exceeded ({} calls)", env.llm_calls)));
        return Ok(best);
    }

    // Clear stale RLM state from parent/sibling — each node starts clean
    let saved_state = env.rlm_state.clone();
    env.rlm_state.clear();

    // --- Phase 1: TRY to solve in one shot ---
    let solve_result = rlm_try_solve(&task, env, max_retries);

    match solve_result {
        RlmNode::Black(result) => {
            // Generation (RED) → Execution succeeded → BLACK
            eprintln!(
                "[rlm depth={}] ■ BLACK: generation verified, result confirmed",
                depth
            );
            // Optional verification
            if do_verify {
                if let Some(verified) = rlm_verify(&task, &result, env)? {
                    // Restore parent state (preserve token counts)
                    merge_rlm_state(env, &saved_state);
                    return Ok(verified);
                }
                // Verification failed — fall through to split
                env.rlm_state.clear();
            } else {
                merge_rlm_state(env, &saved_state);
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
        let best = env
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
        eprintln!("[rlm depth={}] ⚠ time budget hit before decompose, returning best effort", depth);
        merge_rlm_state(env, &saved_state);
        let best = env.rlm_state.get("result").cloned()
            .unwrap_or(LispVal::Str("Time budget exceeded before decompose".to_string()));
        return Ok(best);
    }
    eprintln!("[rlm depth={}] ⟳ SPLITTING into 2 subtasks...", depth);

    let halves = rlm_decompose(&task, env)?;

    if halves.is_empty() {
        // Decomposition failed — best effort
        merge_rlm_state(env, &saved_state);
        let best = env
            .rlm_state
            .get("result")
            .cloned()
            .unwrap_or(LispVal::Str("Decomposition failed".to_string()));
        return Ok(best);
    }

    // --- Phase 4: DFS — recurse on left, then right ---
    // Each child gets a clean rlm_state, but inherits cumulative token counts
    let mut child_results: Vec<LispVal> = Vec::new();

    for (i, subtask) in halves.iter().enumerate() {
        eprintln!(
            "[rlm depth={}] → child {}/{}: {}",
            depth,
            i + 1,
            halves.len(),
            truncate_str(subtask, 80)
        );
        // Save state before child, restore after (isolate siblings)
        let pre_child_state = env.rlm_state.clone();
        env.rlm_state.clear();

        let child_result = rlm_fractal_inner(subtask.clone(), env, depth + 1, max_depth, deadline);

        // Restore parent state (keep cumulative tokens/calls)
        let child_tokens = env.tokens_used;
        let child_calls = env.llm_calls;
        env.rlm_state = pre_child_state;
        env.tokens_used = child_tokens;
        env.llm_calls = child_calls;

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

    // --- Phase 5: SYNTHESIZE ---
    eprintln!(
        "[rlm depth={}] ★ SYNTHESIZING {} child results",
        depth,
        child_results.len()
    );

    let combined = rlm_synthesize(&task, &child_results, env)?;

    // Restore parent state (preserve token counts)
    merge_rlm_state(env, &saved_state);

    // Optional verification of synthesized result
    if do_verify {
        if let Some(verified) = rlm_verify(&task, &combined, env)? {
            return Ok(verified);
        }
    }

    Ok(combined)
}

/// Result of a single try-solve attempt
/// RED = generation (LLM produced output, needs verification)
/// BLACK = success (output verified via execution)
enum RlmNode {
    Black(LispVal),   // Generated (RED) → Executed → Success → BLACK
    Red(String),      // Generated (RED) → Execution failed → stays RED → trigger split
}

/// Try to solve a task in one shot: generate Lisp code, eval it, check for (final ...)
fn rlm_try_solve(task: &str, env: &mut Env, max_retries: usize) -> RlmNode {
    let sys_prompt = std::env::var("RLM_SYSTEM_PROMPT")
        .unwrap_or_else(|_| RLM_SYSTEM_PROMPT.to_string());

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
                rlm_state_summary(env)
            ),
        ),
    ];

    for attempt in 0..=max_retries {
        // Clear stale AssertPassed from previous iteration so each attempt starts clean
        env.rlm_state.remove("AssertPassed");

        // Call LLM
        let resp = match env.llm_provider.as_ref().unwrap().complete(&messages, Some(8192)) {
            Ok(r) => r,
            Err(e) => return RlmNode::Red(format!("LLM error: {}", e)),
        };
        env.tokens_used += resp.tokens;
        env.llm_calls += 1;

        let code_str = strip_markdown_fences(&resp.content);
        messages.push(("assistant".to_string(), truncate_str(&resp.content, 500)));

        eprintln!("[rlm try {}] code:\n{}", attempt, truncate_str(&code_str, 300));

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
        let snap = env.take_snapshot();
        let mut eval_ok = true;
        let mut err_msg = String::new();
        let mut result = LispVal::Nil;

        for expr in &exprs {
            match lisp_eval(expr, env) {
                Ok(v) => result = v,
                Err(e) => {
                    env.restore_snapshot(snap);
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
        let is_final = env
            .rlm_state
            .get("Final")
            .map(|v| is_truthy(v))
            .unwrap_or(false);
        let is_asserted = env
            .rlm_state
            .get("AssertPassed")
            .map(|v| is_truthy(v))
            .unwrap_or(false);

        if is_final && is_asserted {
            // Generation (RED) → Execution OK + Assertion passed → BLACK
            if let Some(r) = env.rlm_state.get("result") {
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
                     Example: (assert (> result 0)) then (final result)".to_string(),
                ));
                continue;
            }
            return RlmNode::Red("Generation not verified — no (assert ...) before (final ...)".to_string());
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
fn rlm_decompose(task: &str, env: &mut Env) -> Result<Vec<String>, String> {
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

    let resp = env
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(1024))?;
    env.tokens_used += resp.tokens;
    env.llm_calls += 1;

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
    let inner = json_str
        .trim_start_matches('[')
        .trim_end_matches(']');
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

    let resp = env
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(4096))?;
    env.tokens_used += resp.tokens;
    env.llm_calls += 1;

    let code_str = strip_markdown_fences(&resp.content);

    let exprs = parse_all(&code_str).map_err(|e| format!("Synthesis parse error: {}", e))?;

    let snap = env.take_snapshot();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        match lisp_eval(expr, env) {
            Ok(v) => result = v,
            Err(e) => {
                env.restore_snapshot(snap);
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
    if let Some(r) = env.rlm_state.get("result") {
        Ok(r.clone())
    } else {
        Ok(result)
    }
}

/// Verify a result — returns Some(verified_result) if OK, None if failed verification
fn rlm_verify(task: &str, result: &LispVal, env: &mut Env) -> Result<Option<LispVal>, String> {
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

    let resp = env
        .llm_provider
        .as_ref()
        .unwrap()
        .complete(&messages, Some(512))?;
    env.tokens_used += resp.tokens;
    env.llm_calls += 1;

    if resp.content.to_uppercase().starts_with("NO") {
        eprintln!("[rlm verify] FAILED: {}", truncate_str(&resp.content, 200));
        Ok(None) // Verification failed — caller can split
    } else {
        Ok(Some(result.clone()))
    }
}

/// Compact summary of rlm_state for context injection
fn rlm_state_summary(env: &Env) -> String {
    if env.rlm_state.is_empty() {
        return "(empty)".to_string();
    }
    let entries: Vec<String> = env
        .rlm_state
        .iter()
        .map(|(k, v)| format!("{} = {}", k, truncate_str(&v.to_string(), 60)))
        .collect();
    truncate_str(&entries.join(", "), 300).to_string()
}

/// Merge saved parent state back into env, preserving cumulative token/call counts
fn merge_rlm_state(env: &mut Env, saved: &BTreeMap<String, LispVal>) {
    let tokens = env.tokens_used;
    let calls = env.llm_calls;
    env.rlm_state = saved.clone();
    env.tokens_used = tokens;
    env.llm_calls = calls;
}

fn lisp_eval_inner(expr: &LispVal, env: &mut Env) -> Result<LispVal, String> {
    let mut current_expr: LispVal = expr.clone();
    let mut _iter_count: u32 = 0;
    '_trampoline: loop {
        _iter_count += 1;
        if _iter_count % 1000 == 0 {
            eprintln!("[trampoline] iter {}, current_expr={}", _iter_count, truncate_str(&current_expr.to_string(), 80));
        }
        match &current_expr {
            LispVal::Nil
            | LispVal::Bool(_)
            | LispVal::Num(_)
            | LispVal::Float(_)
            | LispVal::Str(_)
            | LispVal::Lambda { .. }
            | LispVal::Macro { .. }
            | LispVal::Map(_) => return Ok(current_expr.clone()),
            LispVal::Recur(_) => return Err("recur outside loop".into()),
            LispVal::Sym(name) => {
                if let Some(v) = env.get(name) {
                    return Ok(v.clone());
                }
                if is_builtin_name(name) {
                    return Ok(current_expr);
                }
                return Err(format!("undefined: {}", name));
            }
            LispVal::List(list) if list.is_empty() => return Ok(LispVal::Nil),
            LispVal::List(list) => {
                if let LispVal::Sym(name) = &list[0] {
                    match name.as_str() {
                        "quote" => return Ok(list.get(1).cloned().unwrap_or(LispVal::Nil)),
                        "quasiquote" => {
                            let expanded =
                                expand_quasiquote(list.get(1).ok_or("quasiquote: need form")?)?;
                            current_expr = expanded;
                            continue '_trampoline;
                        }
                        "define" => {
                            match list.get(1) {
                                // (define (name args...) body) sugar → (define name (lambda (args...) body))
                                Some(LispVal::List(inner)) if !inner.is_empty() => {
                                    if let Some(LispVal::Sym(name)) = inner.get(0) {
                                        let params: Vec<String> = inner[1..]
                                            .iter()
                                            .map(|v| match v {
                                                LispVal::Sym(s) => s.clone(),
                                                _ => "_".to_string(),
                                            })
                                            .collect();
                                        let body = list.get(2).cloned().unwrap_or(LispVal::Nil);
                                        let lam = LispVal::Lambda {
                                            params,
                                            rest_param: None,
                                            body: Box::new(body),
closed_env: std::sync::Arc::new(env.clone().into_bindings()),
                                        };
                                        env.push(name.clone(), lam);
                                        return Ok(LispVal::Nil);
                                    }
                                    return Err("define: need symbol in head position".into());
                                }
                                // (define symbol value)
                                Some(LispVal::Sym(s)) => {
                                    let val = match list.get(2) {
                                        Some(v) => lisp_eval(v, env)?,
                                        None => LispVal::Nil,
                                    };
                                    env.push(s.clone(), val);
                                    return Ok(LispVal::Nil);
                                }
                                _ => return Err("define: need symbol".into()),
                            }
                        }
                        "if" => {
                            let cond = lisp_eval(list.get(1).ok_or("if: need cond")?, env)?;
                            current_expr = if is_truthy(&cond) {
                                list.get(2).ok_or("if: need then")?.clone()
                            } else {
                                list.get(3).cloned().unwrap_or(LispVal::Nil)
                            };
                            continue '_trampoline;
                        }
                        "cond" => {
                            let mut found: Option<LispVal> = None;
                            for clause in &list[1..] {
                                if let LispVal::List(parts) = clause {
                                    if parts.is_empty() {
                                        continue;
                                    }
                                    if let LispVal::Sym(kw) = &parts[0] {
                                        if kw == "else" {
                                            found = parts.get(1).cloned();
                                            break;
                                        }
                                    }
                                    let test = lisp_eval(&parts[0], env)?;
                                    if is_truthy(&test) {
                                        found = Some(parts.get(1).cloned().unwrap_or(test));
                                        break;
                                    }
                                }
                            }
                            match found {
                                Some(e) => {
                                    current_expr = e;
                                    continue '_trampoline;
                                }
                                None => return Ok(LispVal::Nil),
                            }
                        }
                        "let" => {
                            let bindings = match list.get(1) {
                                Some(LispVal::List(b)) => b,
                                _ => return Err("let: bindings must be list".into()),
                            };
                            let base_len = env.len();
                            for b in bindings {
                                if let LispVal::List(pair) = b {
                                    if pair.len() == 2 {
                                        if let LispVal::Sym(name) = &pair[0] {
                                            let val = lisp_eval(&pair[1], env)?;
                                            env.push(name.clone(), val);
                                        }
                                    }
                                }
                            }
                            // Evaluate ALL body forms (list[2..]), return the last one
                            // Use a closure so env.truncate always runs even on error
                            let body_exprs = &list[2..];
                            let result: Result<LispVal, String> = (|| {
                                if body_exprs.is_empty() {
                                    return Ok(LispVal::Nil);
                                }
                                let mut r = LispVal::Nil;
                                for e in body_exprs {
                                    r = lisp_eval(e, env)?;
                                }
                                Ok(r)
                            })();
                            env.truncate(base_len);
                            return result;
                        }
                        "lambda" => {
                            let (params, rest_param) =
                                parse_params(list.get(1).ok_or("lambda: need params")?)?;
                            let body = list.get(2).ok_or("lambda: need body")?;
                            return Ok(LispVal::Lambda {
                                params,
                                rest_param,
                                body: Box::new(body.clone()),
closed_env: std::sync::Arc::new(env.clone().into_bindings()),
                            });
                        }
                        "defmacro" => {
                            let macro_name = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                _ => return Err("defmacro: first arg must be symbol".into()),
                            };
                            let (params, rest_param) =
                                parse_params(list.get(2).ok_or("defmacro: need params")?)?;
                            let body = list.get(3).ok_or("defmacro: need body")?;
                            env.push(
                                macro_name,
                                LispVal::Macro {
                                    params,
                                    rest_param,
                                    body: Box::new(body.clone()),
closed_env: std::sync::Arc::new(env.clone().into_bindings()),
                                },
                            );
                            return Ok(LispVal::Nil);
                        }
                        "progn" | "begin" => {
                            let exprs = &list[1..];
                            if exprs.is_empty() {
                                return Ok(LispVal::Nil);
                            }
                            for e in &exprs[..exprs.len() - 1] {
                                lisp_eval(e, env)?;
                            }
                            current_expr = exprs.last().unwrap().clone();
                            continue '_trampoline;
                        }
                        "and" => {
                            if list.len() == 1 {
                                return Ok(LispVal::Bool(true));
                            }
                            let exprs = &list[1..];
                            for e in &exprs[..exprs.len() - 1] {
                                let r = lisp_eval(e, env)?;
                                if !is_truthy(&r) {
                                    return Ok(r);
                                }
                            }
                            current_expr = exprs.last().unwrap().clone();
                            continue '_trampoline;
                        }
                        "or" => {
                            if list.len() == 1 {
                                return Ok(LispVal::Bool(false));
                            }
                            let exprs = &list[1..];
                            for e in &exprs[..exprs.len() - 1] {
                                let r = lisp_eval(e, env)?;
                                if is_truthy(&r) {
                                    return Ok(r);
                                }
                            }
                            current_expr = exprs.last().unwrap().clone();
                            continue '_trampoline;
                        }
                        "not" => {
                            let v = lisp_eval(list.get(1).ok_or("not: need arg")?, env)?;
                            return Ok(LispVal::Bool(!is_truthy(&v)));
                        }
                        "try" => {
                            let expr_to_try = list.get(1).ok_or("try: need expression")?;
                            let res = match lisp_eval(expr_to_try, env) {
                                Ok(val) => return Ok(val),
                                Err(err_msg) => {
                                    let catch_clause =
                                        list.get(2).ok_or("try: need catch clause")?;
                                    if let LispVal::List(clause) = catch_clause {
                                        if clause.is_empty()
                                            || clause[0] != LispVal::Sym("catch".into())
                                        {
                                            return Err(
                                                "try: second arg must be (catch var body...)"
                                                    .into(),
                                            );
                                        }
                                        let error_var = match clause.get(1) {
                                            Some(LispVal::Sym(s)) => s.clone(),
                                            _ => {
                                                return Err(
                                                    "try: catch needs a variable name".into()
                                                )
                                            }
                                        };
                                        env.push(error_var.clone(), LispVal::Str(err_msg));
                                        let base_len = env.len();
                                        let catch_result: Result<LispVal, String> = (|| {
                                            let mut r = LispVal::Nil;
                                            for body_expr in &clause[2..] {
                                                r = lisp_eval(body_expr, env)?;
                                            }
                                            Ok(r)
                                        })();
                                        env.truncate(base_len);
                                        catch_result?
                                    } else {
                                        return Err("try: catch clause must be a list".into());
                                    }
                                }
                            };
                            return Ok(res);
                        }
                        "match" => {
                            let val = lisp_eval(list.get(1).ok_or("match: need expr")?, env)?;
                            let mut matched: Option<(Vec<(String, LispVal)>, LispVal)> = None;
                            for clause in &list[2..] {
                                if let LispVal::List(parts) = clause {
                                    if parts.len() >= 2 {
                                        if let Some(bindings) = match_pattern(&parts[0], &val) {
                                            matched = Some((
                                                bindings,
                                                parts.get(1).cloned().unwrap_or(LispVal::Nil),
                                            ));
                                            break;
                                        }
                                    }
                                }
                            }
                            match matched {
                                Some((bindings, body)) => {
                                    let base_len = env.len();
                                    for (name, v) in bindings {
                                        env.push(name, v);
                                    }
                                    let result = lisp_eval(&body, env);
                                    env.truncate(base_len);
                                    return result;
                                }
                                None => return Ok(LispVal::Nil),
                            }
                        }
                        "loop" => {
                            let bindings = match list.get(1) {
                                Some(LispVal::List(b)) => b,
                                _ => return Err("loop: bindings must be list".into()),
                            };
                            let body = list.get(2).ok_or("loop: need body")?;
                            let mut binding_names: Vec<String> = Vec::new();
                            let mut binding_vals: Vec<LispVal> = Vec::new();
                            let is_pair_style =
                                bindings.iter().all(|b| matches!(b, LispVal::List(_)));
                            if is_pair_style {
                                for b in bindings {
                                    if let LispVal::List(pair) = b {
                                        if pair.len() == 2 {
                                            if let LispVal::Sym(name) = &pair[0] {
                                                binding_names.push(name.clone());
                                                binding_vals.push(lisp_eval(&pair[1], env)?);
                                            }
                                        }
                                    }
                                }
                            } else {
                                if bindings.len() % 2 != 0 {
                                    return Err("loop: flat bindings need even count".into());
                                }
                                let mut i = 0;
                                while i < bindings.len() {
                                    if let LispVal::Sym(name) = &bindings[i] {
                                        binding_names.push(name.clone());
                                        binding_vals.push(lisp_eval(&bindings[i + 1], env)?);
                                    } else {
                                        return Err(format!(
                                            "loop: binding name must be sym, got {}",
                                            bindings[i]
                                        ));
                                    }
                                    i += 2;
                                }
                            }
                            let result = loop {
                                let base_len = env.len();
                                for (i, name) in binding_names.iter().enumerate() {
                                    env.push(name.clone(), binding_vals[i].clone());
                                }
                                let result = lisp_eval(body, env);
                                env.truncate(base_len);
                                match result? {
                                    LispVal::Recur(new_vals) => {
                                        if new_vals.len() != binding_names.len() {
                                            return Err(format!(
                                                "recur: expected {} args, got {}",
                                                binding_names.len(),
                                                new_vals.len()
                                            ));
                                        }
                                        binding_vals = new_vals;
                                    }
                                    other => break other,
                                }
                            };
                            return Ok(result);
                        }
                        "recur" => {
                            let vals: Vec<LispVal> = list[1..]
                                .iter()
                                .map(|a| lisp_eval(a, env))
                                .collect::<Result<_, _>>()?;
                            return Ok(LispVal::Recur(vals));
                        }
                        "require" => {
                            let module_name = match list.get(1) {
                                Some(LispVal::Str(s)) => s.as_str(),
                                _ => return Err("require: need string module name".into()),
                            };
                            let prefix: Option<&str> = match list.get(2) {
                                Some(LispVal::Str(s)) => Some(s.as_str()),
                                None => None,
                                _ => return Err("require: prefix must be string".into()),
                            };
                            let marker =
                                format!("__loaded_{}__{}", module_name, prefix.unwrap_or(""));
                            if env.contains(&marker) {
                                return Ok(LispVal::Nil);
                            }
                            // Try stdlib first, then file path
                            let code: String =
                                if let Some(stdlib_code) = get_stdlib_code(module_name) {
                                    stdlib_code.to_string()
                                } else {
                                    // File-based loading: resolve relative to RLM_MODULE_PATH or cwd
                                    let path = if module_name.starts_with('/')
                                        || module_name.starts_with("./")
                                        || module_name.starts_with("../")
                                    {
                                        module_name.to_string()
                                    } else {
                                        let base = std::env::var("RLM_MODULE_PATH")
                                            .unwrap_or_else(|_| ".".to_string());
                                        format!("{}/{}.lisp", base, module_name)
                                    };
                                    std::fs::read_to_string(&path).map_err(|e| {
                                        format!("require: cannot load '{}': {}", path, e)
                                    })?
                                };
                            if let Some(pfx) = prefix {
                                let mut module_env = Env::new();
                                let module_exprs = parse_all(&code)?;
                                for expr in &module_exprs {
                                    lisp_eval(expr, &mut module_env)?;
                                }
                                // If module defines __exports__, only import those; otherwise import all
                                let exports: Option<Vec<String>> =
                                    module_env.get("__exports__").and_then(|v| match v {
                                        LispVal::List(items) => Some(
                                            items
                                                .iter()
                                                .filter_map(|i| match i {
                                                    LispVal::Str(s) => Some(s.clone()),
                                                    LispVal::Sym(s) => Some(s.clone()),
                                                    _ => None,
                                                })
                                                .collect(),
                                        ),
                                        _ => None,
                                    });
                                let bindings = module_env.into_bindings();
                                for (k, v) in &bindings {
                                    if k.starts_with("__") {
                                        continue;
                                    } // skip internals
                                    if let Some(ref exp) = exports {
                                        if !exp.contains(&k) {
                                            continue;
                                        }
                                    }
                                    env.push(format!("{}/{}", pfx, k), v.clone());
                                }
                            } else {
                                let module_exprs = parse_all(&code)?;
                                for expr in &module_exprs {
                                    lisp_eval(expr, env)?;
                                }
                            }
                            env.push(marker, LispVal::Bool(true));
                            return Ok(LispVal::Nil);
                        }
                        "export" => {
                            // (export sym1 sym2 ...) or (export "sym1" "sym2" ...)
                            let names: Vec<String> = list[1..]
                                .iter()
                                .map(|a| match a {
                                    LispVal::Sym(s) => s.clone(),
                                    LispVal::Str(s) => s.clone(),
                                    other => format!("{}", other),
                                })
                                .collect();
                            let existing = env.get("__exports__").cloned();
                            let merged = match existing {
                                Some(LispVal::List(mut items)) => {
                                    for n in &names {
                                        if !items.iter().any(|i| match i {
                                            LispVal::Str(s) => s == n,
                                            LispVal::Sym(s) => s == n,
                                            _ => false,
                                        }) {
                                            items.push(LispVal::Str(n.clone()));
                                        }
                                    }
                                    LispVal::List(items)
                                }
                                _ => LispVal::List(names.into_iter().map(LispVal::Str).collect()),
                            };
                            env.push("__exports__".to_string(), merged);
                            return Ok(LispVal::Bool(true));
                        }
                        "final" => {
                            let val = lisp_eval(list.get(1).ok_or("final: need value")?, env)?;
                            env.rlm_state
                                .insert("Final".to_string(), LispVal::Bool(true));
                            env.rlm_state.insert("result".to_string(), val);
                            return Ok(LispVal::Bool(true));
                        }
                        "final-var" => {
                            let var_name = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                Some(LispVal::Str(s)) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "final-var: need symbol or string, got {:?}",
                                        other
                                    ))
                                }
                            };
                            let val = env.get(&var_name).cloned().ok_or_else(|| {
                                format!("final-var: undefined variable '{}'", var_name)
                            })?;
                            env.rlm_state
                                .insert("Final".to_string(), LispVal::Bool(true));
                            env.rlm_state.insert("result".to_string(), val);
                            return Ok(LispVal::Bool(true));
                        }
                        "assert" => {
                            let condition = lisp_eval(list.get(1).ok_or("assert: need condition")?, env)?;
                            if is_truthy(&condition) {
                                env.rlm_state
                                    .insert("AssertPassed".to_string(), LispVal::Bool(true));
                                return Ok(LispVal::Bool(true));
                            } else {
                                return Err(format!(
                                    "assert failed: {}",
                                    truncate_str(&list[1].to_string(), 100)
                                ));
                            }
                        }
                        "rlm-set" => {
                            let key = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                Some(LispVal::Str(s)) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "rlm-set: key must be symbol or string, got {:?}",
                                        other
                                    ))
                                }
                            };
                            let val = match list.get(2) {
                                Some(v) => lisp_eval(v, env)?,
                                None => LispVal::Nil,
                            };
                            env.rlm_state.insert(key, val);
                            return Ok(LispVal::Bool(true));
                        }
                        "rlm-get" => {
                            let key = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                Some(LispVal::Str(s)) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "rlm-get: key must be symbol or string, got {:?}",
                                        other
                                    ))
                                }
                            };
                            return Ok(env.rlm_state.get(&key).cloned().unwrap_or(LispVal::Nil));
                        }
                        "set!" => {
                            let name = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                _ => return Err("set!: need symbol".into()),
                            };
                            let val = lisp_eval(list.get(2).ok_or("set!: need value")?, env)?;
                            if let Some(slot) = env.get_mut(&name) {
                                *slot = val;
                                return Ok(LispVal::Nil);
                            } else {
                                return Err(format!("set!: undefined variable '{}'", name));
                            }
                        }
                        _ => return dispatch_call(list, env),
                    }
                } else {
                    return dispatch_call(list, env);
                }
            }
        }
    }
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
    closed_env: &std::sync::Arc<Vec<(String, LispVal)>>,
    args: &[LispVal],
    caller_env: &mut Env,
) -> Result<LispVal, String> {
    let base_len = caller_env.len();

    // Track any existing bindings that get overwritten by closed_env/params
    // so we can restore them after evaluation. This is necessary because
    // Env::push() updates existing bindings in-place, and truncate() only
    // removes entries appended past base_len — it can't undo in-place updates.
    let mut saved: Vec<(String, Option<LispVal>)> = Vec::new();

    // Helper: push a binding, saving any existing value first
    macro_rules! push_saving {
        ($name:expr, $val:expr) => {{
            let name = $name;
            let existing = caller_env.get(name).cloned();
            let existed = caller_env.contains(name);
            caller_env.push(name.to_string(), $val);
            if existed {
                saved.push((name.to_string(), existing));
            }
        }};
    }

    for (k, v) in closed_env.iter() {
        push_saving!(k, v.clone());
    }
    for (i, p) in params.iter().enumerate() {
        push_saving!(p, args.get(i).cloned().unwrap_or(LispVal::Nil));
    }
    if let Some(rest_name) = rest_param {
        let rest_args: Vec<LispVal> = args.get(params.len()..).unwrap_or(&[]).to_vec();
        push_saving!(rest_name, LispVal::List(rest_args));
    }

    let result = {
        eprintln!("[apply_lambda] BEFORE lisp_eval, body={}, env len={}, eval_count={}", 
            truncate_str(&body.to_string(), 60), caller_env.len(), caller_env.eval_count);
        let r = lisp_eval(body, caller_env);
        eprintln!("[apply_lambda] AFTER lisp_eval, result={:?}", r.as_ref().map(|v| truncate_str(&v.to_string(), 50)));
        r
    };

    // Restore: truncate any new bindings, then restore overwritten ones
    caller_env.truncate(base_len);
    for (name, orig_val) in saved.into_iter().rev() {
        match orig_val {
            Some(v) => caller_env.push(name, v),
            None => {
                // Wasn't in env before push — but push() created it.
                // After truncate it's gone if it was appended past base_len.
                // If it was created by updating an index that existed at base_len,
                // truncate already restored it. No action needed for the None case
                // because the binding was new and truncate removed it.
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Function dispatch
// ---------------------------------------------------------------------------

fn dispatch_call(list: &[LispVal], env: &mut Env) -> Result<LispVal, String> {
    let head = &list[0];
    let raw_args: Vec<LispVal> = list[1..].to_vec();

    // Check if head resolves to a Macro — macros get unevaluated args
    if let LispVal::Sym(name) = head {
        if let Some(func) = env.get(name) {
            if matches!(func, LispVal::Macro { .. }) {
                let func_clone = func.clone();
                return call_val(&func_clone, &raw_args, env);
            }
        }
    }

    // Normal path: evaluate args
    let args: Vec<LispVal> = raw_args
        .iter()
        .map(|a| lisp_eval(a, env))
        .collect::<Result<_, _>>()?;

    if let LispVal::Sym(name) = head {
        // ── Dispatch chain: delegate to domain modules ──
        if let Some(result) = dispatch_arithmetic::handle(name, &args)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_collections::handle(name, &args, env)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_strings::handle(name, &args)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_predicates::handle(name, &args)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_json::handle(name, &args)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_http::handle(name, &args)? {
            return Ok(result);
        }
        if let Some(result) = dispatch_state::handle(name, &args, env)? {
            return Ok(result);
        }

        // ── Inline builtins: crypto + LLM/RLM ──
        match name.as_str() {
            "sha256" => builtin_sha256(&args),
            "keccak256" => builtin_keccak256(&args),

            // --- LLM builtins ---

            // --- LLM builtins ---
            "llm" => {
                let prompt = as_str(&args[0])?;
                let messages = vec![
                    ("system".to_string(), "You are a helpful assistant with access to a Lisp runtime called lisp-rlm.".to_string()),
                    ("user".to_string(), prompt),
                ];
                let resp = env
                    .llm_provider
                    .as_ref()
                    .ok_or("llm: no LLM provider configured")?
                    .complete(&messages, Some(2048))?;
                env.tokens_used += resp.tokens;
                env.llm_calls += 1;
                Ok(LispVal::Str(resp.content))
            }
            "llm-code" => {
                let prompt = as_str(&args[0])?;
                let messages = vec![
                    ("system".to_string(), RLM_SYSTEM_PROMPT.to_string()),
                    ("user".to_string(), prompt),
                ];
                let resp = env
                    .llm_provider
                    .as_ref()
                    .ok_or("llm-code: no LLM provider configured")?
                    .complete(&messages, Some(2048))?;

                env.tokens_used += resp.tokens;
                env.llm_calls += 1;

                let code_str = strip_markdown_fences(&resp.content);

                // Parse and eval the LLM-generated Lisp code
                let exprs = parse_all(&code_str)?;
                let mut result = LispVal::Nil;
                for expr in &exprs {
                    result = lisp_eval(expr, env)?;
                }
                Ok(result)
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
                rlm_fractal(task, env, 0, max_depth)
            }

            // --- Sub-RLM: delegates to the same fractal loop ---
            "sub-rlm" => {
                let sub_task = as_str(&args[0])?;
                if env.rlm_depth >= 5 {
                    return Err("sub-rlm: max depth (5) exceeded".into());
                }
                env.rlm_depth += 1;
                let max_depth: usize = std::env::var("RLM_MAX_DEPTH")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(6);
                let result = rlm_fractal(sub_task, env, 0, max_depth);
                env.rlm_depth -= 1;
                match &result {
                    Ok(v) => Ok(LispVal::Str(v.to_string())),
                    Err(e) => Ok(LispVal::Str(format!("error: {}", e))),
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
                Ok(LispVal::Str(entries.join("\n")))
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
                    let result = lisp_eval(&call, env)?;
                    results.push(result);
                }
                Ok(LispVal::List(results))
            }
            "show-context" => {
                let context_val = env
                    .rlm_state
                    .get("context")
                    .cloned()
                    .unwrap_or(LispVal::Nil);
                let context_str = context_val.to_string();
                let preview = truncate_str(&context_str, 200);
                let final_set = env
                    .rlm_state
                    .get("Final")
                    .map(|v| is_truthy(v))
                    .unwrap_or(false);
                Ok(LispVal::Str(format!(
                    "Context length: {} chars\nPreview: {}\nIteration: {}\nFinal set: {}",
                    context_str.len(),
                    preview,
                    env.rlm_iteration,
                    final_set
                )))
            }

            // --- Token tracking ---
            "rlm-tokens" => Ok(LispVal::Num(env.tokens_used as i64)),
            "rlm-calls" => Ok(LispVal::Num(env.llm_calls as i64)),
            "rlm-write" => {
                // Like (rlm "task") but returns the generated code as a string
                // Also saves to file if path is provided as second arg
                let task = as_str(&args[0])?;
                let save_path = args.get(1).map(|v| as_str(v)).transpose()?;

                if env.llm_provider.is_none() {
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
try catch error

DO NOT wrap code in markdown fences. DO NOT add explanations."#;

                // First call: initial code generation
                let gen_messages = vec![
                    ("system".to_string(), sys.to_string()),
                    ("user".to_string(), task.clone()),
                ];
                let gen_resp = env
                    .llm_provider
                    .as_ref()
                    .unwrap()
                    .complete(&gen_messages, Some(8192))?;
                env.tokens_used += gen_resp.tokens;
                env.llm_calls += 1;
                let code = strip_markdown_fences(&gen_resp.content);

                // Verify parse, retry once if broken
                let final_code = if crate::parser::parse_all(&code).is_err() {
                    let fix_messages = vec![
                        ("system".to_string(), sys.to_string()),
                        ("assistant".to_string(), code.clone()),
                        ("user".to_string(), "The previous code had a parse error. Write it again, fixed. Return ONLY valid raw Lisp code, no markdown, no explanations.".to_string()),
                    ];
                    let fix_resp = env
                        .llm_provider
                        .as_ref()
                        .unwrap()
                        .complete(&fix_messages, Some(8192))?;
                    env.tokens_used += fix_resp.tokens;
                    env.llm_calls += 1;
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

                Ok(LispVal::Str(final_code))
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
                Ok(LispVal::Map(BTreeMap::from([
                    ("name".to_string(), LispVal::Str(sig_name)),
                    (
                        "inputs".to_string(),
                        LispVal::List(inputs.into_iter().map(LispVal::Str).collect()),
                    ),
                    (
                        "outputs".to_string(),
                        LispVal::List(outputs.into_iter().map(LispVal::Str).collect()),
                    ),
                ])))
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
                Ok(LispVal::Str(prompt))
            }
            "rlm/trace" => {
                let step = as_str(&args[0])?;
                let data = &args[1];
                eprintln!("[RLM] {}: {}", step, data);
                Ok(LispVal::Bool(true))
            }
            "rlm/config" => {
                let key = as_str(&args[0])?;
                let val = args[1].clone();
                env.push(format!("__rlm_{}__", key), val);
                Ok(LispVal::Bool(true))
            }

            _ => {
                let func = env
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("undefined: {}", name))?;
                call_val(&func, &args, env)
            }
        }
    } else if let LispVal::Lambda {
        params,
        rest_param,
        body,
        closed_env,
    } = head
    {
        apply_lambda(params, &rest_param, body, closed_env, &args, env)
    } else if let LispVal::List(ll) = head {
        if ll.len() < 3 {
            return Err("inline lambda too short".into());
        }
        let (params, rest_param) = parse_params(&ll[1])?;
        apply_lambda(&params, &rest_param, &ll[2], &std::sync::Arc::new(vec![]), &args, env)
    } else {
        Err("not callable".into())
    }
}

fn call_val(func: &LispVal, args: &[LispVal], env: &mut Env) -> Result<LispVal, String> {
    match func {
        LispVal::Lambda {
            params,
            rest_param,
            body,
            closed_env,
        } => {
            eprintln!("[call_val] applying lambda with {} params, closed_env len {}", params.len(), closed_env.len());
            let result = apply_lambda(params, rest_param, body, closed_env, args, env);
            eprintln!("[call_val] lambda result: {:?}", result.as_ref().map(|v| truncate_str(&v.to_string(), 50)));
            result
        }
        LispVal::Macro {
            params,
            rest_param,
            body,
            closed_env,
        } => {
            // Macros receive UNEVALUATED args, return code to be evaluated
            let expanded = apply_lambda(params, rest_param, body, closed_env, args, env)?;
            lisp_eval(&expanded, env)
        }
        LispVal::List(ll) if ll.len() >= 3 => {
            let (params, rest_param) = parse_params(&ll[1])?;
            apply_lambda(&params, &rest_param, &ll[2], &std::sync::Arc::new(vec![]), args, env)
        }
        LispVal::Sym(_) => {
            let mut call = vec![func.clone()];
            call.extend(args.iter().cloned());
            dispatch_call(&call, env)
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
        let mut result = LispVal::Nil;
        for expr in &exprs {
            result = lisp_eval(expr, &mut env)?;
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
