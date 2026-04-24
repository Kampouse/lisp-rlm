use std::collections::BTreeMap;

use crate::helpers::*;
use crate::parser::parse_all;
use crate::types::{get_stdlib_code, Env, LispVal};

// ---------------------------------------------------------------------------
// Hex helpers
// ---------------------------------------------------------------------------

/// Encode a byte slice as a lowercase hexadecimal string.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[(b >> 4) as usize] as char);
        s.push(HEX_CHARS[(b & 0xf) as usize] as char);
    }
    s
}

/// Decode a lowercase hexadecimal string into a byte vector.
///
/// Non-hex characters or odd-length strings will produce a shorter result
/// (invalid pairs are silently skipped).
pub fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()))
        .collect()
}

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
                            let var = match list.get(1) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                _ => return Err("define: need symbol".into()),
                            };
                            let val = match list.get(2) {
                                Some(v) => lisp_eval(v, env)?,
                                None => LispVal::Nil,
                            };
                            env.push(var, val);
                            return Ok(LispVal::Nil);
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
        if let Some((_, func)) = env.iter().rev().find(|(k, _)| k == name) {
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
                let i = as_num(args.get(0).ok_or("nth: need index")?)? as usize;
                match args.get(1) {
                    Some(LispVal::List(l)) => l.get(i).cloned().ok_or("index out of range".into()),
                    _ => Err("nth: need list".into()),
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
                let val = if args.len() == 1 { lisp_eval(&args[0], env)? } else { args[0].clone() };
                let j = lisp_to_json(&val);
                Ok(LispVal::Str(j.to_string()))
            }
            "to-json" => {
                let json_val = lisp_to_json(&args[0]);
                serde_json::to_string(&json_val).map(LispVal::Str).map_err(|e| format!("to-json: {}", e))
            }

            // --- Crypto (standalone using sha2/keccak crates or stubs) ---
            "sha256" => {

                let data = as_str(&args[0])?;
                // Use a simple SHA-256 implementation
                let hash = sha256_hash(data.as_bytes());
                Ok(LispVal::Str(hex_encode(&hash)))
            }
            "keccak256" => {
                let data = as_str(&args[0])?;
                let hash = keccak256_hash(data.as_bytes());
                Ok(LispVal::Str(hex_encode(&hash)))
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
                for elem in &lst {
                    acc = call_val(func, &[acc.clone(), elem.clone()], env)?;
                }
                Ok(acc)
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
                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("http-get: {}", e))?;
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
                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("http-post: {}", e))?;
                let body = rt.block_on(async {
                    let client = reqwest::Client::new();
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
                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("http-get-json: {}", e))?;
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

                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("llm: {}", e))?;
                let resp = rt.block_on(async {
                    let client = reqwest::Client::new();
                    let body = serde_json::json!({
                        "model": model,
                        "messages": [
                            {"role": "system", "content": "You are a helpful assistant with access to a Lisp runtime called lisp-rlm. You can reason about problems and suggest Lisp code using the available builtins: + - * / mod = < > <= >= not and or list cons car cdr nth len append reverse map filter reduce sort range zip find some every nil? list? number? string? bool? map? type? empty? str-concat str-contains str-split str-trim str-upcase str-downcase str-length str-substring str-index-of str-starts-with str-ends-with print println read-file write-file append-file file-exists? shell http-get http-post http-get-json from-json to-json sha256 keccak256 to-int to-float to-string define def let lambda if cond match quote quasiquote loop recur begin progn defmacro require try catch error"},
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
                    v["choices"][0]["message"]["content"].as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("llm: unexpected response: {}", text))
                })?;
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

                let builtin_ref = r#"You are a Lisp code generator for lisp-rlm. Return ONLY valid Lisp expressions. No explanations, no markdown fences.

Available builtins:
- Arithmetic: + - * / mod
- Comparison: = < > <= >= not
- Logic: and or
- Lists: list cons car cdr nth len append reverse map filter reduce sort range zip find some every
- Predicates: nil? list? number? string? bool? map? macro? type? empty?
- Strings: str-concat str-contains str-split str-trim str-upcase str-downcase str-length str-substring str-index-of str-starts-with str-ends-with
- IO: print println read-file write-file append-file file-exists? shell
- HTTP: http-get http-post http-get-json
- JSON: from-json to-json json-parse json-get json-get-in json-build
- LLM: llm llm-code
- Crypto: sha256 keccak256
- Types: to-int to-float to-string to-num
- Special forms: define def let lambda if cond match quote quasiquote unquote unquote-splicing loop recur begin progn defmacro require try catch error"#;

                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("llm-code: {}", e))?;
                let code_str = rt.block_on(async {
                    let client = reqwest::Client::new();
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
                    v["choices"][0]["message"]["content"].as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("llm-code: unexpected response: {}", text))
                })?;

                // Parse and eval the LLM-generated Lisp code
                let exprs = parse_all(&code_str)?;
                let mut result = LispVal::Nil;
                for expr in &exprs {
                    result = lisp_eval(expr, env)?;
                }
                Ok(result)
            }

            // --- Env ---
            "env/get" => {
                let key = as_str(&args[0])?;
                match std::env::var(&key) {
                    Ok(v) => Ok(LispVal::Str(v)),
                    Err(_) => Ok(LispVal::Nil),
                }
            }

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
                let func = env.iter().rev().find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
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

