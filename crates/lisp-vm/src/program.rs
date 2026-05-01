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
use lisp_core::helpers::parse_params;
use lisp_core::types::{get_stdlib_code, Env, EvalState, LispVal};

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
    // Flatten begin/progn so defines inside them get processed by Phase 2/3.
    // Non-define expressions are re-grouped into (begin ...) by Phase 5,
    // so set_target_globals persists across set! and subsequent reads.
    let mut remaining: Vec<&LispVal> = Vec::new();

    for form in forms {
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
                        // pure: pass through — desugar_define_to_pair handles stripping
                        preprocessed.push(form);
                        continue;
                    }
                    "deftype" => {
                        // (deftype (Name param1 param2 ...) Variant1 Variant2 ...)
                        // (deftype Name Variant1 Variant2 ...)  — simplified, no params
                        process_deftype(list)?;
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
    for &name in lisp_core::helpers::BUILTIN_NAMES {
        if !env.contains(name) {
            env.insert_mut(name.to_string(), LispVal::BuiltinFn(name.to_string()));
        }
    }

    // ── Phase 3: Separate defines from expressions ──
    // Defines must be executed imperatively (writing to env) so that
    // subsequent code and nested run_program calls (require) can see them.
    let mut defines: Vec<((String, LispVal), Option<String>)> = Vec::new();
    let mut exprs: Vec<&LispVal> = Vec::new();
    let mut in_defines = true;

     for form in &preprocessed {
        if in_defines {
            let bindings = desugar_define_to_pairs(form)?;
            if !bindings.is_empty() {
                defines.extend(bindings);
                continue;
            }
            in_defines = false;
        }
        exprs.push(form);
    }

    // ── Phase 4: Execute defines imperatively ──
    // Each define's value expression is compiled and run separately.
    // The result is stored in env so subsequent defines/expressions can see it.
    for ((name, val_expr), pure_type) in &defines {
        // For function defines (lambda values), pass the name so self_name
        // is set for recursive calls via CallSelf.
        let func_name = if matches!(
            val_expr,
            LispVal::List(ref l) if !l.is_empty() && matches!(&l[0], LispVal::Sym(s) if s == "lambda")
        ) {
            Some(name.as_str())
        } else {
            None
        };

        let closed_env =
            std::sync::Arc::new(std::sync::RwLock::new(env.snapshot()));
        let cl = try_compile_lambda(
            &[],
            val_expr,
            &closed_env
                .read()
                .unwrap()
                .clone()
                .into_iter()
                .collect::<Vec<_>>(),
            env,
            func_name,
            pure_type.as_deref(),
        )
        .ok_or_else(|| {
            format!(
                "run_program: compilation failed for define '{}' = {:?}",
                name, val_expr
            )
        })?;

        let value = run_compiled_lambda(&cl, &[], env, state)?;
        env.insert_mut(name.clone(), value);
    }

    // ── Phase 5: Evaluate remaining expressions ──
    if exprs.is_empty() {
        return Ok(LispVal::Nil);
    }


    // Build body: single expr or (begin expr1 expr2 ...)
    let body = if exprs.len() == 1 {
        (*exprs[0]).clone()
    } else {
        LispVal::List(
            std::iter::once(LispVal::Sym("begin".into()))
                .chain(exprs.iter().map(|e| (*e).clone()))
                .collect(),
        )
    };

    let closed_env = std::sync::Arc::new(std::sync::RwLock::new(env.snapshot()));
    let cl = try_compile_lambda(
        &[],
        &body,
        &closed_env
            .read()
            .unwrap()
            .clone()
            .into_iter()
            .collect::<Vec<_>>(),
        env,
        None,
        None,
    )
        .ok_or_else(|| {
            format!(
                "run_program: compilation failed for body expression(s): {:?}",
                body
            )
        })?;

    // Share the live env via Arc so nested run_compiled_lambda calls (e.g., for-each
    // calling inner lambdas) can see each other's StoreGlobal mutations.
    state.global_env = Some(std::sync::Arc::new(std::sync::RwLock::new(env.clone())));
    let result = run_compiled_lambda(&cl, &[], env, state)?;
    state.global_env = None;
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

/// Process a `(deftype (Name ...) Variant1 Variant2 ...)` or `(deftype Name Variant1 ...)` form.
fn process_deftype(list: &[LispVal]) -> Result<(), String> {
    if list.len() < 2 {
        return Err("deftype: need type name and at least one variant".into());
    }

    // Determine type name and where variants start
    let type_name: String;
    let variants_start: usize;

    match &list[1] {
        LispVal::List(type_spec) => {
            // (deftype (Name param1 param2 ...) Variant1 Variant2 ...)
            if type_spec.is_empty() {
                return Err("deftype: type spec cannot be empty".into());
            }
            match &type_spec[0] {
                LispVal::Sym(s) => type_name = s.clone(),
                _ => return Err("deftype: type name must be a symbol".into()),
            }
            variants_start = 2;
        }
        LispVal::Sym(s) => {
            // (deftype Name Variant1 Variant2 ...) — simplified syntax
            type_name = s.clone();
            variants_start = 2;
        }
        _ => return Err("deftype: second arg must be a symbol or (Name params...) list".into()),
    }

    if list.len() < variants_start + 1 {
        return Err("deftype: need at least one variant".into());
    }

    // Parse variant specs: (VariantName field1 field2 ...) or just VariantName
    let mut variants: Vec<(&str, u8)> = Vec::new();
    for item in &list[variants_start..] {
        match item {
            LispVal::List(v) => {
                if v.is_empty() {
                    return Err("deftype: variant spec cannot be empty".into());
                }
                match &v[0] {
                    LispVal::Sym(name) => {
                        let n_fields = (v.len() - 1) as u8;
                        variants.push((name, n_fields));
                    }
                    _ => return Err("deftype: variant name must be a symbol".into()),
                }
            }
            LispVal::Sym(name) => {
                // Nullary variant
                variants.push((name, 0));
            }
            _ => return Err("deftype: variant must be a symbol or (Name fields...) list".into()),
        }
    }

    lisp_core::helpers::register_type(&type_name, &variants);
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
        let sub_forms = lisp_core::parser::parse_all(&module_code)?;
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
                match name.as_str() {
                    "define" if list.len() >= 2 => {
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
                    "progn" | "begin" => {
                        // Recurse into progn/begin to find nested defines
                        let inner_refs: Vec<&LispVal> = list[1..].iter().collect();
                        names.extend(collect_define_names(&inner_refs));
                    }
                    _ => {}
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
                    if let Some(binding) = desugar_define_to_pair(form).ok().flatten() {
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

/// Strip type annotations from a `(define ...)` form.
/// E.g. `(define (f x) :: int -> int body)` → `(define (f x) body)`
/// Finds `::` in the list and removes it plus all type tokens between it and the body.
fn strip_type_annotation(list: &[LispVal]) -> Vec<LispVal> {
    // Find the position of `::` (the type annotation marker)
    let Some(arrow_pos) = list.iter().position(|v| {
        matches!(v, LispVal::Sym(s) if s == "::")
    }) else {
        // No type annotation — return as-is
        return list.to_vec();
    };

    // Take everything before `::` plus the last element (the body)
    let mut result: Vec<LispVal> = list[..arrow_pos].to_vec();
    if let Some(body) = list.last() {
        result.push(body.clone());
    }
    result
}

/// Desugar a top-level form into zero or more (name, value_expr) pairs.
/// Handles `(pure (define ...) ...)` with multiple inner forms by type-checking
/// them all in a shared environment and returning all bindings.
/// Desugar a top-level form into define bindings with optional pure type annotations.
///
/// For `(pure (define f ...) (define g ...))`: type-checks all defines with a shared
/// environment, strips annotations, returns bindings with their inferred pure types.
/// For regular `(define f ...)`: returns a single binding with no pure type.
fn desugar_define_to_pairs(form: &LispVal) -> Result<Vec<((String, LispVal), Option<String>)>, String> {
    if let LispVal::List(list) = form {
        if let Some(LispVal::Sym(s)) = list.first() {
            if s == "pure" && list.len() >= 2 {
                // Collect all inner forms that are defines
                let inner_forms = &list[1..];
                let define_forms: Vec<&LispVal> = inner_forms
                    .iter()
                    .filter(|f| {
                        if let LispVal::List(l) = f {
                            matches!(l.first(), Some(LispVal::Sym(s)) if s == "define")
                        } else {
                            false
                        }
                    })
                    .collect();

                if !define_forms.is_empty() {
                    // Type-check all forms with a shared environment
                    let check_results = crate::typing::check_pure_block(&define_forms)?;

                    // Build a name→type map from check results
                    let mut type_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                    for r in &check_results {
                        type_map.insert(r.name.clone(), r.inferred_type.to_string());
                    }

                    // Strip annotations and desugar each define
                    let mut bindings = Vec::new();
                    for df in &define_forms {
                        if let LispVal::List(l) = df {
                            let stripped = strip_type_annotation(l);
                            if let Some(pair) = desugar_define(stripped.as_slice()) {
                                let pure_type = type_map.get(&pair.0).cloned();
                                bindings.push((pair, pure_type));
                            }
                        }
                    }
                    return Ok(bindings);
                }

                // Single non-define form inside pure — shouldn't be a binding
                return Ok(Vec::new());
            }
        }
        // Regular define (non-pure)
        Ok(desugar_define(list).into_iter().map(|p| (p, None)).collect())
    } else {
        Ok(Vec::new())
    }
}

/// Desugar a single `(define ...)` form (passed as &LispVal) into a (name, value_expr) pair.
/// Used by desugar_program (interpreter path) and for single-form pure blocks.
fn desugar_define_to_pair(form: &LispVal) -> Result<Option<(String, LispVal)>, String> {
    if let LispVal::List(list) = form {
        // Handle (pure (define ...)) — strip wrapper and type annotations
        if let Some(LispVal::Sym(s)) = list.first() {
            if s == "pure" && list.len() >= 2 {
                if let LispVal::List(inner) = &list[1] {
                    // Run type checker — propagate errors (type mismatches, impure forms)
                    let _check = crate::typing::check_pure_define(&list[1..])?;

                    // Strip :: type annotations from inner define
                    let stripped = strip_type_annotation(inner);
                    return Ok(desugar_define(stripped.as_slice()));
                }
            }
        }
        Ok(desugar_define(list))
    } else {
        Ok(None)
    }
}

/// Desugar a single `(define ...)` form into a (name, value_expr) pair.
fn desugar_define(list: &[LispVal]) -> Option<(String, LispVal)> {
    // Must start with "define"
    if list.is_empty() {
        return None;
    }
    if let LispVal::Sym(ref s) = list[0] {
        if s != "define" {
            return None;
        }
    } else {
        return None;
    }
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
