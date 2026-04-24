use std::collections::BTreeMap;

use std::sync::LazyLock;

use crate::helpers::*;
use crate::parser::parse_all;
use crate::types::{get_stdlib_code, Env, LispVal};

pub mod crypto;
pub mod helpers;
pub mod quasiquote;

use crypto::{builtin_sha256, builtin_keccak256};
use helpers::{truncate_str, strip_markdown_fences, extract_first_valid_expr};
use quasiquote::expand_quasiquote;

/// Shared tokio runtime — avoids creating a new runtime per LLM/HTTP call.
static SHARED_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().expect("failed to create tokio runtime")
});

/// Shared reqwest client with a 60s timeout — reused across all HTTP/LLM calls.
static SHARED_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to create reqwest client")
});

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
            } else if let Some(f) = n.as_f64() {
                LispVal::Float(f)
            } else if let Some(u) = n.as_u64() {
                LispVal::Num(u as i64)
            } else {
                LispVal::Num(0)
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
            let obj: serde_json::Map<String, serde_json::Value> =
                m.iter().map(|(k, v)| (k.clone(), lisp_to_json(v))).collect();
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
IO: print println read-file write-file append-file file-exists? shell
HTTP: http-get http-post http-get-json
JSON: from-json to-json json-parse json-get json-get-in json-build
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

fn lisp_eval_inner(expr: &LispVal, env: &mut Env) -> Result<LispVal, String> {
    let mut current_expr: LispVal = expr.clone();
    '_trampoline: loop {
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
                            let expanded = expand_quasiquote(list.get(1).ok_or("quasiquote: need form")?)?;
                            current_expr = expanded;
                            continue '_trampoline;
                        }
                        "define" => {
                            match list.get(1) {
                                // (define (name args...) body) sugar → (define name (lambda (args...) body))
                                Some(LispVal::List(inner)) if !inner.is_empty() => {
                                    if let Some(LispVal::Sym(name)) = inner.get(0) {
                                        let params: Vec<String> = inner[1..].iter().map(|v| match v {
                                            LispVal::Sym(s) => s.clone(),
                                            _ => "_".to_string(),
                                        }).collect();
                                        let body = list.get(2).cloned().unwrap_or(LispVal::Nil);
                                        let lam = LispVal::Lambda {
                                            params,
                                            rest_param: None,
                                            body: Box::new(body),
                                            closed_env: Box::new(env.clone().into_bindings()),
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
                                    if parts.is_empty() { continue; }
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
                                Some(e) => { current_expr = e; continue '_trampoline; }
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
                            let result = list.get(2).map(|e| lisp_eval(e, env)).unwrap_or(Ok(LispVal::Nil));
                            env.truncate(base_len);
                            return result;
                        }
                        "lambda" => {
                            let (params, rest_param) = parse_params(list.get(1).ok_or("lambda: need params")?)?;
                            let body = list.get(2).ok_or("lambda: need body")?;
                            return Ok(LispVal::Lambda {
                                params,
                                rest_param,
                                body: Box::new(body.clone()),
                                closed_env: Box::new(env.clone().into_bindings()),
                            });
                        }
                        "defmacro" => {
                            let macro_name = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                _ => return Err("defmacro: first arg must be symbol".into()),
                            };
                            let (params, rest_param) = parse_params(list.get(2).ok_or("defmacro: need params")?)?;
                            let body = list.get(3).ok_or("defmacro: need body")?;
                            env.push(macro_name, LispVal::Macro {
                                params,
                                rest_param,
                                body: Box::new(body.clone()),
                                closed_env: Box::new(env.clone().into_bindings()),
                            });
                            return Ok(LispVal::Nil);
                        }
                        "progn" | "begin" => {
                            let exprs = &list[1..];
                            if exprs.is_empty() { return Ok(LispVal::Nil); }
                            for e in &exprs[..exprs.len() - 1] {
                                lisp_eval(e, env)?;
                            }
                            current_expr = exprs.last().unwrap().clone();
                            continue '_trampoline;
                        }
                        "and" => {
                            if list.len() == 1 { return Ok(LispVal::Bool(true)); }
                            let exprs = &list[1..];
                            for e in &exprs[..exprs.len() - 1] {
                                let r = lisp_eval(e, env)?;
                                if !is_truthy(&r) { return Ok(r); }
                            }
                            current_expr = exprs.last().unwrap().clone();
                            continue '_trampoline;
                        }
                        "or" => {
                            if list.len() == 1 { return Ok(LispVal::Bool(false)); }
                            let exprs = &list[1..];
                            for e in &exprs[..exprs.len() - 1] {
                                let r = lisp_eval(e, env)?;
                                if is_truthy(&r) { return Ok(r); }
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
                                    let catch_clause = list.get(2).ok_or("try: need catch clause")?;
                                    if let LispVal::List(clause) = catch_clause {
                                        if clause.is_empty() || clause[0] != LispVal::Sym("catch".into()) {
                                            return Err("try: second arg must be (catch var body...)".into());
                                        }
                                        let error_var = match clause.get(1) {
                                            Some(LispVal::Sym(s)) => s.clone(),
                                            _ => return Err("try: catch needs a variable name".into()),
                                        };
                                        env.push(error_var, LispVal::Str(err_msg));
                                        let base_len = env.len() - 1;
                                        let mut r = LispVal::Nil;
                                        for body_expr in &clause[2..] {
                                            r = lisp_eval(body_expr, env)?;
                                        }
                                        env.truncate(base_len);
                                        r
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
                                            matched = Some((bindings, parts.get(1).cloned().unwrap_or(LispVal::Nil)));
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
                            let is_pair_style = bindings.iter().all(|b| matches!(b, LispVal::List(_)));
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
                                        return Err(format!("loop: binding name must be sym, got {}", bindings[i]));
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
                                            return Err(format!("recur: expected {} args, got {}", binding_names.len(), new_vals.len()));
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
                            let marker = format!("__stdlib_{}__{}", module_name, prefix.unwrap_or(""));
                            if env.contains(&marker) {
                                return Ok(LispVal::Nil);
                            }
                            if let Some(code) = get_stdlib_code(module_name) {
                                if let Some(pfx) = prefix {
                                    let mut module_env = Env::new();
                                    let module_exprs = parse_all(code)?;
                                    for expr in &module_exprs {
                                        lisp_eval(expr, &mut module_env)?;
                                    }
                                    for (k, v) in module_env.into_bindings() {
                                        env.push(format!("{}/{}", pfx, k), v);
                                    }
                                } else {
                                    let module_exprs = parse_all(code)?;
                                    for expr in &module_exprs {
                                        lisp_eval(expr, env)?;
                                    }
                                }
                                env.push(marker, LispVal::Bool(true));
                                return Ok(LispVal::Nil);
                            }
                            return Err(format!("require: unknown module '{}'", module_name));
                        }
                        "final" => {
                            let val = lisp_eval(list.get(1).ok_or("final: need value")?, env)?;
                            env.rlm_state.insert("Final".to_string(), LispVal::Bool(true));
                            env.rlm_state.insert("result".to_string(), val);
                            return Ok(LispVal::Bool(true));
                        }
                        "final-var" => {
                            let var_name = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                Some(LispVal::Str(s)) => s.clone(),
                                other => return Err(format!("final-var: need symbol or string, got {:?}", other)),
                            };
                            let val = env.get(&var_name).cloned().ok_or_else(|| format!("final-var: undefined variable '{}'", var_name))?;
                            env.rlm_state.insert("Final".to_string(), LispVal::Bool(true));
                            env.rlm_state.insert("result".to_string(), val);
                            return Ok(LispVal::Bool(true));
                        }
                        "rlm-set" => {
                            let key = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                Some(LispVal::Str(s)) => s.clone(),
                                other => return Err(format!("rlm-set: key must be symbol or string, got {:?}", other)),
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
                                other => return Err(format!("rlm-get: key must be symbol or string, got {:?}", other)),
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
    closed_env: &Vec<(String, LispVal)>,
    args: &[LispVal],
    caller_env: &mut Env,
) -> Result<LispVal, String> {
    let base_len = caller_env.len();
    for (k, v) in closed_env {
        caller_env.push(k.clone(), v.clone());
    }
    for (i, p) in params.iter().enumerate() {
        caller_env.push(p.clone(), args.get(i).cloned().unwrap_or(LispVal::Nil));
    }
    if let Some(rest_name) = rest_param {
        let rest_args: Vec<LispVal> = args.get(params.len()..).unwrap_or(&[]).to_vec();
        caller_env.push(rest_name.clone(), LispVal::List(rest_args));
    }
    let result = lisp_eval(body, caller_env);
    caller_env.truncate(base_len);
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
        match name.as_str() {
            "+" => do_arith(&args, |a, b| a + b, |a, b| a + b),
            "-" => do_arith(&args, |a, b| a - b, |a, b| a - b),
            "*" => do_arith(&args, |a, b| a * b, |a, b| a * b),
            "/" => {
                if any_float(&args) {
                    let b = as_float(args.get(1).ok_or("/ needs 2 args")?)?;
                    if b == 0.0 { return Err("div by zero".into()); }
                    Ok(LispVal::Float(as_float(&args[0])? / b))
                } else {
                    let b = as_num(args.get(1).ok_or("/ needs 2 args")?)?;
                    if b == 0 { return Err("div by zero".into()); }
                    Ok(LispVal::Num(as_num(&args[0])? / b))
                }
            }
            "mod" => do_arith(&args, |a, b| i64::rem_euclid(a, b), |a, b| a % b),
            "=" | "==" => {
                if any_float(&args) {
                    Ok(LispVal::Bool(as_float(&args[0])? == as_float(&args[1])?))
                } else {
                    Ok(LispVal::Bool(args.get(0) == args.get(1)))
                }
            }
            "!=" | "/=" => {
                if any_float(&args) {
                    Ok(LispVal::Bool(as_float(&args[0])? != as_float(&args[1])?))
                } else {
                    Ok(LispVal::Bool(args.get(0) != args.get(1)))
                }
            }
            "<" => {
                if any_float(&args) { Ok(LispVal::Bool(as_float(&args[0])? < as_float(&args[1])?)) }
                else { Ok(LispVal::Bool(as_num(&args[0])? < as_num(&args[1])?)) }
            }
            ">" => {
                if any_float(&args) { Ok(LispVal::Bool(as_float(&args[0])? > as_float(&args[1])?)) }
                else { Ok(LispVal::Bool(as_num(&args[0])? > as_num(&args[1])?)) }
            }
            "<=" => {
                if any_float(&args) { Ok(LispVal::Bool(as_float(&args[0])? <= as_float(&args[1])?)) }
                else { Ok(LispVal::Bool(as_num(&args[0])? <= as_num(&args[1])?)) }
            }
            ">=" => {
                if any_float(&args) { Ok(LispVal::Bool(as_float(&args[0])? >= as_float(&args[1])?)) }
                else { Ok(LispVal::Bool(as_num(&args[0])? >= as_num(&args[1])?)) }
            }
            "list" => Ok(LispVal::List(args)),
            "car" => match args.get(0) {
                Some(LispVal::List(l)) if !l.is_empty() => Ok(l[0].clone()),
                _ => Ok(LispVal::Nil),
            },
            "cdr" => match args.get(0) {
                Some(LispVal::List(l)) if l.len() > 1 => Ok(LispVal::List(l[1..].to_vec())),
                Some(LispVal::List(_)) => Ok(LispVal::List(vec![])),  // empty tail → ()
                _ => Ok(LispVal::Nil),
            },
            "cons" => match args.get(1) {
                Some(LispVal::List(l)) => {
                    let mut n = vec![args[0].clone()];
                    n.extend(l.iter().cloned());
                    Ok(LispVal::List(n))
                }
                _ => Ok(LispVal::List(args)),
            },
            "len" => match args.get(0) {
                Some(LispVal::List(l)) => Ok(LispVal::Num(l.len() as i64)),
                Some(LispVal::Str(s)) => Ok(LispVal::Num(s.len() as i64)),
                Some(LispVal::Nil) => Ok(LispVal::Num(0)),
                _ => Err("len: need list or string".into()),
            },
            "append" => {
                let mut r = Vec::new();
                for a in &args {
                    if let LispVal::List(l) = a { r.extend(l.iter().cloned()); }
                    else { r.push(a.clone()); }
                }
                Ok(LispVal::List(r))
            }
            "nth" => {
                // Standard Lisp order: (nth list index)
                let list_val = args.get(0).ok_or("nth: need list and index")?;
                let i = as_num(args.get(1).ok_or("nth: need index")?)? as usize;
                match list_val {
                    LispVal::List(l) => l.get(i).cloned().ok_or("nth: index out of range".into()),
                    _ => Err("nth: first arg must be a list".into()),
                }
            }
            "str-concat" => {
                let parts: Vec<String> = args.iter().map(|a| match a {
                    LispVal::Str(s) => s.clone(),
                    _ => a.to_string(),
                }).collect();
                Ok(LispVal::Str(parts.join("")))
            }
            "str-contains" => Ok(LispVal::Bool(as_str(&args[0])?.contains(&as_str(&args[1])?))),
            "to-string" => Ok(LispVal::Str(args[0].to_string())),
            "read" => {
                let s = as_str(&args[0])?;
                match crate::parser::parse_all(&s) {
                    Ok(exprs) => exprs.into_iter().next().ok_or_else(|| "read: empty input".to_string()),
                    Err(e) => Err(format!("read: parse error: {}", e)),
                }
            }
            "read-all" => {
                let s = as_str(&args[0])?;
                match crate::parser::parse_all(&s) {
                    Ok(exprs) => Ok(LispVal::List(exprs)),
                    Err(e) => Err(format!("read-all: parse error: {}", e)),
                }
            }
            "str-length" => {
                let s = as_str(&args[0])?;
                Ok(LispVal::Num(s.chars().count() as i64))
            }
            "str-substring" => {
                let s = as_str(&args[0])?;
                let start = as_num(args.get(1).ok_or("str-substring: need start")?)? as usize;
                let end = as_num(args.get(2).ok_or("str-substring: need end")?)? as usize;
                let chars: Vec<char> = s.chars().collect();
                if start > end || end > chars.len() {
                    return Err(format!("str-substring: indices out of range ({}..{} for len {})", start, end, chars.len()));
                }
                Ok(LispVal::Str(chars[start..end].iter().collect()))
            }
            "str-split" => {
                let s = as_str(&args[0])?;
                let delim = as_str(args.get(1).ok_or("str-split: need delimiter")?)?;
                // If delim is a single char, use exact split; if multi-char, split on any char in delim (char set mode)
                let parts: Vec<LispVal> = if delim.len() == 1 {
                    s.split(delim.chars().next().unwrap()).filter(|p| !p.is_empty()).map(|p| LispVal::Str(p.to_string())).collect()
                } else {
                    // Treat multi-char delimiter as a character set — split on any char in it
                    let char_set: Vec<char> = delim.chars().collect();
                    let mut parts = Vec::new();
                    let mut current = String::new();
                    for ch in s.chars() {
                        if char_set.contains(&ch) {
                            if !current.is_empty() {
                                parts.push(LispVal::Str(std::mem::take(&mut current)));
                            }
                        } else {
                            current.push(ch);
                        }
                    }
                    if !current.is_empty() {
                        parts.push(LispVal::Str(current));
                    }
                    parts
                };
                Ok(LispVal::List(parts))
            }
            "str-split-exact" => {
                let s = as_str(&args[0])?;
                let delim = as_str(args.get(1).ok_or("str-split-exact: need delimiter")?)?;
                let parts: Vec<LispVal> = s.split(&delim).map(|p| LispVal::Str(p.to_string())).collect();
                Ok(LispVal::List(parts))
            }
            "str-trim" => {
                let s = as_str(&args[0])?;
                Ok(LispVal::Str(s.trim().to_string()))
            }
            "str-index-of" => {
                let haystack = as_str(&args[0])?;
                let needle = as_str(args.get(1).ok_or("str-index-of: need needle")?)?;
                let idx = haystack.find(&needle).map(|i| i as i64).unwrap_or(-1);
                Ok(LispVal::Num(idx))
            }
            "str-upcase" => Ok(LispVal::Str(as_str(&args[0])?.to_uppercase())),
            "str-downcase" => Ok(LispVal::Str(as_str(&args[0])?.to_lowercase())),
            "str-starts-with" => {
                let s = as_str(&args[0])?;
                let prefix = as_str(args.get(1).ok_or("str-starts-with: need prefix")?)?;
                Ok(LispVal::Bool(s.starts_with(&prefix)))
            }
            "str-ends-with" => {
                let s = as_str(&args[0])?;
                let suffix = as_str(args.get(1).ok_or("str-ends-with: need suffix")?)?;
                Ok(LispVal::Bool(s.ends_with(&suffix)))
            }
            "str=" => {
                let a = as_str(args.get(0).ok_or("str=: need 2 args")?)?;
                let b = as_str(args.get(1).ok_or("str=: need 2 args")?)?;
                Ok(LispVal::Bool(a == b))
            }
            "str!=" => {
                let a = as_str(args.get(0).ok_or("str!=: need 2 args")?)?;
                let b = as_str(args.get(1).ok_or("str!=: need 2 args")?)?;
                Ok(LispVal::Bool(a != b))
            }
            "nil?" => Ok(LispVal::Bool(
                matches!(&args[0], LispVal::Nil) || matches!(&args[0], LispVal::List(ref v) if v.is_empty()),
            )),
            "list?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::List(_)))),
            "number?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::Num(_) | LispVal::Float(_)))),
            "to-float" => match &args[0] {
                LispVal::Float(f) => Ok(LispVal::Float(*f)),
                LispVal::Num(n) => Ok(LispVal::Float(*n as f64)),
                LispVal::Str(s) => s.parse::<f64>().map(LispVal::Float).map_err(|_| format!("to-float: cannot parse '{}'", s)),
                other => Err(format!("to-float: expected number, got {}", other)),
            },
            "to-int" => match &args[0] {
                LispVal::Num(n) => Ok(LispVal::Num(*n)),
                LispVal::Float(f) => Ok(LispVal::Num(*f as i64)),
                LispVal::Str(s) => s.parse::<i64>().map(LispVal::Num).map_err(|_| format!("to-int: cannot parse '{}'", s)),
                other => Err(format!("to-int: expected number, got {}", other)),
            },
            "to-num" => match &args[0] {
                LispVal::Num(n) => Ok(LispVal::Num(*n)),
                LispVal::Float(f) => Ok(LispVal::Num(*f as i64)),
                LispVal::Str(s) => s.parse::<i64>().map(LispVal::Num).map_err(|_| format!("to-num: cannot parse '{}'", s)),
                other => Err(format!("to-num: expected number, got {}", other)),
            },
            "type?" => Ok(LispVal::Str(match &args[0] {
                LispVal::Nil => "nil",
                LispVal::Bool(_) => "boolean",
                LispVal::Num(_) => "number",
                LispVal::Float(_) => "number",
                LispVal::Str(_) => "string",
                LispVal::List(_) => "list",
                LispVal::Map(_) => "map",
                    LispVal::Lambda { .. } => "lambda",
                    LispVal::Macro { .. } => "macro",
                    LispVal::Sym(_) => "symbol",
                    _ => "unknown",
            }.to_string())),
            "bool?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::Bool(_)))),
            "string?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::Str(_)))),
            "map?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::Map(_)))),
            "macro?" => Ok(LispVal::Bool(matches!(&args[0], LispVal::Macro { .. }))),
            "error" => {
                let msg = args.get(0).map(|v| format!("{}", v)).unwrap_or_else(|| "error".to_string());
                Err(msg)
            }
            "debug" | "near/log-debug" => {
                let msg = args.get(0).map(|v| format!("{}", v)).unwrap_or_else(|| "debug".to_string());
                eprintln!("[DEBUG] {}", msg);
                Ok(LispVal::Nil)
            }
            "trace" => {
                let val = args.get(0).cloned().unwrap_or(LispVal::Nil);
                eprintln!("[TRACE] {}", val);
                Ok(val)
            }
            "inspect" => {
                let val = args.get(0).cloned().unwrap_or(LispVal::Nil);
                let type_str = match &val {
                    LispVal::Nil => "nil",
                    LispVal::Bool(_) => "boolean",
                    LispVal::Num(_) => "integer",
                    LispVal::Float(_) => "float",
                    LispVal::Str(_) => "string",
                    LispVal::List(items) => return Ok(LispVal::Str(format!("list[{}]: {}", items.len(), val))),
                    LispVal::Map(m) => return Ok(LispVal::Str(format!("map{{{} keys}}: {}", m.len(), val))),
                    LispVal::Lambda { params, .. } => return Ok(LispVal::Str(format!("lambda({}): <function>", params.len()))),
                    LispVal::Sym(s) => return Ok(LispVal::Str(format!("symbol: {}", s))),
                    _ => "unknown",
                };
                Ok(LispVal::Str(format!("{}: {}", type_str, val)))
            }

            // --- Dict / Map builtins ---
            "dict" => {
                let mut m = BTreeMap::new();
                let mut i = 0;
                while i + 1 < args.len() {
                    let key = as_str(&args[i]).map_err(|_| "dict: keys must be strings")?;
                    m.insert(key, args[i + 1].clone());
                    i += 2;
                }
                Ok(LispVal::Map(m))
            }
            "dict/get" => {
                let m = match &args[0] { LispVal::Map(m) => m, _ => return Err("dict/get: expected map".into()) };
                let key = as_str(&args[1]).map_err(|_| "dict/get: key must be string")?;
                Ok(m.get(&key).cloned().unwrap_or(LispVal::Nil))
            }
            "dict/set" => {
                let mut m = match &args[0] { LispVal::Map(m) => m.clone(), _ => return Err("dict/set: expected map".into()) };
                let key = as_str(&args[1]).map_err(|_| "dict/set: key must be string")?;
                m.insert(key, args.get(2).cloned().unwrap_or(LispVal::Nil));
                Ok(LispVal::Map(m))
            }
            "dict/has?" => {
                let m = match &args[0] { LispVal::Map(m) => m, _ => return Err("dict/has?: expected map".into()) };
                let key = as_str(&args[1]).map_err(|_| "dict/has?: key must be string")?;
                Ok(LispVal::Bool(m.contains_key(&key)))
            }
            "dict/keys" => {
                let m = match &args[0] { LispVal::Map(m) => m, _ => return Err("dict/keys: expected map".into()) };
                Ok(LispVal::List(m.keys().map(|k| LispVal::Str(k.clone())).collect()))
            }
            "dict/vals" => {
                let m = match &args[0] { LispVal::Map(m) => m, _ => return Err("dict/vals: expected map".into()) };
                Ok(LispVal::List(m.values().cloned().collect()))
            }
            "dict/remove" => {
                let mut m = match &args[0] { LispVal::Map(m) => m.clone(), _ => return Err("dict/remove: expected map".into()) };
                let key = as_str(&args[1]).map_err(|_| "dict/remove: key must be string")?;
                m.remove(&key);
                Ok(LispVal::Map(m))
            }
            "dict/merge" => {
                let mut m = match &args[0] { LispVal::Map(m) => m.clone(), _ => return Err("dict/merge: first arg must be map".into()) };
                match &args[1] {
                    LispVal::Map(m2) => { for (k, v) in m2 { m.insert(k.clone(), v.clone()); } }
                    _ => return Err("dict/merge: second arg must be map".into()),
                }
                Ok(LispVal::Map(m))
            }

            // --- JSON ---
            "json-parse" | "from-json" => {
                let s = as_str(&args[0])?;
                match serde_json::from_str::<serde_json::Value>(&s) {
                    Ok(v) => Ok(json_to_lisp(v)),
                    Err(e) => Err(format!("json-parse: {}", e)),
                }
            }
            "json-get" => {
                let s = as_str(&args[0])?;
                let key = as_str(&args[1])?;
                let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| format!("json-get: parse error: {}", e))?;
                match v.get(&key) {
                    Some(val) => Ok(json_to_lisp(val.clone())),
                    None => Ok(LispVal::Nil),
                }
            }
            "json-get-in" => {
                let s = as_str(&args[0])?;
                let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| format!("json-get-in: parse error: {}", e))?;
                let mut cur = &v;
                for arg in &args[1..] {
                    let key = as_str(arg)?;
                    cur = cur.get(&key).unwrap_or(&serde_json::Value::Null);
                }
                Ok(json_to_lisp(cur.clone()))
            }
            "json-build" => {
                let j = lisp_to_json(&args[0]);
                Ok(LispVal::Str(j.to_string()))
            }
            "to-json" => {
                let json_val = lisp_to_json(&args[0]);
                serde_json::to_string(&json_val).map(LispVal::Str).map_err(|e| format!("to-json: {}", e))
            }

            // --- Crypto (standalone using sha2/keccak crates or stubs) ---
            "sha256" => {
                builtin_sha256(&args)
            }
            "keccak256" => {
                builtin_keccak256(&args)
            }

            // --- List stdlib native builtins ---
            "empty?" => Ok(LispVal::Bool(
                matches!(&args[0], LispVal::Nil) || matches!(&args[0], LispVal::List(ref v) if v.is_empty()),
            )),
            "range" => {
                let start = as_num(args.get(0).ok_or("range: need 2 args")?)?;
                let end = as_num(args.get(1).ok_or("range: need 2 args")?)?;
                if start >= end { return Ok(LispVal::List(vec![])); }
                Ok(LispVal::List((start..end).map(LispVal::Num).collect()))
            }
            "reverse" => match &args[0] {
                LispVal::List(l) => Ok(LispVal::List(l.iter().rev().cloned().collect())),
                LispVal::Nil => Ok(LispVal::List(vec![])),
                other => Err(format!("reverse: expected list, got {}", other)),
            },
            "sort" => {
                let mut vals = match &args[0] {
                    LispVal::List(l) => l.clone(),
                    LispVal::Nil => vec![],
                    other => return Err(format!("sort: expected list, got {}", other)),
                };
                vals.sort_by(|a, b| {
                    let fa = match a { LispVal::Num(n) => *n as f64, LispVal::Float(f) => *f, _ => 0.0 };
                    let fb = match b { LispVal::Num(n) => *n as f64, LispVal::Float(f) => *f, _ => 0.0 };
                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok(LispVal::List(vals))
            }
            "zip" => {
                let a = match &args[0] { LispVal::List(l) => l.clone(), LispVal::Nil => vec![], other => return Err(format!("zip: expected list, got {}", other)) };
                let b = match args.get(1) { Some(LispVal::List(l)) => l.clone(), Some(LispVal::Nil) => vec![], Some(other) => return Err(format!("zip: expected list, got {}", other)), None => return Err("zip: need 2 args".into()) };
                Ok(LispVal::List(a.iter().zip(b.iter()).map(|(x, y)| LispVal::List(vec![x.clone(), y.clone()])).collect()))
            }
            "map" => {
                let func = args.get(0).ok_or("map: need (f list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::List(vec![])),
                    Some(other) => return Err(format!("map: expected list, got {}", other)),
                    None => return Err("map: need (f list)".into()),
                };
                // Fast path: compile single-param lambda to bytecode
                if let LispVal::Lambda { params, rest_param: None, body, closed_env } = func {
                    if params.len() == 1 {
                        if let Some(cl) = crate::bytecode::try_compile_lambda(
                            params, body, closed_env, env,
                        ) {
                            if lst.is_empty() {
                                return Ok(LispVal::List(vec![]));
                            }
                            // Try first element — if bytecode can't handle it (macro,
                            // user fn, etc), fall back gracefully
                            if let Ok(first_result) = crate::bytecode::run_compiled_lambda(&cl, &[lst[0].clone()]) {
                                let mut result = Vec::with_capacity(lst.len());
                                result.push(first_result);
                                for elem in &lst[1..] {
                                    result.push(crate::bytecode::run_compiled_lambda(&cl, &[elem.clone()])?);
                                }
                                return Ok(LispVal::List(result));
                            }
                            // First element failed — fall through to eval path
                        }
                    }
                }
                // Fallback: full eval per element
                let mut result = Vec::with_capacity(lst.len());
                for elem in &lst {
                    result.push(call_val(func, &[elem.clone()], env)?);
                }
                Ok(LispVal::List(result))
            }
            "filter" => {
                let func = args.get(0).ok_or("filter: need (pred list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::List(vec![])),
                    Some(other) => return Err(format!("filter: expected list, got {}", other)),
                    None => return Err("filter: need (pred list)".into()),
                };
                // Fast path: compile single-param lambda to bytecode
                if let LispVal::Lambda { params, rest_param: None, body, closed_env } = func {
                    if params.len() == 1 {
                        if let Some(cl) =
                            crate::bytecode::try_compile_lambda(params, body, closed_env, env)
                        {
                            if lst.is_empty() {
                                return Ok(LispVal::List(vec![]));
                            }
                            // Try first element — if bytecode can't handle it, fall back
                            if let Ok(first_result) =
                                crate::bytecode::run_compiled_lambda(&cl, &[lst[0].clone()])
                            {
                                let mut result = Vec::new();
                                if is_truthy(&first_result) {
                                    result.push(lst[0].clone());
                                }
                                for elem in &lst[1..] {
                                    if is_truthy(&crate::bytecode::run_compiled_lambda(&cl, &[elem.clone()])?) {
                                        result.push(elem.clone());
                                    }
                                }
                                return Ok(LispVal::List(result));
                            }
                            // First element failed — fall through to eval path
                        }
                    }
                }
                // Fallback: full eval per element
                let mut result = Vec::new();
                for elem in &lst {
                    if is_truthy(&call_val(func, &[elem.clone()], env)?) {
                        result.push(elem.clone());
                    }
                }
                Ok(LispVal::List(result))
            }
            "reduce" => {
                let func = args.get(0).ok_or("reduce: need (f init list)")?;
                let mut acc = args.get(1).ok_or("reduce: need (f init list)")?.clone();
                let lst = match args.get(2) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(acc),
                    Some(other) => return Err(format!("reduce: expected list, got {}", other)),
                    None => return Err("reduce: need (f init list)".into()),
                };
                let acc_str = acc.to_string();
                for elem in &lst {
                    let prev_acc = acc.clone();
                    acc = call_val(func, &[prev_acc.clone(), elem.clone()], env)?;
                    // Warn if accumulator doesn't incorporate the current element
                    let new_acc_str = acc.to_string();
                    if new_acc_str == prev_acc.to_string() && !lst.is_empty() {
                        eprintln!("[WARN] reduce: accumulator unchanged after processing element. \
                            Your function may be ignoring the current element. \
                            Consider using (str-join sep lst) for string joining.");
                    }
                }
                Ok(acc)
            }
            "str-join" => {
                let sep = as_str(args.get(0).ok_or("str-join: need (separator list)")?)?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l,
                    Some(LispVal::Nil) => return Ok(LispVal::Str(String::new())),
                    Some(other) => return Err(format!("str-join: expected list, got {}", other)),
                    None => return Err("str-join: need (separator list)".into()),
                };
                let parts: Vec<String> = lst.iter().map(|v| match v {
                    LispVal::Str(s) => s.clone(),
                    _ => v.to_string(),
                }).collect();
                Ok(LispVal::Str(parts.join(&sep)))
            }
            "find" => {
                let func = args.get(0).ok_or("find: need (pred list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::Nil),
                    Some(other) => return Err(format!("find: expected list, got {}", other)),
                    None => return Err("find: need (pred list)".into()),
                };
                for elem in &lst {
                    if is_truthy(&call_val(func, &[elem.clone()], env)?) {
                        return Ok(elem.clone());
                    }
                }
                Ok(LispVal::Nil)
            }
            "some" => {
                let func = args.get(0).ok_or("some: need (pred list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::Bool(false)),
                    Some(other) => return Err(format!("some: expected list, got {}", other)),
                    None => return Err("some: need (pred list)".into()),
                };
                for elem in &lst {
                    if is_truthy(&call_val(func, &[elem.clone()], env)?) {
                        return Ok(LispVal::Bool(true));
                    }
                }
                Ok(LispVal::Bool(false))
            }
            "every" => {
                let func = args.get(0).ok_or("every: need (pred list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::Bool(true)),
                    Some(other) => return Err(format!("every: expected list, got {}", other)),
                    None => return Err("every: need (pred list)".into()),
                };
                for elem in &lst {
                    if !is_truthy(&call_val(func, &[elem.clone()], env)?) {
                        return Ok(LispVal::Bool(false));
                    }
                }
                Ok(LispVal::Bool(true))
            }

            "fmt" => {
                let template = match &args[0] {
                    LispVal::Str(s) => s.clone(),
                    _ => return Err("fmt: need template string".into()),
                };
                let data = &args[1];
                let mut result = String::new();
                let chars: Vec<char> = template.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] == '{' {
                        let mut key = String::new();
                        i += 1;
                        while i < chars.len() && chars[i] != '}' {
                            key.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() { i += 1; }
                        let mut found = false;
                        if let LispVal::Map(map) = data {
                            if let Some(val) = map.get(&key) {
                                match val {
                                    LispVal::Str(s) => result.push_str(s),
                                    _ => result.push_str(&val.to_string()),
                                }
                                found = true;
                            }
                        }
                        if !found {
                            result.push('{');
                            result.push_str(&key);
                            result.push('}');
                        }
                    } else {
                        result.push(chars[i]);
                        i += 1;
                    }
                }
                Ok(LispVal::Str(result))
            }

            // --- File I/O ---
            "file/read" => {
                let path = as_str(&args[0])?;
                match std::fs::read_to_string(&path) {
                    Ok(s) => Ok(LispVal::Str(s)),
                    Err(e) => Err(format!("file/read: {}", e)),
                }
            }
            "file/write" => {
                let path = as_str(&args[0])?;
                let content = as_str(&args[1])?;
                match std::fs::write(&path, &content) {
                    Ok(()) => Ok(LispVal::Bool(true)),
                    Err(e) => Err(format!("file/write: {}", e)),
                }
            }
            "file/exists?" => {
                let path = as_str(&args[0])?;
                Ok(LispVal::Bool(std::path::Path::new(&path).exists()))
            }
            "file/list" => {
                let path = as_str(&args[0])?;
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let names: Vec<LispVal> = entries
                            .filter_map(|e| e.ok())
                            .map(|e| LispVal::Str(e.file_name().to_string_lossy().to_string()))
                            .collect();
                        Ok(LispVal::List(names))
                    }
                    Err(e) => Err(format!("file/list: {}", e)),
                }
            }

            // --- File I/O (convenience aliases) ---
            "write-file" => {
                let path = as_str(&args[0])?;
                let content = as_str(&args[1])?;
                // Unescape common sequences
                let content = content.replace("\\n", "\n").replace("\\t", "\t").replace("\\\"", "\"");
                match std::fs::write(&path, &content) {
                    Ok(()) => Ok(LispVal::Bool(true)),
                    Err(e) => Err(format!("write-file: {}", e)),
                }
            }
            "read-file" => {
                let path = as_str(&args[0])?;
                match std::fs::read_to_string(&path) {
                    Ok(s) => Ok(LispVal::Str(s)),
                    Err(e) => Err(format!("read-file: {}", e)),
                }
            }
            "load-file" => {
                let path = as_str(&args[0])?;
                let code = std::fs::read_to_string(&path)
                    .map_err(|e| format!("load-file: {}", e))?;
                let exprs = parse_all(&code)?;
                let mut result = LispVal::Nil;
                for expr in &exprs {
                    result = lisp_eval(expr, env)?;
                }
                Ok(result)
            }
            "append-file" => {
                let path = as_str(&args[0])?;
                let content = as_str(&args[1])?;
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|e| format!("append-file: {}", e))?;
                f.write_all(content.as_bytes())
                    .map_err(|e| format!("append-file: {}", e))?;
                Ok(LispVal::Bool(true))
            }
            "file-exists?" => {
                let path = as_str(&args[0])?;
                Ok(LispVal::Bool(std::path::Path::new(&path).exists()))
            }
            "shell" => {
                let cmd = as_str(&args[0])?;
                let allow = std::env::var("RLM_ALLOW_SHELL").unwrap_or_default();
                if allow != "1" && allow != "true" {
                    return Err("shell: blocked unless RLM_ALLOW_SHELL=1 is set".into());
                }
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .map_err(|e| format!("shell: {}", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("shell: exit {:?}: {}{}", output.status.code(), stdout, stderr));
                }
                Ok(LispVal::Str(stdout))
            }

            // --- HTTP builtins ---
            "http-get" => {
                let url = as_str(&args[0])?;
                let rt = &SHARED_RUNTIME;
                let body = rt.block_on(async {
                    reqwest::get(&url).await
                        .map_err(|e| format!("http-get: {}", e))?
                        .text().await
                        .map_err(|e| format!("http-get: {}", e))
                })?;
                Ok(LispVal::Str(body))
            }
            "http-post" => {
                let url = as_str(&args[0])?;
                let body_str = as_str(args.get(1).ok_or("http-post: need body")?)?;
                let rt = &SHARED_RUNTIME;
                let body = rt.block_on(async {
                    let client = &SHARED_CLIENT;
                    client.post(&url)
                        .header("Content-Type", "application/json")
                        .body(body_str)
                        .send().await
                        .map_err(|e| format!("http-post: {}", e))?
                        .text().await
                        .map_err(|e| format!("http-post: {}", e))
                })?;
                Ok(LispVal::Str(body))
            }
            "http-get-json" => {
                let url = as_str(&args[0])?;
                let rt = &SHARED_RUNTIME;
                let body = rt.block_on(async {
                    reqwest::get(&url).await
                        .map_err(|e| format!("http-get-json: {}", e))?
                        .text().await
                        .map_err(|e| format!("http-get-json: {}", e))
                })?;
                let v: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| format!("http-get-json: parse error: {}", e))?;
                Ok(json_to_lisp(v))
            }

            // --- LLM builtins ---

            // --- LLM builtins ---
            "llm" => {
                let prompt = as_str(&args[0])?;
                let api_key = std::env::var("RLM_API_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .or_else(|_| std::env::var("GLM_API_KEY"))
                    .map_err(|_| "llm: set RLM_API_KEY, OPENAI_API_KEY, or GLM_API_KEY")?;
                let api_base = std::env::var("RLM_API_BASE")
                    .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());
                let model = std::env::var("RLM_MODEL")
                    .unwrap_or_else(|_| "glm-5.1".to_string());

                let rt = &SHARED_RUNTIME;
                let (resp, tokens) = rt.block_on(async {
                    let client = &SHARED_CLIENT;
                    let body = serde_json::json!({
                        "model": model,
                        "messages": [
                            {"role": "system", "content": "You are a helpful assistant with access to a Lisp runtime called lisp-rlm."},
                            {"role": "user", "content": prompt}
                        ],
                        "max_tokens": 2048
                    });
                    let resp = client.post(format!("{}/chat/completions", api_base))
                        .header("Authorization", format!("Bearer {}", api_key))
                        .json(&body)
                        .send().await
                        .map_err(|e| format!("llm: request failed: {}", e))?;
                    let text = resp.text().await
                        .map_err(|e| format!("llm: read body failed: {}", e))?;
                    let v: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| format!("llm: json parse error: {}", e))?;
                    let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                    let content = v["choices"][0]["message"]["content"].as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("llm: unexpected response: {}", text))?;
                    Ok::<_, String>((content, tokens))
                })?;
                env.tokens_used += tokens;
                env.llm_calls += 1;
                Ok(LispVal::Str(resp))
            }
            "llm-code" => {
                let prompt = as_str(&args[0])?;
                let api_key = std::env::var("RLM_API_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .or_else(|_| std::env::var("GLM_API_KEY"))
                    .map_err(|_| "llm-code: set RLM_API_KEY, OPENAI_API_KEY, or GLM_API_KEY")?;
                let api_base = std::env::var("RLM_API_BASE")
                    .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());
                let model = std::env::var("RLM_MODEL")
                    .unwrap_or_else(|_| "glm-5.1".to_string());

                let builtin_ref = RLM_SYSTEM_PROMPT;

                let rt = &SHARED_RUNTIME;
                let (raw_code, tokens) = rt.block_on(async {
                    let client = &SHARED_CLIENT;
                    let body = serde_json::json!({
                        "model": model,
                        "messages": [
                            {"role": "system", "content": builtin_ref},
                            {"role": "user", "content": prompt}
                        ],
                        "max_tokens": 2048
                    });
                    let resp = client.post(format!("{}/chat/completions", api_base))
                        .header("Authorization", format!("Bearer {}", api_key))
                        .json(&body)
                        .send().await
                        .map_err(|e| format!("llm-code: request failed: {}", e))?;
                    let text = resp.text().await
                        .map_err(|e| format!("llm-code: read body failed: {}", e))?;
                    let v: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| format!("llm-code: json parse error: {}", e))?;
                    let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                    let content = v["choices"][0]["message"]["content"].as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("llm-code: unexpected response: {}", text))?;
                    Ok::<_, String>((content, tokens))
                })?;

                env.tokens_used += tokens;
                env.llm_calls += 1;

                let code_str = strip_markdown_fences(&raw_code);

                // Parse and eval the LLM-generated Lisp code
                let exprs = parse_all(&code_str)?;
                let mut result = LispVal::Nil;
                for expr in &exprs {
                    result = lisp_eval(expr, env)?;
                }
                Ok(result)
            }

            // --- RLM agent loop ---
            "rlm" => {
                let task = as_str(&args[0])?;

                let api_key = std::env::var("RLM_API_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .or_else(|_| std::env::var("GLM_API_KEY"))
                    .map_err(|_| "rlm: set RLM_API_KEY, OPENAI_API_KEY, or GLM_API_KEY")?;
                let api_base = std::env::var("RLM_API_BASE")
                    .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());
                let model = std::env::var("RLM_MODEL")
                    .unwrap_or_else(|_| "glm-5.1".to_string());

                // Use the rich system prompt aligned with MIT paper
                let sys_prompt = std::env::var("RLM_SYSTEM_PROMPT").unwrap_or_else(|_| RLM_SYSTEM_PROMPT.to_string());
                let max_iterations: usize = std::env::var("RLM_MAX_ITERATIONS")
                    .ok().and_then(|s| s.parse().ok()).unwrap_or(10);
                let do_verify = std::env::var("RLM_VERIFY").unwrap_or_default() == "1";

                let mut messages = vec![
                    serde_json::json!({"role": "system", "content": sys_prompt}),
                    serde_json::json!({"role": "user", "content": format!("Your task: {}\n\nUse the REPL to explore context, plan, and solve. Call (show-context) and (show-vars) to understand your environment.", task)}),
                ];

                // --- Planning step: determine ACTION vs CODE mode ---
                let mut action_mode = false;
                {
                    let plan_prompt = format!(
                        "Given this task, choose your approach: ACTION or CODE.\n\n\
                         ACTION means you'll execute single Lisp expressions directly (like reading files, calling llm, processing strings). Best for: summarization, analysis, one-shot tasks.\n\
                         CODE means you'll write a multi-step Lisp program with defines, loops, and logic. Best for: computation, data processing, multi-step algorithms.\n\n\
                         Reply with just ACTION or CODE and a brief one-line plan.\n\nTask: {}",
                        task
                    );
                    let rt = &SHARED_RUNTIME;
                    let (plan_resp, plan_tokens) = rt.block_on(async {
                        let client = &SHARED_CLIENT;
                        let body = serde_json::json!({
                            "model": model,
                            "messages": [
                                {"role": "system", "content": "Reply with just ACTION or CODE followed by a brief plan. One line only."},
                                {"role": "user", "content": plan_prompt}
                            ],
                            "max_tokens": 256
                        });
                        let resp = client.post(format!("{}/chat/completions", api_base))
                            .header("Authorization", format!("Bearer {}", api_key))
                            .json(&body)
                            .send().await.map_err(|e| format!("rlm plan: {}", e))?;
                        let text = resp.text().await.map_err(|e| format!("rlm plan: {}", e))?;
                        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("rlm plan: {}", e))?;
                        let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                        let content = v["choices"][0]["message"]["content"].as_str()
                            .map(|s| s.to_string()).ok_or_else(|| "rlm plan: no content".to_string())?;
                        Ok::<_, String>((content, tokens))
                    })?;
                    env.tokens_used += plan_tokens;
                    env.llm_calls += 1;
                    let plan_upper = plan_resp.to_uppercase();
                    action_mode = plan_upper.contains("ACTION") && !plan_upper.contains("CODE");
                    eprintln!("[rlm] mode: {} — {}", if action_mode { "ACTION" } else { "CODE" }, plan_resp.trim());
                }

                let mut final_result = LispVal::Nil;
                let max_retries: usize = 3;
                let saved_iteration = env.rlm_iteration;

                for iter in 0..max_iterations {
                    env.rlm_iteration = saved_iteration + iter;

                    // Iteration-specific prompt (paper's key insight)
                    let iter_prompt = if action_mode {
                        if iter == 0 {
                            "You are in ACTION mode. Return a SINGLE Lisp expression to evaluate. No definitions, no multi-step programs — just one expression like (read-file ...), (llm ...), (str-chunk ...), etc. Explore the context first. End with (final \"answer\") when done.".to_string()
                        } else {
                            let last_result_preview = match env.rlm_state.get("last_result") {
                                Some(v) => truncate_str(&v.to_string(), 200),
                                None => "none yet".to_string(),
                            };
                            format!("ACTION mode, iteration {}. Last result: {}. Return your NEXT single Lisp expression. Use (final \"answer\") or (final-var name) when done.", iter, last_result_preview)
                        }
                    } else if iter == 0 {
                        "You have not interacted with the context yet. Your next action should be to look through it and plan your approach. Don't provide a final answer yet.".to_string()
                    } else {
                        "The history above is your previous interactions. Continue using the REPL to solve the task.".to_string()
                    };
                    messages.push(serde_json::json!({"role": "user", "content": iter_prompt}));

                    // Snapshot env
                    let snap = env.take_snapshot();

                    // Call LLM
                    let rt = &SHARED_RUNTIME;
                    let (resp_text, tokens) = rt.block_on(async {
                        let client = &SHARED_CLIENT;
                        let body = serde_json::json!({
                            "model": model,
                            "messages": messages,
                            "max_tokens": 8192
                        });
                        let resp = client.post(format!("{}/chat/completions", api_base))
                            .header("Authorization", format!("Bearer {}", api_key))
                            .json(&body)
                            .send().await
                            .map_err(|e| format!("rlm: request failed: {}", e))?;
                        let text = resp.text().await
                            .map_err(|e| format!("rlm: read body failed: {}", e))?;
                        let v: serde_json::Value = serde_json::from_str(&text)
                            .map_err(|e| format!("rlm: json parse error: {}", e))?;
                        let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                        let content = v["choices"][0]["message"]["content"].as_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| format!("rlm: unexpected response: {}", text))?;
                        Ok::<_, String>((content, tokens))
                    })?;

                    env.tokens_used += tokens;
                    env.llm_calls += 1;

                    // Strip markdown fences
                    let code_str = strip_markdown_fences(&resp_text);

                    // Append assistant message (truncated for context window management)
                    messages.push(serde_json::json!({"role": "assistant", "content": truncate_str(&resp_text, 500)}));

                    eprintln!("[rlm iter {}] code:\n{}", iter, code_str);

                    // Try to parse with better error recovery
                    let exprs = match parse_all(&code_str) {
                        Ok(e) => e,
                        Err(parse_err) => {
                            // Try extracting first valid expression
                            if let Some(first_expr) = extract_first_valid_expr(&code_str) {
                                vec![first_expr]
                            } else {
                                messages.push(serde_json::json!({"role": "user", "content": format!("Parse error at: {}. Please fix and return valid Lisp code.", parse_err)}));
                                continue;
                            }
                        }
                    };

                    // Try to eval with retry logic
                    let mut result = LispVal::Nil;
                    let mut eval_ok = false;

                    for retry in 0..max_retries {
                        let snap_inner = env.take_snapshot();
                        let mut failed = false;
                        let mut err_msg = String::new();

                        // Try executing all expressions at once
                        for expr in &exprs {
                            match lisp_eval(expr, env) {
                                Ok(v) => result = v,
                                Err(e) => {
                                    err_msg = e;
                                    env.restore_snapshot(snap_inner.clone());
                                    failed = true;
                                    break;
                                }
                            }
                        }

                        if !failed {
                            eval_ok = true;
                            break;
                        }

                        // If multi-expr failed, try one by one
                        if exprs.len() > 1 {
                            env.restore_snapshot(snap_inner.clone());
                            let mut individual_ok = true;
                            let mut individual_result = LispVal::Nil;
                            for (j, sub_expr) in exprs.iter().enumerate() {
                                match lisp_eval(sub_expr, env) {
                                    Ok(v) => individual_result = v,
                                    Err(se) => {
                                        env.restore_snapshot(snap_inner.clone());
                                        err_msg = format!("Expr {}/{} failed: {}", j + 1, exprs.len(), se);
                                        individual_ok = false;
                                        break;
                                    }
                                }
                            }
                            if individual_ok {
                                result = individual_result;
                                eval_ok = true;
                                break;
                            }
                        }

                        // Ask LLM to retry
                        if retry + 1 < max_retries {
                            let err_truncated = truncate_str(&err_msg, 200);
                            messages.push(serde_json::json!({"role": "user", "content": format!("Error (retry {}/{}): {}. Try again.", retry + 1, max_retries, err_truncated)}));
                            // Get new code from LLM
                            let rt2 = &SHARED_RUNTIME;
                            let (retry_text, retry_tokens) = rt2.block_on(async {
                                let client = &SHARED_CLIENT;
                                let body = serde_json::json!({
                                    "model": model,
                                    "messages": messages,
                                    "max_tokens": 8192
                                });
                                let resp = client.post(format!("{}/chat/completions", api_base))
                                    .header("Authorization", format!("Bearer {}", api_key))
                                    .json(&body)
                                    .send().await.map_err(|e| format!("rlm: {}", e))?;
                                let text = resp.text().await.map_err(|e| format!("rlm: {}", e))?;
                                let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("rlm: {}", e))?;
                                let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                                let content = v["choices"][0]["message"]["content"].as_str()
                                    .map(|s| s.to_string()).ok_or_else(|| "rlm: unexpected".to_string())?;
                                Ok::<_, String>((content, tokens))
                            })?;
                            env.tokens_used += retry_tokens;
                            env.llm_calls += 1;
                            messages.push(serde_json::json!({"role": "assistant", "content": truncate_str(&retry_text, 500)}));
                            let retry_code = strip_markdown_fences(&retry_text);
                            // Re-parse for next retry iteration
                            // We'll just loop and re-use the original exprs vec... 
                            // Actually let's break to outer loop since this gets complex
                            break;
                        }
                    }

                    if !eval_ok {
                        // Move to next iteration — LLM will see the error in messages
                        continue;
                    }

                    eprintln!("[rlm iter {}] result: {}", iter, truncate_str(&result.to_string(), 200));
                    final_result = result.clone();
                    // Store last_result for ACTION mode context
                    env.rlm_state.insert("last_result".to_string(), result);

                    // Check if code set Final=true via (final ...) or (final-var ...)
                    let is_final = env.rlm_state.get("Final")
                        .map(|v| is_truthy(v))
                        .unwrap_or(false);
                    if is_final {
                        if let Some(r) = env.rlm_state.get("result") {
                            final_result = r.clone();
                        }
                        break;
                    }

                    // Metadata-only stdout truncation (paper's key insight)
                    // Instead of appending full result, append only metadata
                    let result_str = final_result.to_string();
                    let result_type = match &final_result {
                        LispVal::Nil => "nil",
                        LispVal::Bool(_) => "bool",
                        LispVal::Num(_) => "int",
                        LispVal::Float(_) => "float",
                        LispVal::Str(_) => "string",
                        LispVal::List(l) => &format!("list[{}]", l.len()),
                        LispVal::Map(m) => &format!("map[{}]", m.len()),
                        _ => "other",
                    };
                    let output_meta = format!(
                        "[Output: type={}, len={}, preview=\"{}\"]",
                        result_type,
                        result_str.len(),
                        truncate_str(&result_str, 100)
                    );
                    messages.push(serde_json::json!({"role": "user", "content": output_meta}));

                    // Continue to next iteration if not final
                    continue;
                }

                // Self-verification step
                if do_verify {
                    let result_str = final_result.to_string();
                    let verify_prompt = format!(
                        "Given the task: {}\nAnd the result: {}\n\nIs this correct? Answer YES or NO and explain briefly.",
                        task, truncate_str(&result_str, 300)
                    );
                    let rt = &SHARED_RUNTIME;
                    let (verdict, vtokens) = rt.block_on(async {
                        let client = &SHARED_CLIENT;
                        let body = serde_json::json!({
                            "model": model,
                            "messages": [
                                {"role": "system", "content": "You are a verification assistant. Answer YES or NO."},
                                {"role": "user", "content": verify_prompt}
                            ],
                            "max_tokens": 512
                        });
                        let resp = client.post(format!("{}/chat/completions", api_base))
                            .header("Authorization", format!("Bearer {}", api_key))
                            .json(&body)
                            .send().await.map_err(|e| format!("rlm verify: {}", e))?;
                        let text = resp.text().await.map_err(|e| format!("rlm verify: {}", e))?;
                        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("rlm verify: {}", e))?;
                        let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                        let content = v["choices"][0]["message"]["content"].as_str()
                            .map(|s| s.to_string()).ok_or_else(|| "rlm verify: no content".to_string())?;
                        Ok::<_, String>((content, tokens))
                    })?;
                    env.tokens_used += vtokens;
                    env.llm_calls += 1;

                    if verdict.to_uppercase().starts_with("NO") {
                        eprintln!("[rlm verify] FAILED: {}", truncate_str(&verdict, 200));
                        // One more iteration with critique
                        messages.push(serde_json::json!({"role": "user", "content": format!("Verification failed: {}. Please fix.", verdict)}));
                        let snap = env.take_snapshot();
                        let rt = &SHARED_RUNTIME;
                        let (fix_text, fix_tokens) = rt.block_on(async {
                            let client = &SHARED_CLIENT;
                            let body = serde_json::json!({
                                "model": model,
                                "messages": messages,
                                "max_tokens": 8192
                            });
                            let resp = client.post(format!("{}/chat/completions", api_base))
                                .header("Authorization", format!("Bearer {}", api_key))
                                .json(&body)
                                .send().await.map_err(|e| format!("rlm: {}", e))?;
                            let text = resp.text().await.map_err(|e| format!("rlm: {}", e))?;
                            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("rlm: {}", e))?;
                            let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;
                            let content = v["choices"][0]["message"]["content"].as_str()
                                .map(|s| s.to_string()).ok_or_else(|| "rlm: unexpected".to_string())?;
                            Ok::<_, String>((content, tokens))
                        })?;
                        env.tokens_used += fix_tokens;
                        env.llm_calls += 1;
                        let fix_code = strip_markdown_fences(&fix_text);
                        if let Ok(fix_exprs) = parse_all(&fix_code) {
                            let mut fix_result = LispVal::Nil;
                            let mut fix_ok = true;
                            for expr in &fix_exprs {
                                match lisp_eval(expr, env) {
                                    Ok(v) => fix_result = v,
                                    Err(_) => {
                                        env.restore_snapshot(snap);
                                        fix_ok = false;
                                        break;
                                    }
                                }
                            }
                            if fix_ok {
                                final_result = fix_result;
                            }
                        }
                    }
                }

                Ok(final_result)
            }

            // --- Sub-RLM ---
            "sub-rlm" => {
                let sub_task = as_str(&args[0])?;
                if env.rlm_depth >= 5 {
                    return Err("sub-rlm: max depth (5) exceeded".into());
                }
                env.rlm_depth += 1;
                let result = lisp_eval(
                    &LispVal::List(vec![
                        LispVal::Sym("rlm".to_string()),
                        LispVal::Str(sub_task),
                    ]),
                    env,
                );
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
            "str-chunk" => {
                let s = as_str(&args[0])?;
                let n = as_num(args.get(1).ok_or("str-chunk: need n")?)? as usize;
                if n == 0 { return Err("str-chunk: n must be > 0".into()); }
                let chars: Vec<char> = s.chars().collect();
                let total = chars.len();
                let chunk_size = (total + n - 1) / n; // ceil division
                if chunk_size == 0 {
                    return Ok(LispVal::List(vec![LispVal::Str(String::new()); n.min(total + 1)]));
                }
                let mut chunks: Vec<LispVal> = Vec::new();
                let mut i = 0;
                while i < total {
                    let end = (i + chunk_size).min(total);
                    let chunk: String = chars[i..end].iter().collect();
                    chunks.push(LispVal::Str(chunk));
                    i += chunk_size;
                }
                Ok(LispVal::List(chunks))
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
                let context_val = env.rlm_state.get("context").cloned().unwrap_or(LispVal::Nil);
                let context_str = context_val.to_string();
                let preview = truncate_str(&context_str, 200);
                let final_set = env.rlm_state.get("Final")
                    .map(|v| is_truthy(v))
                    .unwrap_or(false);
                Ok(LispVal::Str(format!(
                    "Context length: {} chars\nPreview: {}\nIteration: {}\nFinal set: {}",
                    context_str.len(), preview, env.rlm_iteration, final_set
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

                let api_key = std::env::var("RLM_API_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .or_else(|_| std::env::var("GLM_API_KEY"))
                    .map_err(|_| "rlm-write: set API key env var")?;
                let api_base = std::env::var("RLM_API_BASE")
                    .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());
                let model = std::env::var("RLM_MODEL")
                    .unwrap_or_else(|_| "glm-5.1".to_string());

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

                let rt = &SHARED_RUNTIME;
                let (code, calls): (String, usize) = rt.block_on(async {
                    let client = &SHARED_CLIENT;
                    let mut calls: usize = 0;

                    // Initial generation
                    let body = serde_json::json!({
                        "model": model,
                        "messages": [
                            {"role": "system", "content": sys},
                            {"role": "user", "content": task}
                        ],
                        "max_tokens": 8192
                    });
                    let resp = client.post(format!("{}/chat/completions", api_base))
                        .header("Authorization", format!("Bearer {}", api_key))
                        .json(&body)
                        .send().await.map_err(|e| format!("rlm-write: {}", e))?;
                    let text = resp.text().await.map_err(|e| format!("rlm-write: {}", e))?;
                    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("rlm-write: {}", e))?;
                    let raw = v["choices"][0]["message"]["content"].as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("rlm-write: unexpected response"))?;
                    calls += 1;
                    let code = strip_markdown_fences(&raw);

                    // Verify parse, retry once if broken
                    let final_code = if crate::parser::parse_all(&code).is_err() {
                        let fix = format!("The previous code had a parse error. Write it again, fixed. Return ONLY valid raw Lisp code, no markdown, no explanations.");
                        let body2 = serde_json::json!({
                            "model": model,
                            "messages": [
                                {"role": "system", "content": sys},
                                {"role": "assistant", "content": &code},
                                {"role": "user", "content": fix}
                            ],
                            "max_tokens": 8192
                        });
                        let resp2 = client.post(format!("{}/chat/completions", api_base))
                            .header("Authorization", format!("Bearer {}", api_key))
                            .json(&body2)
                            .send().await.map_err(|e| format!("rlm-write retry: {}", e))?;
                        let text2 = resp2.text().await.map_err(|e| format!("rlm-write retry: {}", e))?;
                        let v2: serde_json::Value = serde_json::from_str(&text2).map_err(|e| format!("rlm-write retry: {}", e))?;
                        calls += 1;
                        if let Some(fixed) = v2["choices"][0]["message"]["content"].as_str() {
                            let fixed = strip_markdown_fences(fixed);
                            if crate::parser::parse_all(&fixed).is_ok() { fixed } else { code }
                        } else { code }
                    } else { code };

                    // Strip trailing write-file calls (rlm-write saves automatically)
                    let final_code = final_code.trim_end().to_string();
                    let final_code = if final_code.ends_with(")") {
                        // Remove last top-level (write-file ...) if present
                        let trimmed = final_code.trim_end();
                        if trimmed.rfind("(write-file").map(|i| {
                            // Check it's a top-level form (count parens before it)
                            let before = &trimmed[..i];
                            let open = before.chars().filter(|c| *c == '(').count();
                            let close = before.chars().filter(|c| *c == ')').count();
                            open == close // top-level if parens are balanced before it
                        }).unwrap_or(false) {
                            trimmed[..trimmed.rfind("(write-file").unwrap()].trim_end().to_string()
                        } else {
                            final_code
                        }
                    } else {
                        final_code
                    };

                    Ok::<(String, usize), String>((final_code, calls))
                })?;

                env.llm_calls += calls;

                // Save to file if path provided (no unescaping — this is source code, \n should stay as \n)
                if let Some(ref path) = save_path {
                    std::fs::write(path, &code).map_err(|e| format!("rlm-write: {}", e))?;
                }

                Ok(LispVal::Str(code))
            }
            // --- Env ---
            "env/get" => {
                let key = as_str(&args[0])?;
                match std::env::var(&key) {
                    Ok(v) => Ok(LispVal::Str(v)),
                    Err(_) => Ok(LispVal::Nil),
                }
            }

            // --- Snapshot builtins ---
            "snapshot" => {
                let snap = env.take_snapshot();
                let id = env.snapshots.len();
                env.snapshots.push(snap);
                Ok(LispVal::Num(id as i64))
            }
            "rollback" => {
                let snap = env.snapshots.pop().ok_or("rollback: no snapshots on stack")?;
                env.restore_snapshot(snap);
                Ok(LispVal::Bool(true))
            }
            "rollback-to" => {
                let idx = as_num(args.get(0).ok_or("rollback-to: need index")?)? as usize;
                let snap = env.snapshots.get(idx).ok_or_else(|| format!("rollback-to: no snapshot at index {}", idx))?.clone();
                env.restore_snapshot(snap);
                Ok(LispVal::Bool(true))
            }

            // rlm-set and rlm-get are now special forms (see lisp_eval_inner)

            // --- Print ---
            "print" | "println" => {
                let s: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                let out = s.join(" ");
                if name == "println" { println!("{}", out); } else { print!("{}", out); }
                Ok(LispVal::Str(out))
            }

            // --- RLM builtins ---
            "rlm/signature" => {
                let sig_name = as_str(&args[0])?;
                let inputs = match &args[1] {
                    LispVal::List(l) => l.iter().map(|v| as_str(v)).collect::<Result<Vec<_>,_>>()?,
                    _ => return Err("rlm/signature: inputs must be list".into()),
                };
                let outputs = match &args[2] {
                    LispVal::List(l) => l.iter().map(|v| as_str(v)).collect::<Result<Vec<_>,_>>()?,
                    _ => return Err("rlm/signature: outputs must be list".into()),
                };
                Ok(LispVal::Map(BTreeMap::from([
                    ("name".to_string(), LispVal::Str(sig_name)),
                    ("inputs".to_string(), LispVal::List(inputs.into_iter().map(LispVal::Str).collect())),
                    ("outputs".to_string(), LispVal::List(outputs.into_iter().map(LispVal::Str).collect())),
                ])))
            }
            "rlm/format-prompt" => {
                let sig = &args[0];
                let input_dict = &args[1];
                let sig_name = match sig { LispVal::Map(m) => m.get("name").and_then(|v| as_str(v).ok()).unwrap_or_default(), _ => "unknown".to_string() };
                let inputs = match sig { LispVal::Map(m) => match m.get("inputs") { Some(LispVal::List(l)) => l.iter().map(|v| as_str(v).unwrap_or_default()).collect::<Vec<_>>(), _ => vec![] }, _ => vec![] };
                let outputs = match sig { LispVal::Map(m) => match m.get("outputs") { Some(LispVal::List(l)) => l.iter().map(|v| as_str(v).unwrap_or_default()).collect::<Vec<_>>(), _ => vec![] }, _ => vec![] };
                let mut prompt = format!("You are a {} function.\n\nInputs:\n", sig_name);
                for inp in &inputs {
                    let val = match input_dict {
                        LispVal::Map(m) => m.get(inp).map(|v| v.to_string()).unwrap_or_else(|| "nil".to_string()),
                        _ => "nil".to_string(),
                    };
                    prompt.push_str(&format!("- {}: {}\n", inp, val));
                }
                prompt.push_str("\nOutputs:\n");
                for out in &outputs { prompt.push_str(&format!("- {}\n", out)); }
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
                let func = env.get(name).cloned()
                    .ok_or_else(|| format!("undefined: {}", name))?;
                call_val(&func, &args, env)
            }
        }
    } else if let LispVal::Lambda { params, rest_param, body, closed_env } = head {
        apply_lambda(params, &rest_param, body, closed_env, &args, env)
    } else if let LispVal::List(ll) = head {
        if ll.len() < 3 { return Err("inline lambda too short".into()); }
        let (params, rest_param) = parse_params(&ll[1])?;
        apply_lambda(&params, &rest_param, &ll[2], &vec![], &args, env)
    } else {
        Err("not callable".into())
    }
}