// ---------------------------------------------------------------------------
// Standalone crypto implementations
// ---------------------------------------------------------------------------

fn sha256_hash(data: &[u8]) -> [u8; 32] {
    // Minimal SHA-256 implementation
    let mut state = [0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                     0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let k: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g; g = f; f = e; e = d.wrapping_add(temp1);
            d = c; c = b; b = a; a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i*4..i*4+4].copy_from_slice(&state[i].to_be_bytes());
    }
    result
}

fn keccak256_hash(data: &[u8]) -> [u8; 32] {
    // Keccak-256 (SHA-3) implementation
    let mut state = [0u64; 25];
    let rate = 136; // bytes for Keccak-256

    let mut msg = data.to_vec();
    msg.push(0x01);
    while msg.len() % rate != 0 {
        msg.push(0);
    }
    *msg.last_mut().unwrap() |= 0x80;

    for block in msg.chunks(rate) {
        for i in 0..(rate / 8) {
            state[i] ^= u64::from_le_bytes(block[i*8..i*8+8].try_into().unwrap());
        }
        keccakf(&mut state);
    }

    let mut result = [0u8; 32];
    for i in 0..4 {
        result[i*8..i*8+8].copy_from_slice(&state[i].to_le_bytes());
    }
    result
}

fn keccakf(state: &mut [u64; 25]) {
    let rc: [u64; 24] = [
        0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000,
        0x000000000000808b, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
        0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
        0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
        0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
        0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
    ];
    let rotation: [[u32; 5]; 5] = [
        [0, 36, 3, 41, 18],
        [1, 44, 10, 45, 2],
        [62, 6, 43, 15, 61],
        [28, 55, 25, 21, 56],
        [27, 20, 39, 8, 14],
    ];

    for round in 0..24 {
        // θ
        let mut c = [0u64; 5];
        for x in 0..5 { c[x] = state[x] ^ state[x+5] ^ state[x+10] ^ state[x+15] ^ state[x+20]; }
        let mut d = [0u64; 5];
        for x in 0..5 { d[x] = c[(x+4)%5] ^ c[(x+1)%5].rotate_left(1); }
        for x in 0..5 { for y in 0..5 { state[x+5*y] ^= d[x]; } }

        // ρ and π
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                b[y + 5 * ((2*x+3*y)%5)] = state[x+5*y].rotate_left(rotation[x][y]);
            }
        }

        // χ
        for y in 0..5 {
            for x in 0..5 {
                state[x+5*y] = b[x+5*y] ^ ((!b[(x+1)%5+5*y]) & b[(x+2)%5+5*y]);
            }
        }

        // ι
        state[0] ^= rc[round];
    }
}

fn expand_quasiquote(form: &LispVal) -> Result<LispVal, String> {
    match form {
        LispVal::List(items) => {
            // Check for (unquote x)
            if items.len() == 2 {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "unquote" {
                        return Ok(items[1].clone());
                    }
                }
            }

            // Check if any element uses unquote-splicing
            let has_splice = items.iter().any(|item| {
                if let LispVal::List(splice_items) = item {
                    splice_items.len() == 2
                        && matches!(&splice_items[0], LispVal::Sym(s) if s == "unquote-splicing")
                } else {
                    false
                }
            });

            if has_splice {
                // Build (append seg1 seg2 ...) where each segment is either
                // (list expanded_elem ...) for non-splice elements
                // or the spliced expr directly for (unquote-splicing x)
                let mut segments: Vec<LispVal> = Vec::new();
                let mut current_list: Vec<LispVal> = vec![LispVal::Sym("list".to_string())];

                for item in items {
                    if let LispVal::List(splice_items) = item {
                        if splice_items.len() == 2 {
                            if let LispVal::Sym(s) = &splice_items[0] {
                                if s == "unquote-splicing" {
                                    // Flush current list segment
                                    if current_list.len() > 1 {
                                        segments.push(LispVal::List(current_list.clone()));
                                    }
                                    current_list = vec![LispVal::Sym("list".to_string())];
                                    // Add spliced expression directly
                                    segments.push(splice_items[1].clone());
                                    continue;
                                }
                            }
                        }
                    }
                    current_list.push(expand_quasiquote(item)?);
                }
                // Flush remaining items
                if current_list.len() > 1 {
                    segments.push(LispVal::List(current_list));
                }

                if segments.is_empty() {
                    Ok(LispVal::List(vec![LispVal::Sym("list".to_string())]))
                } else if segments.len() == 1 {
                    Ok(segments.into_iter().next().unwrap())
                } else {
                    let mut append_form = vec![LispVal::Sym("append".to_string())];
                    append_form.extend(segments);
                    Ok(LispVal::List(append_form))
                }
            } else {
                // No splicing — simple list construction
                let mut result_items = vec![LispVal::Sym("list".to_string())];
                for item in items {
                    result_items.push(expand_quasiquote(item)?);
                }
                Ok(LispVal::List(result_items))
            }
        }
        LispVal::Sym(_) => Ok(LispVal::List(vec![
            LispVal::Sym("quote".to_string()),
            form.clone(),
        ])),
        _ => Ok(form.clone()),
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
