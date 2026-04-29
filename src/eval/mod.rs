use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::helpers::*;
use crate::parser::parse_all;
use crate::types::{Env, EvalState, LispVal};
pub mod crypto;
pub mod helpers;
pub mod llm_provider;
pub mod quasiquote;

pub mod dispatch_arithmetic;
// Domain-specific dispatch modules (v0.2 god-function split)
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


/// Evaluate a single Lisp expression (delegates to VM via run_program).
pub fn lisp_eval(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    crate::program::run_program(&[expr.clone()], env, state)
}

/// Apply a function value to arguments (delegates to VM).
pub fn apply_lambda(
    func: &LispVal,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::vm_call_lambda(func, args, env, state)
}

/// Dispatch a builtin call by name with evaluated arguments.
pub fn dispatch_call_with_args(
    name: &str,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::eval_builtin(name, args, Some(env), Some(state))
}

/// Call a function value (delegates to VM).
pub fn call_val(
    func: &LispVal,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::vm_call_lambda(func, args, env, state)
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
        // After tree-walker removal, llm/llm-code are not wired in eval_builtin
        assert!(err.contains("unknown builtin"));
    }

    #[test]
    fn test_llm_code_no_api_key() {
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        let r = eval_str(r#"(llm-code "compute 2+2")"#);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(err.contains("unknown builtin"));
    }
}