fn call_val(func: &LispVal, args: &[LispVal], env: &mut Env) -> Result<LispVal, String> {
    match func {
        LispVal::Lambda { params, rest_param, body, closed_env } => {
            apply_lambda(params, rest_param, body, closed_env, args, env)
        }
        LispVal::Macro { params, rest_param, body, closed_env } => {
            // Macros receive UNEVALUATED args, return code to be evaluated
            let expanded = apply_lambda(params, rest_param, body, closed_env, args, env)?;
            lisp_eval(&expanded, env)
        }
        LispVal::List(ll) if ll.len() >= 3 => {
            let (params, rest_param) = parse_params(&ll[1])?;
            apply_lambda(&params, &rest_param, &ll[2], &vec![], args, env)
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
        let s = match r.unwrap() { LispVal::Str(s) => s, _ => panic!("expected string") };
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
        let body = match r.unwrap() { LispVal::Str(s) => s, _ => panic!("expected string") };
        assert!(body.contains("httpbin.org"));
    }

    #[test]
    fn test_http_post() {
        let r = eval_str(r#"(http-post "https://httpbin.org/post" (to-json (dict "hello" "world")))"#);
        assert!(r.is_ok(), "http-post failed: {:?}", r);
        let body = match r.unwrap() { LispVal::Str(s) => s, _ => panic!("expected string") };
        assert!(body.contains("hello"));
    }

    #[test]
    fn test_http_get_json() {
        let r = eval_str(r#"(http-get-json "https://httpbin.org/json")"#);
        assert!(r.is_ok(), "http-get-json failed: {:?}", r);
        // Should return a LispVal::Map (parsed JSON)
        match r.unwrap() {
            LispVal::Map(_) => {},
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
        assert!(r.unwrap_err().contains("API_KEY"));
    }

    #[test]
    fn test_llm_code_no_api_key() {
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        let r = eval_str(r#"(llm-code "compute 2+2")"#);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("API_KEY"));
    }
}
