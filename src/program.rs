//! Program-level evaluation: desugar top-level forms, compile, run through VM.
//!
//! Strategy: transform a sequence of top-level forms into a single zero-param
//! lambda `(lambda () <desugared-body>)`, compile it, and run it via the VM.
//!
//! `define` → `let` binding. All other forms pass through unchanged (the
//! compiler already handles `if`, `cond`, `let`, `begin`, `and`, `or`,
//! `when`, `unless`, `loop`/`recur`, `set!`, `match`, `case`, `try`, `do`,
//! etc. inside function bodies).
//!
//! Pre-processing forms (`defmacro`, `require`, `export`, `pure`) are handled
//! imperatively before compilation. They don't participate in the desugared
//! lambda — they modify the env directly.

use crate::bytecode::{run_compiled_lambda, try_compile_lambda};
use crate::helpers::parse_params;
use crate::types::{get_stdlib_code, Env, EvalState, LispVal};

/// Run a program (sequence of top-level forms) through the VM.
///
/// This is the replacement for `lisp_eval` at the top level. It:
/// 1. Flattens top-level `progn`/`begin` forms.
/// 2. Processes pre-processing forms (`defmacro`, `require`, `export`, `pure`)
///    imperatively, mutating `env` directly.
/// 3. Collects all `define` names for forward reference support.
/// 4. Desugars remaining forms into nested `let` + `lambda`.
/// 5. Compiles the desugared program as a zero-param lambda.
/// 6. Runs it through the VM.
///
/// Returns the value of the last expression, or `LispVal::Nil`.
pub fn run_program(
    forms: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    if forms.is_empty() {
        return Ok(LispVal::Nil);
    }

    // ── Phase 1: Flatten progn/begin + pre-processing ──
    let mut remaining: Vec<&LispVal> = Vec::new();

    for form in forms {
        // Flatten progn/begin at top level — their contents become top-level forms
        if let LispVal::List(list) = form {
            if let Some(LispVal::Sym(name)) = list.first() {
                if name.as_str() == "progn" || name.as_str() == "begin" {
                    for inner in &list[1..] {
                        remaining.push(inner);
                    }
                    continue;
                }
            }
        }
        remaining.push(form);
    }

    // Process defmacro, require, export, pure imperatively.
    let mut preprocessed: Vec<&LispVal> = Vec::new();
    for form in &remaining {
        if let LispVal::List(list) = form {
            if let Some(LispVal::Sym(name)) = list.first() {
                match name.as_str() {
                    "defmacro" => {
                        process_defmacro(list, env)?;
                        continue;
                    }
                    "require" => {
                        process_require(list, env, state)?;
                        continue;
                    }
                    "export" => {
                        process_export(list, env)?;
                        continue;
                    }
                    "pure" => {
                        // pure: type-check, then strip annotation and continue
                        // For now, just desugar it — the compiler will see it
                        // as a define with a :: annotation that it can ignore
                        // TODO: integrate typing if needed
                        preprocessed.push(form);
                        continue;
                    }
                    _ => {}
                }
            }
        }
        preprocessed.push(form);
    }

    // ── Phase 2: Two-pass forward reference collection ──
    // Collect all define names so the compiler knows about them at compile time.
    let forward_names = collect_define_names(&preprocessed);

    // Pre-populate env with forward-referenced names as Nil.
    // The compiler will see them in the env and can capture/resolve them.
    for name in &forward_names {
        if !env.contains(name) {
            env.insert_mut(name.clone(), LispVal::Nil);
        }
    }

    // Pre-populate env with first-class builtin function values.
    // This allows builtins like `list`, `append`, `map` to be captured
    // and passed as arguments to higher-order functions (map, filter, compose).
    for &name in crate::helpers::BUILTIN_NAMES {
        if !env.contains(name) {
            env.insert_mut(name.to_string(), LispVal::BuiltinFn(name.to_string()));
        }
    }

    // ── Phase 3: Desugar ──
    let desugared = desugar_program(&preprocessed);

    // ── Phase 4: Compile as zero-param lambda ──
    // Build: (lambda () <desugared-body>)
    let closed_env =
        std::sync::Arc::new(std::sync::RwLock::new(env.snapshot()));
    let lambda_body = desugared;

    let cl = try_compile_lambda(
        &[], // zero params
        &lambda_body,
        &closed_env
            .read()
            .unwrap()
            .clone()
            .into_iter()
            .collect::<Vec<_>>(),
        env,
        None, // no func_name
        None, // no pure_type
    )
    .ok_or_else(|| {
        format!(
            "run_program: compilation failed for desugared program. \
             Forms may contain unsupported constructs."
        )
    })?;

    // ── Phase 5: Run through VM ──
    let result = run_compiled_lambda(&cl, &[], env, state)?;

    Ok(result)
}

/// Process a `(defmacro name params body)` form imperatively.
fn process_defmacro(list: &[LispVal], env: &mut Env) -> Result<(), String> {
    let macro_name = match list.get(1) {
        Some(LispVal::Sym(s)) => s.clone(),
        _ => return Err("defmacro: first arg must be symbol".into()),
    };
    let (params, rest_param) =
        parse_params(list.get(2).ok_or("defmacro: need params")?)?;
    let body = list.get(3).ok_or("defmacro: need body")?.clone();
    let snap = env.get_or_create_scope_snapshot();
    env.push(
        macro_name,
        LispVal::Macro {
            params,
            rest_param,
            body: Box::new(body),
            closed_env: snap,
        },
    );
    Ok(())
}

/// Process a `(require module-name [prefix])` form imperatively.
fn process_require(
    list: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<(), String> {
    let module_name = match list.get(1) {
        Some(LispVal::Str(s)) => s.as_str().to_string(),
        _ => return Err("require: need string module name".into()),
    };
    let prefix: Option<String> = list.get(2).and_then(|v| {
        if let LispVal::Sym(s) = v {
            Some(s.clone())
        } else {
            None
        }
    });

    // Try to load stdlib module
    let code = get_stdlib_code(&module_name);
    if let Some(module_code) = code {
        let sub_forms = crate::parser::parse_all(&module_code)?;
        if let Some(ref pfx) = prefix {
            // Prefix all define names
            let prefixed: Vec<LispVal> = sub_forms
                .into_iter()
                .map(|f| prefix_form(&f, pfx))
                .collect();
            run_program(&prefixed, env, state)?;
        } else {
            run_program(&sub_forms, env, state)?;
        }
    } else {
        return Err(format!("require: unknown module '{}'", module_name));
    }
    Ok(())
}

/// Prefix all define names in a form with the given prefix.
fn prefix_form(form: &LispVal, prefix: &str) -> LispVal {
    if let LispVal::List(list) = form {
        if !list.is_empty() {
            if let LispVal::Sym(name) = &list[0] {
                if name == "define" && list.len() >= 2 {
                    if let LispVal::Sym(binding_name) = &list[1] {
                        let prefixed_name = format!("{}{}", prefix, binding_name);
                        let mut new_list = list.clone();
                        new_list[1] = LispVal::Sym(prefixed_name);
                        return LispVal::List(new_list);
                    } else if let LispVal::List(inner) = &list[1] {
                        // (define (name ...) ...) — prefix the function name
                        if !inner.is_empty() {
                            if let LispVal::Sym(fn_name) = &inner[0] {
                                let prefixed_name = format!("{}{}", prefix, fn_name);
                                let mut new_inner = inner.clone();
                                new_inner[0] = LispVal::Sym(prefixed_name);
                                let mut new_list = list.clone();
                                new_list[1] = LispVal::List(new_inner);
                                return LispVal::List(new_list);
                            }
                        }
                    }
                }
            }
        }
        // Recurse into sub-forms
        LispVal::List(list.iter().map(|f| prefix_form(f, prefix)).collect())
    } else {
        form.clone()
    }
}

/// Process an `(export name1 name2 ...)` form imperatively.
fn process_export(_list: &[LispVal], _env: &mut Env) -> Result<(), String> {
    // TODO: implement export tracking
    Ok(())
}

/// Collect all `define` names from a list of forms (for forward references).
fn collect_define_names(forms: &[&LispVal]) -> Vec<String> {
    let mut names = Vec::new();
    for form in forms {
        if let LispVal::List(list) = form {
            if let Some(LispVal::Sym(name)) = list.first() {
                if name.as_str() == "define" && list.len() >= 2 {
                    match &list[1] {
                        LispVal::Sym(s) => names.push(s.clone()),
                        LispVal::List(inner) if !inner.is_empty() => {
                            if let LispVal::Sym(s) = &inner[0] {
                                names.push(s.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    names
}

/// Desugar a list of top-level forms into a single expression.
///
/// Rules:
/// - `(define (f params...) body...)` → nested `let` with `lambda`
/// - `(define x expr)` → nested `let` with evaluated expr
/// - Everything else → passed through
///
/// The result is nested let-bindings wrapping the last expression.
fn desugar_program(forms: &[&LispVal]) -> LispVal {
    if forms.is_empty() {
        return LispVal::Nil;
    }

    let mut defines: Vec<(String, LispVal)> = Vec::new();
    let mut exprs: Vec<LispVal> = Vec::new();

    for form in forms {
        if let LispVal::List(list) = form {
            if let Some(LispVal::Sym(name)) = list.first() {
                if name.as_str() == "define" {
                    if let Some(binding) = desugar_define(list) {
                        defines.push(binding);
                        continue;
                    }
                }
            }
        }
        exprs.push((*form).clone());
    }

    // Build the body: any remaining expressions after the last define
    let body = if exprs.is_empty() {
        LispVal::Nil
    } else if exprs.len() == 1 {
        exprs.into_iter().next().unwrap()
    } else {
        LispVal::List(
            std::iter::once(LispVal::Sym("begin".into()))
                .chain(exprs.into_iter())
                .collect(),
        )
    };

    // Wrap defines as nested lets
    wrap_in_lets(defines, body)
}

/// Desugar a single `(define ...)` form into a (name, value_expr) pair.
fn desugar_define(list: &[LispVal]) -> Option<(String, LispVal)> {
    if list.len() < 2 {
        return None;
    }

    match &list[1] {
        // (define (name params...) body...)
        LispVal::List(inner) if !inner.is_empty() => {
            if let LispVal::Sym(name) = &inner[0] {
                let rest_param = inner
                    .iter()
                    .position(|v| matches!(v, LispVal::Sym(s) if s == "&rest"))
                    .and_then(|pos| inner.get(pos + 1))
                    .and_then(|v| {
                        if let LispVal::Sym(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });

                let fixed_params: Vec<String> = inner[1..] // skip function name
                    .iter()
                    .take_while(|v| !matches!(v, LispVal::Sym(s) if s == "&rest"))
                    .filter_map(|v| {
                        if let LispVal::Sym(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                let body = if list.len() > 3 {
                    LispVal::List(
                        vec![LispVal::Sym("begin".into())]
                            .into_iter()
                            .chain(list[2..].iter().cloned())
                            .collect(),
                    )
                } else {
                    list.get(2).cloned().unwrap_or(LispVal::Nil)
                };

                let param_list = if let Some(ref rest) = rest_param {
                    let mut p: Vec<LispVal> = fixed_params
                        .iter()
                        .map(|s| LispVal::Sym(s.clone()))
                        .collect();
                    p.push(LispVal::Sym("&rest".into()));
                    p.push(LispVal::Sym(rest.clone()));
                    LispVal::List(p)
                } else {
                    LispVal::List(
                        fixed_params
                            .iter()
                            .map(|s| LispVal::Sym(s.clone()))
                            .collect(),
                    )
                };

                let lambda = LispVal::List(vec![
                    LispVal::Sym("lambda".into()),
                    param_list,
                    body,
                ]);

                Some((name.clone(), lambda))
            } else {
                None
            }
        }
        // (define name expr)
        LispVal::Sym(name) => {
            let val_expr = list.get(2).cloned().unwrap_or(LispVal::Nil);
            Some((name.clone(), val_expr))
        }
        _ => None,
    }
}

/// Wrap a body expression in nested let bindings.
fn wrap_in_lets(
    bindings: Vec<(String, LispVal)>,
    body: LispVal,
) -> LispVal {
    bindings
        .into_iter()
        .rev()
        .fold(body, |inner, (name, val)| {
            if name.is_empty() {
                inner
            } else {
                LispVal::List(vec![
                    LispVal::Sym("let".into()),
                    LispVal::List(vec![LispVal::List(vec![
                        LispVal::Sym(name),
                        val,
                    ])]),
                    inner,
                ])
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_define_names() {
        let forms = vec![
            LispVal::List(vec![
                LispVal::Sym("define".into()),
                LispVal::Sym("x".into()),
                LispVal::Num(1),
            ]),
            LispVal::List(vec![
                LispVal::Sym("define".into()),
                LispVal::List(vec![
                    LispVal::Sym("foo".into()),
                    LispVal::Sym("a".into()),
                    LispVal::Sym("b".into()),
                ]),
                LispVal::Num(42),
            ]),
        ];
        let refs: Vec<&LispVal> = forms.iter().collect();
        let names = collect_define_names(&refs);
        assert_eq!(names, vec!["x", "foo"]);
    }

    #[test]
    fn test_desugar_simple_define() {
        let forms = vec![LispVal::List(vec![
            LispVal::Sym("define".into()),
            LispVal::Sym("x".into()),
            LispVal::Num(42),
        ])];
        let refs: Vec<&LispVal> = forms.iter().collect();
        let result = desugar_program(&refs);
        // Should be (let ((x 42)) nil)
        assert!(matches!(&result, LispVal::List(l) if !l.is_empty()));
    }

    #[test]
    fn test_desugar_function_define() {
        let forms = vec![LispVal::List(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![
                LispVal::Sym("add".into()),
                LispVal::Sym("a".into()),
                LispVal::Sym("b".into()),
            ]),
            LispVal::List(vec![
                LispVal::Sym("+".into()),
                LispVal::Sym("a".into()),
                LispVal::Sym("b".into()),
            ]),
        ])];
        let refs: Vec<&LispVal> = forms.iter().collect();
        let result = desugar_program(&refs);
        assert!(matches!(&result, LispVal::List(l) if !l.is_empty()));
    }

    #[test]
    fn test_wrap_in_lets_empty() {
        let body = LispVal::Num(42);
        let result = wrap_in_lets(vec![], body);
        assert_eq!(result, LispVal::Num(42));
    }

    #[test]
    fn test_wrap_in_lets_single() {
        let body = LispVal::Sym("x".into());
        let result = wrap_in_lets(vec![("x".into(), LispVal::Num(1))], body);
        assert!(matches!(&result, LispVal::List(l) if l.len() == 3));
    }

    #[test]
    fn test_desugar_define_with_rest() {
        let forms = vec![LispVal::List(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![
                LispVal::Sym("variadic".into()),
                LispVal::Sym("a".into()),
                LispVal::Sym("&rest".into()),
                LispVal::Sym("rest".into()),
            ]),
            LispVal::Sym("a".into()),
        ])];
        let refs: Vec<&LispVal> = forms.iter().collect();
        let result = desugar_program(&refs);
        assert!(matches!(&result, LispVal::List(l) if !l.is_empty()));
    }

    #[test]
    fn test_progn_flattening() {
        // progn at top level should be flattened
        let forms = vec![LispVal::List(vec![
            LispVal::Sym("progn".into()),
            LispVal::List(vec![
                LispVal::Sym("define".into()),
                LispVal::Sym("a".into()),
                LispVal::Num(1),
            ]),
            LispVal::List(vec![
                LispVal::Sym("define".into()),
                LispVal::Sym("b".into()),
                LispVal::Num(2),
            ]),
            LispVal::List(vec![
                LispVal::Sym("+".into()),
                LispVal::Sym("a".into()),
                LispVal::Sym("b".into()),
            ]),
        ])];
        let refs: Vec<&LispVal> = forms.iter().collect();
        let names = collect_define_names(&refs);
        assert_eq!(names, vec!["a", "b"]);
    }
}
