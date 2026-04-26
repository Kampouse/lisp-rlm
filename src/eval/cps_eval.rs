// ---------------------------------------------------------------------------
// Fully iterative evaluator: eval_step + handle_cont + catch_error
// ---------------------------------------------------------------------------

use crate::helpers::{is_builtin_name, is_truthy, match_pattern, parse_params};
use crate::parser::parse_all;
use crate::types::{get_stdlib_code, Env, EvalState, LispVal};
use std::sync::{Arc, RwLock};

use super::continuation::{Cont, EvalResult, Step};
use super::dispatch_types::{format_type, parse_type, RlType};
use super::{dispatch_call, expand_quasiquote, lisp_eval};

/// Create a Lambda with pre-compiled bytecode (cached at define-time).
/// Compilation is best-effort — if it fails, `compiled` is None and the
/// lambda falls back to tree-walking eval at call time.
fn make_lambda(
    params: Vec<String>,
    rest_param: Option<String>,
    body: Box<crate::types::LispVal>,
    closed_env: std::sync::Arc<std::sync::RwLock<im::HashMap<String, crate::types::LispVal>>>,
    pure_type: Option<String>,
    outer_env: &crate::types::Env,
    func_name: Option<&str>,
) -> crate::types::LispVal {
    use crate::types::LispVal;
    let compiled = crate::bytecode::try_compile_lambda(
        &params,
        &body,
        &closed_env
            .read()
            .unwrap()
            .clone()
            .into_iter()
            .collect::<Vec<_>>(),
        outer_env,
        func_name,
    )
    .map(|cl| Box::new(cl));
    // Auto-memoize: attach cache for pure compiled lambdas
    let memo_cache = if pure_type.is_some() && compiled.is_some() {
        Some(std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())))
    } else {
        None
    };
    LispVal::Lambda {
        params,
        rest_param,
        body,
        closed_env,
        pure_type,
        compiled,
        memo_cache,
    }
}

/// Evaluate a single expression (no recursion).
/// Returns Step::Done for atoms/immediates, or Step::EvalNext to evaluate a
/// sub-expression with continuations pushed onto the stack.
pub fn eval_step(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<Step, String> {
    match expr {
        LispVal::Nil
        | LispVal::Bool(_)
        | LispVal::Num(_)
        | LispVal::Float(_)
        | LispVal::Str(_)
        | LispVal::Lambda { .. }
        | LispVal::CaseLambda { .. }
        | LispVal::Macro { .. }
        | LispVal::Map(_)
        | LispVal::Memoized { .. } => Ok(Step::Done(expr.clone())),

        LispVal::Recur(_) => Err("recur outside loop".into()),

        LispVal::Sym(name) => {
            // Keywords (starting with :) are self-evaluating — used for type descriptors
            if name.starts_with(':') {
                Ok(Step::Done(expr.clone()))
            } else if let Some(v) = env.get(name) {
                Ok(Step::Done(v.clone()))
            } else if is_builtin_name(name) {
                Ok(Step::Done(expr.clone()))
            } else {
                Err(format!("undefined: {}", name))
            }
        }

        LispVal::List(list) if list.is_empty() => Ok(Step::Done(LispVal::Nil)),

        LispVal::List(list) => {
            if let LispVal::Sym(name) = &list[0] {
                // Type descriptor lists (head is a keyword starting with :) are self-evaluating
                if name.starts_with(':') {
                    return Ok(Step::Done(expr.clone()));
                }
                match name.as_str() {
                    // ── quote ──
                    "quote" => Ok(Step::Done(list.get(1).cloned().unwrap_or(LispVal::Nil))),

                    // ── quasiquote ──
                    "quasiquote" => {
                        let expanded =
                            expand_quasiquote(list.get(1).ok_or("quasiquote: need form")?)?;
                        Ok(Step::EvalNext {
                            expr: expanded,
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── define ──
                    "define" => match list.get(1) {
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
                                let lam = make_lambda(
                                    params,
                                    None,
                                    Box::new(body),
                                    env.get_or_create_scope_snapshot(),
                                    state.pending_pure_type.take(),
                                    env,
                                    Some(name.as_str()),
                                );
                                env.push(name.clone(), lam.clone());
                                env.propagate_to_scope_snapshot(&name, &lam);
                                Ok(Step::Done(LispVal::Nil))
                            } else {
                                Err("define: need symbol in head position".into())
                            }
                        }
                        Some(LispVal::Sym(s)) => match list.get(2) {
                            Some(v) => Ok(Step::EvalNext {
                                expr: v.clone(),
                                conts: vec![Cont::DefineSet { name: s.clone() }],
                                new_env: None,
                            }),
                            None => {
                                env.push(s.clone(), LispVal::Nil);
                                Ok(Step::Done(LispVal::Nil))
                            }
                        },
                        _ => Err("define: need symbol".into()),
                    },

                    // ── pure ──
                    "pure" => {
                        // Type-check a pure define, then evaluate a cleaned version.
                        // The inferred type is stored in state.pending_pure_type so
                        // the subsequent define/lambda creation picks it up.
                        let args = &list[1..];
                        match crate::typing::check_pure_define(args) {
                            Ok(result) => {
                                eprintln!("[pure] {} :: {} ✓", result.name, result.inferred_type);
                                state.pending_pure_type = Some(result.inferred_type.to_string());
                                // Build a clean define without the :: type annotation
                                let define_list = match list.get(1) {
                                    Some(LispVal::List(dl)) => dl.clone(),
                                    other => {
                                        return Err(format!(
                                            "pure: expected define form, got {:?}",
                                            other
                                        ))
                                    }
                                };
                                let clean = strip_type_annotation(&define_list);
                                eval_step(&LispVal::List(clean), env, state)
                            }
                            Err(type_err) => Err(format!("pure type error: {}", type_err)),
                        }
                    }

                    // ── set! ──
                    "set!" => {
                        let name = match list.get(1) {
                            Some(LispVal::Sym(s)) => s.clone(),
                            _ => return Err("set!: need symbol".into()),
                        };
                        match list.get(2) {
                            Some(v) => Ok(Step::EvalNext {
                                expr: v.clone(),
                                conts: vec![Cont::SetVal { name }],
                                new_env: None,
                            }),
                            None => Err("set!: need value".into()),
                        }
                    }

                    // ── if ──
                    "if" => Ok(Step::EvalNext {
                        expr: list.get(1).ok_or("if: need cond")?.clone(),
                        conts: vec![Cont::IfBranch {
                            then_branch: list.get(2).ok_or("if: need then")?.clone(),
                            else_branch: list.get(3).cloned().unwrap_or(LispVal::Nil),
                        }],
                        new_env: None,
                    }),

                    // ── cond ──
                    "cond" => {
                        let clauses: Vec<LispVal> = list[1..].to_vec();
                        if clauses.is_empty() {
                            return Ok(Step::Done(LispVal::Nil));
                        }
                        eval_cond_clauses(clauses, env)
                    }

                    // ── let ──
                    "let" => {
                        let bindings = match list.get(1) {
                            Some(LispVal::List(b)) => b,
                            _ => return Err("let: bindings must be list".into()),
                        };
                        let pairs: Vec<(String, LispVal)> = bindings
                            .iter()
                            .filter_map(|b| {
                                if let LispVal::List(pair) = b {
                                    if pair.len() == 2 {
                                        if let LispVal::Sym(name) = &pair[0] {
                                            return Some((name.clone(), pair[1].clone()));
                                        }
                                    }
                                }
                                None
                            })
                            .collect();
                        let body_exprs: Vec<LispVal> = list[2..].to_vec();
                        eval_let(pairs, body_exprs, env)
                    }

                    // ── let* (sequential bindings) ──
                    "let*" => {
                        let bindings = match list.get(1) {
                            Some(LispVal::List(b)) => b,
                            _ => return Err("let*: bindings must be list".into()),
                        };
                        let body_exprs: Vec<LispVal> = list[2..].to_vec();
                        if bindings.is_empty() {
                            return Ok(Step::EvalNext {
                                expr: LispVal::List(
                                    vec![LispVal::Sym("begin".into())]
                                        .into_iter()
                                        .chain(body_exprs)
                                        .collect(),
                                ),
                                conts: vec![],
                                new_env: None,
                            });
                        }
                        let mut result = if body_exprs.len() == 1 {
                            body_exprs.into_iter().next().unwrap()
                        } else {
                            LispVal::List(
                                vec![LispVal::Sym("begin".into())]
                                    .into_iter()
                                    .chain(body_exprs)
                                    .collect(),
                            )
                        };
                        for b in bindings.into_iter().rev() {
                            result = LispVal::List(vec![
                                LispVal::Sym("let".into()),
                                LispVal::List(vec![b.clone()]),
                                result,
                            ]);
                        }
                        Ok(Step::EvalNext {
                            expr: result,
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── letrec (recursive bindings) ──
                    "letrec" => {
                        let bindings = match list.get(1) {
                            Some(LispVal::List(b)) => b,
                            _ => return Err("letrec: bindings must be list".into()),
                        };
                        let body_exprs: Vec<LispVal> = list[2..].to_vec();
                        let names: Vec<String> = bindings
                            .iter()
                            .filter_map(|b| {
                                if let LispVal::List(pair) = b {
                                    if pair.len() == 2 {
                                        if let LispVal::Sym(name) = &pair[0] {
                                            return Some(name.clone());
                                        }
                                    }
                                }
                                None
                            })
                            .collect();
                        let val_exprs: Vec<LispVal> = bindings
                            .iter()
                            .filter_map(|b| {
                                if let LispVal::List(pair) = b {
                                    if pair.len() == 2 {
                                        return Some(pair[1].clone());
                                    }
                                }
                                None
                            })
                            .collect();
                        // Push all names as Nil first
                        for name in &names {
                            env.push(name.clone(), LispVal::Nil);
                        }
                        // Set up shared scope snapshot so lambdas created
                        // during set! share one Arc (for mutual recursion).
                        let _ = env.shared_scope_snapshot();
                        // Build: (begin (set! n1 v1) (set! n2 v2) ... body...)
                        let mut seq: Vec<LispVal> = Vec::new();
                        for (name, val) in names.iter().zip(val_exprs.iter()) {
                            seq.push(LispVal::List(vec![
                                LispVal::Sym("set!".into()),
                                LispVal::Sym(name.clone()),
                                val.clone(),
                            ]));
                        }
                        seq.extend(body_exprs);
                        // We already bound names in env. Eval the set!s and body as begin.
                        // Don't use eval_let (which snapshots env) — just eval begin directly.
                        if seq.is_empty() {
                            return Ok(Step::Done(LispVal::Nil));
                        }
                        Ok(Step::EvalNext {
                            expr: LispVal::List(
                                vec![LispVal::Sym("begin".into())]
                                    .into_iter()
                                    .chain(seq)
                                    .collect(),
                            ),
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── case ──
                    "case" => {
                        let key = list.get(1).ok_or("case: need key expr")?;
                        let clauses: Vec<LispVal> = list[2..].to_vec();
                        // Evaluate key, then match against datums
                        // We need to eval key first, then check clauses
                        // Use a continuation approach
                        Ok(Step::EvalNext {
                            expr: key.clone(),
                            conts: vec![Cont::CaseMatch { clauses }],
                            new_env: None,
                        })
                    }

                    // ── when ──
                    "when" => {
                        let test = list.get(1).ok_or("when: need test")?;
                        let body: Vec<LispVal> = list[2..].to_vec();
                        Ok(Step::EvalNext {
                            expr: LispVal::List(vec![
                                LispVal::Sym("if".into()),
                                test.clone(),
                                LispVal::List(
                                    vec![LispVal::Sym("begin".into())]
                                        .into_iter()
                                        .chain(body)
                                        .collect(),
                                ),
                            ]),
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── unless ──
                    "unless" => {
                        let test = list.get(1).ok_or("unless: need test")?;
                        let body: Vec<LispVal> = list[2..].to_vec();
                        Ok(Step::EvalNext {
                            expr: LispVal::List(vec![
                                LispVal::Sym("if".into()),
                                LispVal::List(vec![LispVal::Sym("not".into()), test.clone()]),
                                LispVal::List(
                                    vec![LispVal::Sym("begin".into())]
                                        .into_iter()
                                        .chain(body)
                                        .collect(),
                                ),
                            ]),
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── delay/force (lazy evaluation) ──
                    "delay" => {
                        // (delay expr) → create a promise (thunk)
                        let body = list.get(1).ok_or("delay: need expression")?;
                        Ok(Step::Done(LispVal::List(vec![
                            LispVal::Sym("promise".into()),
                            body.clone(),
                            LispVal::Bool(false), // not yet forced
                        ])))
                    }

                    // ── define-values ──
                    "define-values" => {
                        // (define-values (a b c) expr)
                        let names = match list.get(1) {
                            Some(LispVal::List(n)) => n
                                .iter()
                                .filter_map(|v| {
                                    if let LispVal::Sym(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>(),
                            _ => return Err("define-values: need name list".into()),
                        };
                        let expr = list.get(2).ok_or("define-values: need expression")?;
                        // Eval expr, then bind names to results
                        Ok(Step::EvalNext {
                            expr: expr.clone(),
                            conts: vec![Cont::DefineValues { names }],
                            new_env: None,
                        })
                    }

                    // ── let-values ──
                    "let-values" | "let*-values" => {
                        // (let-values (((a b) expr)) body...)
                        // Simplified: eval each binding, destructure, then eval body
                        let bindings = match list.get(1) {
                            Some(LispVal::List(b)) => b,
                            _ => return Err("let-values: need bindings".into()),
                        };
                        let body_exprs: Vec<LispVal> = list[2..].to_vec();
                        // Collect all names and exprs
                        let mut all_names: Vec<Vec<String>> = Vec::new();
                        let mut all_exprs: Vec<LispVal> = Vec::new();
                        for b in bindings {
                            if let LispVal::List(pair) = b {
                                if pair.len() == 2 {
                                    let names = match &pair[0] {
                                        LispVal::List(n) => n
                                            .iter()
                                            .filter_map(|v| {
                                                if let LispVal::Sym(s) = v {
                                                    Some(s.clone())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect(),
                                        LispVal::Sym(s) => vec![s.clone()],
                                        _ => vec![],
                                    };
                                    all_names.push(names);
                                    all_exprs.push(pair[1].clone());
                                }
                            }
                        }
                        // Eval each expr, bind results, then body
                        if all_exprs.is_empty() {
                            return Ok(Step::EvalNext {
                                expr: LispVal::List(
                                    vec![LispVal::Sym("begin".into())]
                                        .into_iter()
                                        .chain(body_exprs)
                                        .collect(),
                                ),
                                conts: vec![],
                                new_env: None,
                            });
                        }
                        // Desugar to: eval first expr, bind, then let-values for rest
                        Ok(Step::EvalNext {
                            expr: all_exprs[0].clone(),
                            conts: vec![Cont::LetValuesBind {
                                names: all_names,
                                remaining_exprs: all_exprs[1..].to_vec(),
                                body_exprs,
                                current_idx: 0,
                            }],
                            new_env: None,
                        })
                    }

                    // ── do ──
                    "do" => {
                        // (do ((var init step) ...) (test result...) body...)
                        let var_clauses = match list.get(1) {
                            Some(LispVal::List(v)) => v,
                            _ => return Err("do: need variable clauses".into()),
                        };
                        let test_clause = match list.get(2) {
                            Some(LispVal::List(t)) => t,
                            _ => return Err("do: need test clause".into()),
                        };
                        let body: Vec<LispVal> = list[3..].to_vec();
                        // Desugar to named let loop
                        let test_expr = test_clause.get(0).ok_or("do: need test expr")?;
                        let result_exprs: Vec<LispVal> = test_clause[1..].to_vec();
                        let loop_var = "__do_loop";
                        // Build: (let loop ((var1 init1) ...) (if test (begin result...) (begin body... (loop step1 ...))))
                        let mut var_bindings: Vec<LispVal> = Vec::new();
                        let mut step_exprs: Vec<LispVal> = Vec::new();
                        let mut var_names: Vec<LispVal> = Vec::new();
                        for vc in var_clauses {
                            if let LispVal::List(parts) = vc {
                                if parts.len() >= 2 {
                                    var_bindings.push(LispVal::List(vec![
                                        parts[0].clone(),
                                        parts[1].clone(),
                                    ]));
                                    var_names.push(parts[0].clone());
                                    let step =
                                        parts.get(2).cloned().unwrap_or_else(|| parts[0].clone());
                                    step_exprs.push(step);
                                }
                            }
                        }
                        let recursive_call = LispVal::List(
                            vec![LispVal::Sym(loop_var.into())]
                                .into_iter()
                                .chain(step_exprs.clone())
                                .collect(),
                        );
                        let mut loop_body: Vec<LispVal> = body.clone();
                        loop_body.push(recursive_call);
                        let else_branch = LispVal::List(
                            vec![LispVal::Sym("begin".into())]
                                .into_iter()
                                .chain(loop_body)
                                .collect(),
                        );
                        let then_branch = LispVal::List(
                            vec![LispVal::Sym("begin".into())]
                                .into_iter()
                                .chain(result_exprs)
                                .collect(),
                        );
                        let if_expr = LispVal::List(vec![
                            LispVal::Sym("if".into()),
                            test_expr.clone(),
                            then_branch.clone(),
                            else_branch,
                        ]);
                        // Named let: (let loop ((v1 i1) ...) body)
                        // We don't have named let, so use loop/recur pattern
                        // Actually, just desugar to a recursive lambda
                        let _params: Vec<String> = var_names
                            .iter()
                            .filter_map(|v| {
                                if let LispVal::Sym(s) = v {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let init_vals: Vec<LispVal> = var_bindings
                            .iter()
                            .filter_map(|b| {
                                if let LispVal::List(p) = b {
                                    p.get(1).cloned()
                                } else {
                                    None
                                }
                            })
                            .collect();
                        // ((lambda (v1 v2 ...) (if test result (begin body... ((lambda ...))))) init1 init2 ...)
                        // Too complex. Use Y-combinator? No, just use set! approach.
                        // Simpler: desugar to loop/recur
                        let _loop_body_expr = if_expr;
                        let _result_expr = LispVal::List(
                            vec![LispVal::Sym("loop".into())]
                                .into_iter()
                                .chain(init_vals.clone())
                                .collect(),
                        );
                        // Use the existing loop/recur
                        // (loop ((v1 init1) (v2 init2)) (if test result (begin body... (recur step1 step2))))
                        let mut loop_bindings: Vec<LispVal> = Vec::new();
                        for (name, init) in var_names.iter().zip(init_vals.iter()) {
                            loop_bindings.push(LispVal::List(vec![name.clone(), init.clone()]));
                        }
                        let recur_expr = LispVal::List(
                            vec![LispVal::Sym("recur".into())]
                                .into_iter()
                                .chain(step_exprs)
                                .collect(),
                        );
                        // Rebuild with recur
                        let mut loop_body2: Vec<LispVal> = body;
                        loop_body2.push(recur_expr);
                        let else_branch2 = LispVal::List(
                            vec![LispVal::Sym("begin".into())]
                                .into_iter()
                                .chain(loop_body2)
                                .collect(),
                        );
                        let if_expr2 = LispVal::List(vec![
                            LispVal::Sym("if".into()),
                            test_expr.clone(),
                            then_branch.clone(),
                            else_branch2,
                        ]);
                        Ok(Step::EvalNext {
                            expr: LispVal::List(vec![
                                LispVal::Sym("loop".into()),
                                LispVal::List(loop_bindings),
                                if_expr2,
                            ]),
                            conts: vec![],
                            new_env: None,
                        })
                    }

                    // ── lambda ──
                    "lambda" => {
                        let (params, rest_param) =
                            parse_params(list.get(1).ok_or("lambda: need params")?)?;
                        let body = list.get(2).ok_or("lambda: need body")?;
                        Ok(Step::Done(make_lambda(
                            params,
                            rest_param,
                            Box::new(body.clone()),
                            env.get_or_create_scope_snapshot(),
                            state.pending_pure_type.take(),
                            env,
                            None,
                        )))
                    }

                    // ── memoize ──
                    "memoize" => {
                        // (memoize f) — returns a memoized version of f
                        // Eval the argument to get a lambda, then wrap it
                        let func_expr = list.get(1).ok_or("memoize: need a function")?;
                        Ok(Step::EvalNext {
                            expr: func_expr.clone(),
                            conts: vec![Cont::Memoize],
                            new_env: None,
                        })
                    }

                    // ── case-lambda ──
                    "case-lambda" => {
                        // (case-lambda (() 'zero) ((x) x) ((x y) (cons x y)) (args (cons 'many args)))
                        // Last form: single symbol catches all args (rest)
                        let cases: Vec<(Vec<String>, Option<String>, LispVal)> = list[1..]
                            .iter()
                            .filter_map(|clause| {
                                if let LispVal::List(parts) = clause {
                                    if parts.is_empty() {
                                        return None;
                                    }
                                    // Check if params is a single symbol (catch-all)
                                    if let LispVal::Sym(s) = &parts[0] {
                                        // Single symbol = rest param, catches all args
                                        let body = if parts.len() > 1 {
                                            LispVal::List(
                                                vec![LispVal::Sym("begin".into())]
                                                    .into_iter()
                                                    .chain(parts[1..].iter().cloned())
                                                    .collect(),
                                            )
                                        } else {
                                            LispVal::Nil
                                        };
                                        return Some((vec![], Some(s.clone()), body));
                                    }
                                    let (params, rest) = parse_params(&parts[0]).ok()?;
                                    let body = if parts.len() > 1 {
                                        LispVal::List(
                                            vec![LispVal::Sym("begin".into())]
                                                .into_iter()
                                                .chain(parts[1..].iter().cloned())
                                                .collect(),
                                        )
                                    } else {
                                        LispVal::Nil
                                    };
                                    Some((params, rest, body))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        Ok(Step::Done(LispVal::CaseLambda {
                            cases,
                            closed_env: env.get_or_create_scope_snapshot(),
                        }))
                    }

                    // ── defmacro ──
                    "defmacro" => {
                        let macro_name = match list.get(1) {
                            Some(LispVal::Sym(s)) => s.clone(),
                            _ => return Err("defmacro: first arg must be symbol".into()),
                        };
                        let (params, rest_param) =
                            parse_params(list.get(2).ok_or("defmacro: need params")?)?;
                        let body = list.get(3).ok_or("defmacro: need body")?;
                        let snap = env.get_or_create_scope_snapshot();
                        env.push(
                            macro_name,
                            LispVal::Macro {
                                params,
                                rest_param,
                                body: Box::new(body.clone()),
                                closed_env: snap,
                            },
                        );
                        Ok(Step::Done(LispVal::Nil))
                    }

                    // ── begin / progn ──
                    "progn" | "begin" => {
                        let exprs: Vec<LispVal> = list[1..].to_vec();
                        if exprs.is_empty() {
                            return Ok(Step::Done(LispVal::Nil));
                        }
                        let last = exprs.len() - 1;
                        let _conts: Vec<Cont> = Vec::new();
                        // Push BeginSeq for all but the last (in reverse for stack order,
                        // but since we use extend, we push from last to first... actually,
                        // we need the FIRST remaining to be popped first. So push in order.)
                        // Actually: extend pushes them bottom-to-top. pop() gets the last one.
                        // We want the first item in `remaining` to be evaluated next.
                        // So we need remaining[0] at the TOP of the stack → push it LAST.
                        // This means we reverse the order when building conts.
                        // But BeginSeq.remaining contains [next, next+1, ...].
                        // We evaluate next first, so it should be at the top of stack
                        // → the last element in `conts` vec → we need to push them
                        // in order: first item pushed first, last pushed last.
                        // stack.extend(conts) adds them in order, pop() gets the last.
                        // So if we want [e2, e3] to be evaluated in order:
                        //   conts = [BeginSeq{remaining:[e3]}, BeginSeq{remaining:[]}]
                        //   No wait, that's wrong.
                        //
                        // Let's think again. For (begin e1 e2 e3):
                        // - Eval e1 first, then we need to eval e2, then e3.
                        // - So cont stack should be: [BeginSeq{remaining:[e2,e3]}] on top
                        // - After e1, pop → BeginSeq{remaining:[e2,e3]} → eval e2 with BeginSeq{remaining:[e3]}
                        // - After e2, pop → BeginSeq{remaining:[e3]} → eval e3 with no cont
                        // - After e3, stack empty → Done
                        //
                        // So we push one cont: BeginSeq{remaining:[e2,e3]}.
                        // handle_cont pops remaining[0] as next, pushes BeginSeq{remaining:rest}.
                        // This works! But for N items we'd push N-1 conts one at a time.
                        //
                        // Simpler: push a SINGLE BeginSeq with all remaining exprs.
                        // eval e1, push BeginSeq{[e2,e3]}, done.
                        // handle_cont: eval remaining[0], push BeginSeq{remaining[1..]}.
                        //
                        // This means exactly 1 cont pushed here.
                        Ok(Step::EvalNext {
                            expr: exprs[0].clone(),
                            conts: if last > 0 {
                                vec![Cont::BeginSeq {
                                    remaining: exprs[1..].to_vec(),
                                }]
                            } else {
                                vec![]
                            },
                            new_env: None,
                        })
                    }

                    // ── and ──
                    "and" => {
                        if list.len() == 1 {
                            return Ok(Step::Done(LispVal::Bool(true)));
                        }
                        let exprs: Vec<LispVal> = list[1..].to_vec();
                        let last = exprs.len() - 1;
                        Ok(Step::EvalNext {
                            expr: exprs[0].clone(),
                            conts: if last > 0 {
                                vec![Cont::AndNext {
                                    remaining: exprs[1..].to_vec(),
                                }]
                            } else {
                                vec![]
                            },
                            new_env: None,
                        })
                    }

                    // ── or ──
                    "or" => {
                        if list.len() == 1 {
                            return Ok(Step::Done(LispVal::Bool(false)));
                        }
                        let exprs: Vec<LispVal> = list[1..].to_vec();
                        let last = exprs.len() - 1;
                        Ok(Step::EvalNext {
                            expr: exprs[0].clone(),
                            conts: if last > 0 {
                                vec![Cont::OrNext {
                                    remaining: exprs[1..].to_vec(),
                                }]
                            } else {
                                vec![]
                            },
                            new_env: None,
                        })
                    }

                    // ── not ──
                    "not" => Ok(Step::EvalNext {
                        expr: list.get(1).ok_or("not: need arg")?.clone(),
                        conts: vec![Cont::NotArg],
                        new_env: None,
                    }),

                    // ── try ──
                    "try" => {
                        let expr_to_try = list.get(1).ok_or("try: need expression")?;
                        let catch_clause = list.get(2).ok_or("try: need catch clause")?;
                        let (var, catch_body) = parse_catch_clause(catch_clause)?;
                        Ok(Step::EvalNext {
                            expr: expr_to_try.clone(),
                            conts: vec![Cont::TryCatch {
                                var,
                                catch_body_exprs: catch_body,
                            }],
                            new_env: None,
                        })
                    }

                    // ── fork (speculative evaluation) ──
                    // (fork expr) — evaluates expr in a forked env, returns result.
                    // Parent's env is unchanged. O(1) via im::HashMap structural sharing.
                    "fork" => {
                        let body = list.get(1).ok_or("fork: need expression")?;
                        let mut forked_env = env.clone();
                        let mut forked_state = state.clone();
                        // Each fork gets its own provider clone
                        forked_state.llm_provider =
                            state.llm_provider.as_ref().map(|p| p.box_clone());
                        let result = super::lisp_eval(body, &mut forked_env, &mut forked_state)?;
                        Ok(Step::Done(result))
                    }

                    // ── contract (runtime type-checked lambda) ──
                    // (contract ((param :type ...) → :ret-type) body...)
                    // Creates a lambda that checks arg types on entry, return type on exit.
                    "contract" => {
                        let sig = list.get(1).ok_or("contract: need signature")?;
                        let body_expr = list.get(2).ok_or("contract: need body")?;

                        // Parse signature: ((p1 :t1 p2 :t2 ...) → :ret)
                        let (params, param_types, ret_type) = parse_contract_sig(sig)?;

                        let lam = make_lambda(
                            params,
                            None,
                            Box::new(body_expr.clone()),
                            env.get_or_create_scope_snapshot(),
                            None,
                            env,
                            None,
                        );

                        // Wrap in a Contract value
                        Ok(Step::Done(LispVal::Map({
                            let mut m = im::HashMap::new();
                            m.insert("__contract".into(), LispVal::Bool(true));
                            m.insert("fn".into(), lam);
                            m.insert(
                                "param_types".into(),
                                LispVal::List(
                                    param_types
                                        .into_iter()
                                        .map(|t| LispVal::Str(format_type(&t)))
                                        .collect(),
                                ),
                            );
                            m.insert(
                                "return_type".into(),
                                LispVal::Str(
                                    ret_type
                                        .map(|t| format_type(&t))
                                        .unwrap_or_else(|| ":any".into()),
                                ),
                            );
                            m
                        })))
                    }

                    // ── match ──
                    "match" => Ok(Step::EvalNext {
                        expr: list.get(1).ok_or("match: need expr")?.clone(),
                        conts: vec![Cont::MatchScrutinee {
                            val: LispVal::Nil, // placeholder, will be replaced by value
                            arms: list[2..].to_vec(),
                        }],
                        new_env: None,
                    }),

                    // ── loop ──
                    "loop" => {
                        let bindings = match list.get(1) {
                            Some(LispVal::List(b)) => b,
                            _ => return Err("loop: bindings must be list".into()),
                        };
                        let body = list.get(2).ok_or("loop: need body")?.clone();
                        let is_pair_style = bindings.iter().all(|b| matches!(b, LispVal::List(_)));

                        let pairs: Vec<(String, LispVal)> = if is_pair_style {
                            bindings
                                .iter()
                                .filter_map(|b| {
                                    if let LispVal::List(pair) = b {
                                        if pair.len() == 2 {
                                            if let LispVal::Sym(name) = &pair[0] {
                                                return Some((name.clone(), pair[1].clone()));
                                            }
                                        }
                                    }
                                    None
                                })
                                .collect()
                        } else {
                            if bindings.len() % 2 != 0 {
                                return Err("loop: flat bindings need even count".into());
                            }
                            let mut pairs = Vec::new();
                            let mut i = 0;
                            while i < bindings.len() {
                                if let LispVal::Sym(name) = &bindings[i] {
                                    pairs.push((name.clone(), bindings[i + 1].clone()));
                                } else {
                                    return Err(format!(
                                        "loop: binding name must be sym, got {}",
                                        bindings[i]
                                    ));
                                }
                                i += 2;
                            }
                            pairs
                        };

                        if pairs.is_empty() {
                            // No bindings, just run the body
                            let snap = env.snapshot();
                            let body_clone = body.clone();
                            Ok(Step::EvalNext {
                                expr: body,
                                conts: vec![Cont::LoopIter {
                                    binding_names: vec![],
                                    binding_vals: vec![],
                                    body: body_clone,
                                    snapshot: snap,
                                }],
                                new_env: None,
                            })
                        } else {
                            // Evaluate first binding init
                            let (first_name, first_val_expr) = &pairs[0];
                            let remaining = pairs[1..].to_vec();
                            Ok(Step::EvalNext {
                                expr: first_val_expr.clone(),
                                conts: vec![Cont::LoopBind {
                                    names: vec![first_name.clone()],
                                    vals: vec![],
                                    remaining,
                                    body,
                                }],
                                new_env: None,
                            })
                        }
                    }

                    // ── recur ──
                    "recur" => {
                        let args: Vec<LispVal> = list[1..].to_vec();
                        if args.is_empty() {
                            return Ok(Step::Done(LispVal::Recur(vec![])));
                        }
                        Ok(Step::EvalNext {
                            expr: args[0].clone(),
                            conts: vec![Cont::RecurArg {
                                done: vec![],
                                remaining: args[1..].to_vec(),
                            }],
                            new_env: None,
                        })
                    }

                    // ── require ──
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
                        let marker = format!("__loaded_{}__{}", module_name, prefix.unwrap_or(""));
                        if env.contains(&marker) {
                            return Ok(Step::Done(LispVal::Nil));
                        }
                        let code: String = if let Some(stdlib_code) = get_stdlib_code(module_name) {
                            stdlib_code.to_string()
                        } else {
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
                            std::fs::read_to_string(&path)
                                .map_err(|e| format!("require: cannot load '{}': {}", path, e))?
                        };
                        if let Some(pfx) = prefix {
                            let mut module_env = Env::new();
                            let module_exprs = parse_all(&code)?;
                            for expr in &module_exprs {
                                lisp_eval(expr, &mut module_env, state)?;
                            }
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
                                }
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
                                lisp_eval(expr, env, state)?;
                            }
                        }
                        env.push(marker, LispVal::Bool(true));
                        Ok(Step::Done(LispVal::Nil))
                    }

                    // ── export ──
                    "export" => {
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
                        Ok(Step::Done(LispVal::Bool(true)))
                    }

                    // ── final ──
                    "final" => Ok(Step::EvalNext {
                        expr: list.get(1).ok_or("final: need value")?.clone(),
                        conts: vec![Cont::FinalVal],
                        new_env: None,
                    }),

                    // ── final-var ──
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
                        state
                            .rlm_state
                            .insert("Final".to_string(), LispVal::Bool(true));
                        state.rlm_state.insert("result".to_string(), val);
                        Ok(Step::Done(LispVal::Bool(true)))
                    }

                    // ── assert ──
                    "assert" => Ok(Step::EvalNext {
                        expr: list.get(1).ok_or("assert: need condition")?.clone(),
                        conts: vec![Cont::AssertCheck {
                            message: list.get(2).map(|v| v.to_string()),
                        }],
                        new_env: None,
                    }),

                    // ── rlm-set ──
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
                        match list.get(2) {
                            Some(v) => Ok(Step::EvalNext {
                                expr: v.clone(),
                                conts: vec![Cont::RlmSetVal { name: key }],
                                new_env: None,
                            }),
                            None => {
                                state.rlm_state.insert(key, LispVal::Nil);
                                Ok(Step::Done(LispVal::Bool(true)))
                            }
                        }
                    }

                    // ── rlm-get ──
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
                        Ok(Step::Done(
                            state.rlm_state.get(&key).cloned().unwrap_or(LispVal::Nil),
                        ))
                    }

                    // ── function call (symbol head) ──
                    _ => {
                        // Check for macros first
                        if let LispVal::Sym(name) = &list[0] {
                            if let Some(func) = env.get(name) {
                                if matches!(func, LispVal::Macro { .. }) {
                                    let func_clone = func.clone();
                                    let raw_args: Vec<LispVal> = list[1..].to_vec();
                                    let r =
                                        crate::eval::call_val(&func_clone, &raw_args, env, state)?;
                                    match r {
                                        crate::eval::EvalResult::Value(v) => {
                                            return Ok(Step::Done(v))
                                        }
                                        crate::eval::EvalResult::TailCall {
                                            expr,
                                            env: tail_env,
                                        } => {
                                            return Ok(Step::EvalNext {
                                                expr,
                                                conts: vec![],
                                                new_env: Some(tail_env),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        // Fast path: if head resolves to a compiled lambda and all args are simple,
                        // skip CPS arg collection and call run_compiled_lambda directly.
                        let raw_args: Vec<LispVal> = list[1..].to_vec();
                        if let LispVal::Sym(name) = &list[0] {
                            if let Some(LispVal::Lambda {
                                params,
                                rest_param: None,
                                compiled: Some(ref cl),
                                ..
                            }) = env.get(name)
                            {
                                if raw_args.len() == params.len() {
                                    // Resolve args — fast path for symbols and self-evaluating values
                                    let mut resolved_args: Vec<LispVal> =
                                        Vec::with_capacity(raw_args.len());
                                    let mut all_simple = true;
                                    for arg in &raw_args {
                                        match arg {
                                            LispVal::Sym(s) => {
                                                if s.starts_with(':') {
                                                    resolved_args.push(arg.clone());
                                                } else if let Some(v) = env.get(s) {
                                                    resolved_args.push(v.clone());
                                                } else if is_builtin_name(s) {
                                                    resolved_args.push(arg.clone());
                                                } else {
                                                    all_simple = false;
                                                    break;
                                                }
                                            }
                                            LispVal::Nil
                                            | LispVal::Bool(_)
                                            | LispVal::Num(_)
                                            | LispVal::Float(_)
                                            | LispVal::Str(_)
                                            | LispVal::Map(_) => {
                                                resolved_args.push(arg.clone());
                                            }
                                            _ => {
                                                all_simple = false;
                                                break;
                                            }
                                        }
                                    }
                                    if all_simple {
                                        match crate::bytecode::run_compiled_lambda(
                                            cl,
                                            &resolved_args,
                                            env,
                                            state,
                                        ) {
                                            Ok(v) => return Ok(Step::Done(v)),
                                            Err(_) => {
                                                // Bytecode failed — fall through to CPS
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // CPS arg collection for regular function calls
                        if raw_args.is_empty() {
                            // No args — dispatch directly
                            let r = crate::eval::dispatch_call(list, env, state)?;
                            match r {
                                crate::eval::EvalResult::Value(v) => Ok(Step::Done(v)),
                                crate::eval::EvalResult::TailCall {
                                    expr,
                                    env: tail_env,
                                } => Ok(Step::EvalNext {
                                    expr,
                                    conts: vec![],
                                    new_env: Some(tail_env),
                                }),
                            }
                        } else {
                            // Evaluate args through CPS trampoline
                            let env_snapshot = env.snapshot();
                            Ok(Step::EvalNext {
                                expr: raw_args[0].clone(),
                                conts: vec![Cont::ArgCollect {
                                    head: list[0].clone(),
                                    done: vec![],
                                    remaining: raw_args[1..].to_vec(),
                                    env_snapshot,
                                }],
                                new_env: None,
                            })
                        }
                    }
                }
            } else {
                // Head is not a symbol — function call (compound head)
                // Evaluate head first, then args via CPS
                let raw_args: Vec<LispVal> = list[1..].to_vec();
                if raw_args.is_empty() {
                    let er = dispatch_call(list, env, state)?;
                    match er {
                        EvalResult::Value(v) => Ok(Step::Done(v)),
                        EvalResult::TailCall {
                            expr,
                            env: tail_env,
                        } => Ok(Step::EvalNext {
                            expr,
                            conts: vec![],
                            new_env: Some(tail_env),
                        }),
                    }
                } else {
                    // First eval head, then collect args
                    // For compound head: ((lambda ...) args...)
                    // We need to eval head first, then args
                    // Use ArgCollect with head as the full expression (will be eval'd in dispatch)
                    let env_snapshot = env.snapshot();
                    Ok(Step::EvalNext {
                        expr: raw_args[0].clone(),
                        conts: vec![Cont::ArgCollect {
                            head: list[0].clone(),
                            done: vec![],
                            remaining: raw_args[1..].to_vec(),
                            env_snapshot,
                        }],
                        new_env: None,
                    })
                }
            }
        }
    }
}

/// Parse a catch clause `(catch var body...)` into (var_name, body_exprs).
fn parse_catch_clause(clause: &LispVal) -> Result<(String, Vec<LispVal>), String> {
    if let LispVal::List(cl) = clause {
        if cl.is_empty() || cl[0] != LispVal::Sym("catch".into()) {
            return Err("try: second arg must be (catch var body...)".into());
        }
        let var = match cl.get(1) {
            Some(LispVal::Sym(s)) => s.clone(),
            _ => return Err("try: catch needs a variable name".into()),
        };
        Ok((var, cl[2..].to_vec()))
    } else {
        Err("try: catch clause must be a list".into())
    }
}

/// Evaluate cond clauses iteratively.
fn eval_cond_clauses(clauses: Vec<LispVal>, _env: &mut Env) -> Result<Step, String> {
    for (i, clause) in clauses.iter().enumerate() {
        if let LispVal::List(parts) = clause {
            if parts.is_empty() {
                continue;
            }
            // Check for else clause
            if let LispVal::Sym(kw) = &parts[0] {
                if kw == "else" {
                    if parts.len() > 1 {
                        let body: Vec<LispVal> = parts[1..].to_vec();
                        return Ok(Step::EvalNext {
                            expr: LispVal::List(
                                vec![LispVal::Sym("begin".into())]
                                    .into_iter()
                                    .chain(body)
                                    .collect(),
                            ),
                            conts: vec![],
                            new_env: None,
                        });
                    }
                    return Ok(Step::Done(LispVal::Nil));
                }
            }
            // Check for (test => proc) form
            let result_expr =
                if parts.len() >= 3 && matches!(&parts[1], LispVal::Sym(s) if s == "=>") {
                    Some(LispVal::List(vec![
                        parts[2].clone(),
                        LispVal::Sym("__cond_val__".into()),
                    ]))
                } else {
                    parts.get(1).cloned()
                };
            let test_expr = parts[0].clone();
            let remaining: Vec<LispVal> = clauses[i + 1..].to_vec();
            let has_arrow = parts.len() >= 3 && matches!(&parts[1], LispVal::Sym(s) if s == "=>");
            return Ok(Step::EvalNext {
                expr: test_expr,
                conts: vec![Cont::CondTest {
                    result_expr,
                    remaining,
                    is_arrow: has_arrow,
                }],
                new_env: None,
            });
        }
    }
    Ok(Step::Done(LispVal::Nil))
}

/// Evaluate let bindings iteratively.
fn eval_let(
    pairs: Vec<(String, LispVal)>,
    body_exprs: Vec<LispVal>,
    _env: &mut Env,
) -> Result<Step, String> {
    if pairs.is_empty() {
        if body_exprs.is_empty() {
            return Ok(Step::Done(LispVal::Nil));
        }
        return Ok(Step::EvalNext {
            expr: body_exprs[0].clone(),
            conts: vec![],
            new_env: None,
        });
    }
    let (name, val_expr) = &pairs[0];
    let remaining = pairs[1..].to_vec();
    let all_names: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
    Ok(Step::EvalNext {
        expr: val_expr.clone(),
        conts: vec![Cont::LetBind {
            name: name.clone(),
            remaining_pairs: remaining,
            body_exprs,
            bound_keys: all_names,
        }],
        new_env: None,
    })
}

/// Handle a continuation with the value produced by evaluating a sub-expression.
pub fn handle_cont(
    cont: Cont,
    val: LispVal,
    env: &mut Env,
    state: &mut EvalState,
) -> Result<Step, String> {
    match cont {
        Cont::IfBranch {
            then_branch,
            else_branch,
        } => {
            let next = if is_truthy(&val) {
                then_branch
            } else {
                else_branch
            };
            Ok(Step::EvalNext {
                expr: next,
                conts: vec![],
                new_env: None,
            })
        }

        Cont::CondTest {
            result_expr,
            remaining,
            is_arrow,
        } => {
            if is_truthy(&val) {
                match result_expr {
                    Some(e) => {
                        if is_arrow {
                            // For (test => proc), bind __cond_val__ and eval (proc __cond_val__)
                            env.push("__cond_val__".into(), val);
                        }
                        Ok(Step::EvalNext {
                            expr: e,
                            conts: vec![],
                            new_env: None,
                        })
                    }
                    None => Ok(Step::Done(val)),
                }
            } else if remaining.is_empty() {
                Ok(Step::Done(LispVal::Nil))
            } else {
                eval_cond_clauses(remaining, env)
            }
        }

        Cont::DefineSet { name } => {
            // If defining a compiled lambda without self_name, recompile with the name
            // so that recursive self-calls emit CallSelf instead of BuiltinCall.
            let val = match &val {
                LispVal::Lambda {
                    params,
                    rest_param,
                    body,
                    compiled: Some(_),
                    pure_type,
                    closed_env,
                    ..
                } => {
                    // Recompile with the function name for CallSelf support
                    let new_compiled = crate::bytecode::try_compile_lambda(
                        params,
                        body,
                        &closed_env
                            .read()
                            .unwrap()
                            .clone()
                            .into_iter()
                            .collect::<Vec<_>>(),
                        env,
                        Some(name.as_str()),
                    );
                    if new_compiled.is_some() {
                        LispVal::Lambda {
                            params: params.clone(),
                            rest_param: rest_param.clone(),
                            body: body.clone(),
                            compiled: new_compiled.map(|cl| Box::new(cl)),
                            pure_type: pure_type.clone(),
                            closed_env: closed_env.clone(),
                            memo_cache: None,
                        }
                    } else {
                        val
                    }
                }
                _ => val,
            };
            env.push(name.clone(), val.clone());
            env.propagate_to_scope_snapshot(&name, &val);
            Ok(Step::Done(LispVal::Nil))
        }

        Cont::DefineValues { names } => {
            // val is the result — if it's a list (from values), destructure
            let vals: Vec<LispVal> = match &val {
                LispVal::List(l) => l.clone(),
                other => vec![other.clone()],
            };
            for (name, v) in names.iter().zip(vals.iter()) {
                env.push(name.clone(), v.clone());
            }
            Ok(Step::Done(LispVal::Nil))
        }

        Cont::LetValuesBind {
            names,
            remaining_exprs,
            body_exprs,
            current_idx,
        } => {
            // Bind the current result to names[current_idx]
            let vals: Vec<LispVal> = match &val {
                LispVal::List(l) => l.clone(),
                other => vec![other.clone()],
            };
            if let Some(current_names) = names.get(current_idx) {
                for (name, v) in current_names.iter().zip(vals.iter()) {
                    env.push(name.clone(), v.clone());
                }
            }
            if remaining_exprs.is_empty() {
                // All bound — eval body
                Ok(Step::EvalNext {
                    expr: LispVal::List(
                        vec![LispVal::Sym("begin".into())]
                            .into_iter()
                            .chain(body_exprs.clone())
                            .collect(),
                    ),
                    conts: vec![],
                    new_env: None,
                })
            } else {
                // Eval next binding expression
                Ok(Step::EvalNext {
                    expr: remaining_exprs[0].clone(),
                    conts: vec![Cont::LetValuesBind {
                        names: names.clone(),
                        remaining_exprs: remaining_exprs[1..].to_vec(),
                        body_exprs: body_exprs.clone(),
                        current_idx: current_idx + 1,
                    }],
                    new_env: None,
                })
            }
        }

        Cont::SetVal { name } => {
            if let Some(slot) = env.get_mut(&name) {
                *slot = val.clone();
                env.propagate_to_shared(&name, &val);
                env.propagate_to_scope_snapshot(&name, &val);
                Ok(Step::Done(LispVal::Nil))
            } else {
                Err(format!("set!: undefined variable '{}'", name))
            }
        }

        Cont::BeginSeq { remaining } => {
            if remaining.is_empty() {
                Ok(Step::Done(val))
            } else {
                Ok(Step::EvalNext {
                    expr: remaining[0].clone(),
                    conts: if remaining.len() > 1 {
                        vec![Cont::BeginSeq {
                            remaining: remaining[1..].to_vec(),
                        }]
                    } else {
                        vec![]
                    },
                    new_env: None,
                })
            }
        }

        Cont::AndNext { remaining } => {
            if !is_truthy(&val) {
                Ok(Step::Done(val))
            } else if remaining.is_empty() {
                Ok(Step::Done(val))
            } else {
                Ok(Step::EvalNext {
                    expr: remaining[0].clone(),
                    conts: if remaining.len() > 1 {
                        vec![Cont::AndNext {
                            remaining: remaining[1..].to_vec(),
                        }]
                    } else {
                        vec![]
                    },
                    new_env: None,
                })
            }
        }

        Cont::OrNext { remaining } => {
            if is_truthy(&val) {
                Ok(Step::Done(val))
            } else if remaining.is_empty() {
                Ok(Step::Done(val))
            } else {
                Ok(Step::EvalNext {
                    expr: remaining[0].clone(),
                    conts: if remaining.len() > 1 {
                        vec![Cont::OrNext {
                            remaining: remaining[1..].to_vec(),
                        }]
                    } else {
                        vec![]
                    },
                    new_env: None,
                })
            }
        }

        Cont::NotArg => Ok(Step::Done(LispVal::Bool(!is_truthy(&val)))),

        Cont::LetBind {
            name,
            remaining_pairs,
            body_exprs,
            bound_keys,
        } => {
            let mut all_bound = bound_keys.clone();
            env.push(name.clone(), val);
            all_bound.push(name.clone());
            if remaining_pairs.is_empty() {
                // All bindings done, eval body
                if body_exprs.is_empty() {
                    return Ok(Step::Done(LispVal::Nil));
                }
                Ok(Step::EvalNext {
                    expr: body_exprs[0].clone(),
                    conts: {
                        let mut cs: Vec<Cont> = Vec::new();
                        if body_exprs.len() > 1 {
                            cs.push(Cont::BeginSeq {
                                remaining: body_exprs[1..].to_vec(),
                            });
                        }
                        cs.push(Cont::LetRestore {
                            bound_keys: all_bound,
                        });
                        cs
                    },
                    new_env: None,
                })
            } else {
                let (next_name, next_expr) = &remaining_pairs[0];
                Ok(Step::EvalNext {
                    expr: next_expr.clone(),
                    conts: vec![Cont::LetBind {
                        name: next_name.clone(),
                        remaining_pairs: remaining_pairs[1..].to_vec(),
                        body_exprs,
                        bound_keys: all_bound,
                    }],
                    new_env: None,
                })
            }
        }

        Cont::LetRestore { bound_keys } => {
            // Only remove the keys that this let introduced (don't wipe entire env)
            for key in &bound_keys {
                env.pop(key);
            }
            Ok(Step::Done(val))
        }

        Cont::MatchScrutinee { arms, .. } => {
            // val is the scrutinee
            for (_i, clause) in arms.iter().enumerate() {
                if let LispVal::List(parts) = clause {
                    if parts.len() >= 2 {
                        if let Some(bindings) = match_pattern(&parts[0], &val) {
                            let body = parts.get(1).cloned().unwrap_or(LispVal::Nil);
                            let snap = env.snapshot();
                            for (name, v) in bindings {
                                env.push(name, v);
                            }
                            return Ok(Step::EvalNext {
                                expr: body,
                                conts: vec![Cont::MatchRestore { snapshot: snap }],
                                new_env: None,
                            });
                        }
                    }
                }
            }
            Ok(Step::Done(LispVal::Nil))
        }

        Cont::MatchRestore { snapshot } => {
            env.restore(snapshot);
            Ok(Step::Done(val))
        }

        Cont::TryCatch { .. } => {
            // The expression succeeded — just pass the value through
            Ok(Step::Done(val))
        }

        Cont::LoopBind {
            mut names,
            mut vals,
            remaining,
            body,
        } => {
            vals.push(val);
            if remaining.is_empty() {
                // All bindings evaluated — start loop iteration
                let snap = env.snapshot();
                for (i, name) in names.iter().enumerate() {
                    env.push(name.clone(), vals[i].clone());
                }
                Ok(Step::EvalNext {
                    expr: body.clone(),
                    conts: vec![Cont::LoopIter {
                        binding_names: names,
                        binding_vals: vals,
                        body,
                        snapshot: snap,
                    }],
                    new_env: None,
                })
            } else {
                let (next_name, next_expr) = &remaining[0];
                names.push(next_name.clone());
                Ok(Step::EvalNext {
                    expr: next_expr.clone(),
                    conts: vec![Cont::LoopBind {
                        names,
                        vals,
                        remaining: remaining[1..].to_vec(),
                        body,
                    }],
                    new_env: None,
                })
            }
        }

        Cont::LoopIter {
            binding_names,
            binding_vals: _binding_vals,
            body,
            snapshot,
        } => {
            match val {
                LispVal::Recur(new_vals) => {
                    if new_vals.len() != binding_names.len() {
                        return Err(format!(
                            "recur: expected {} args, got {}",
                            binding_names.len(),
                            new_vals.len()
                        ));
                    }
                    // Restore env, then rebind with new values
                    env.restore(snapshot);
                    let new_snap = env.snapshot();
                    for (i, name) in binding_names.iter().enumerate() {
                        env.push(name.clone(), new_vals[i].clone());
                    }
                    Ok(Step::EvalNext {
                        expr: body.clone(),
                        conts: vec![Cont::LoopIter {
                            binding_names,
                            binding_vals: new_vals,
                            body,
                            snapshot: new_snap,
                        }],
                        new_env: None,
                    })
                }
                other => {
                    env.restore(snapshot);
                    Ok(Step::Done(other))
                }
            }
        }

        Cont::RecurArg {
            mut done,
            remaining,
        } => {
            done.push(val);
            if remaining.is_empty() {
                Ok(Step::Done(LispVal::Recur(done)))
            } else {
                Ok(Step::EvalNext {
                    expr: remaining[0].clone(),
                    conts: vec![Cont::RecurArg {
                        done,
                        remaining: remaining[1..].to_vec(),
                    }],
                    new_env: None,
                })
            }
        }

        Cont::CaseMatch { clauses } => {
            // val is the evaluated key, check each clause
            for clause in clauses {
                if let LispVal::List(parts) = clause {
                    if parts.is_empty() {
                        continue;
                    }
                    if let LispVal::Sym(s) = &parts[0] {
                        if s == "else" {
                            let body: Vec<LispVal> = parts[1..].to_vec();
                            return Ok(Step::EvalNext {
                                expr: LispVal::List(
                                    vec![LispVal::Sym("begin".into())]
                                        .into_iter()
                                        .chain(body)
                                        .collect(),
                                ),
                                conts: vec![],
                                new_env: None,
                            });
                        }
                    }
                    if let LispVal::List(datums) = &parts[0] {
                        for datum in datums {
                            if datum == &val {
                                let body: Vec<LispVal> = parts[1..].to_vec();
                                return Ok(Step::EvalNext {
                                    expr: LispVal::List(
                                        vec![LispVal::Sym("begin".into())]
                                            .into_iter()
                                            .chain(body)
                                            .collect(),
                                    ),
                                    conts: vec![],
                                    new_env: None,
                                });
                            }
                        }
                    } else if parts[0] == val {
                        let body: Vec<LispVal> = parts[1..].to_vec();
                        return Ok(Step::EvalNext {
                            expr: LispVal::List(
                                vec![LispVal::Sym("begin".into())]
                                    .into_iter()
                                    .chain(body)
                                    .collect(),
                            ),
                            conts: vec![],
                            new_env: None,
                        });
                    }
                }
            }
            Ok(Step::Done(LispVal::Nil))
        }

        Cont::ArgCollect {
            head,
            mut done,
            remaining,
            env_snapshot,
        } => {
            // val is the result of evaluating one arg
            done.push(val);
            if remaining.is_empty() {
                // All args collected — dispatch the call
                env.restore(env_snapshot);
                if let LispVal::Sym(name) = &head {
                    match crate::eval::dispatch_call_with_args(name, &done, env, state)? {
                        crate::eval::EvalResult::Value(v) => return Ok(Step::Done(v)),
                        crate::eval::EvalResult::TailCall {
                            expr,
                            env: tail_env,
                        } => {
                            return Ok(Step::EvalNext {
                                expr,
                                conts: vec![],
                                new_env: Some(tail_env),
                            })
                        }
                    }
                }
                // Non-symbol head
                let head_val = crate::eval::lisp_eval(&head, env, state)?;
                match crate::eval::call_val(&head_val, &done, env, state)? {
                    crate::eval::EvalResult::Value(v) => Ok(Step::Done(v)),
                    crate::eval::EvalResult::TailCall {
                        expr,
                        env: tail_env,
                    } => Ok(Step::EvalNext {
                        expr,
                        conts: vec![],
                        new_env: Some(tail_env),
                    }),
                }
            } else {
                // Restore env before evaluating next arg (previous arg may have modified it)
                env.restore(env_snapshot.clone());
                // Eval next arg
                Ok(Step::EvalNext {
                    expr: remaining[0].clone(),
                    conts: vec![Cont::ArgCollect {
                        head,
                        done,
                        remaining: remaining[1..].to_vec(),
                        env_snapshot,
                    }],
                    new_env: None,
                })
            }
        }

        Cont::FinalVal => {
            state
                .rlm_state
                .insert("Final".to_string(), LispVal::Bool(true));
            state.rlm_state.insert("result".to_string(), val);
            Ok(Step::Done(LispVal::Bool(true)))
        }

        Cont::AssertCheck { message } => {
            if is_truthy(&val) {
                state
                    .rlm_state
                    .insert("AssertPassed".to_string(), LispVal::Bool(true));
                Ok(Step::Done(LispVal::Bool(true)))
            } else {
                match message {
                    Some(msg) => Err(format!("assert failed: {}", msg)),
                    None => Err("assert failed".into()),
                }
            }
        }

        Cont::RlmSetVal { name } => {
            state.rlm_state.insert(name, val);
            Ok(Step::Done(LispVal::Bool(true)))
        }

        Cont::Memoize => match val {
            f @ LispVal::Lambda { .. } => Ok(Step::Done(LispVal::Memoized {
                func: Box::new(f),
                cache: Arc::new(RwLock::new(im::HashMap::new())),
            })),
            _ => Err("memoize: expected lambda".into()),
        },
    }
}

/// Walk the continuation stack looking for a TryCatch handler.
/// If found, evaluate the catch body. If not found, re-raise the error.
pub fn catch_error(
    stack: &mut Vec<Cont>,
    error: String,
    env: &mut Env,
    _state: &mut EvalState,
) -> Result<Step, String> {
    // Find the nearest TryCatch
    while let Some(cont) = stack.pop() {
        if let Cont::TryCatch {
            var,
            catch_body_exprs,
        } = cont
        {
            // Found handler — bind error var, eval catch body
            env.push(var, LispVal::Str(error));
            if catch_body_exprs.is_empty() {
                return Ok(Step::Done(LispVal::Nil));
            }
            let mut cs: Vec<Cont> = Vec::new();
            if catch_body_exprs.len() > 1 {
                cs.push(Cont::BeginSeq {
                    remaining: catch_body_exprs[1..].to_vec(),
                });
            }
            return Ok(Step::EvalNext {
                expr: catch_body_exprs[0].clone(),
                conts: cs,
                new_env: None,
            });
        }
        // Discard non-TryCatch continuations (they're in the try block we're escaping)
    }
    Err(error)
}

/// Parse a contract signature: (x :int y :str -> :ret-type)
/// Also supports grouped form: ((x :int) (y :str) -> :ret-type)
/// Returns (param_names, param_types, optional_return_type)
fn parse_contract_sig(sig: &LispVal) -> Result<(Vec<String>, Vec<RlType>, Option<RlType>), String> {
    let list = match sig {
        LispVal::List(l) => l,
        other => return Err(format!("contract: signature must be a list, got {}", other)),
    };

    // Find the -> (arrow) separator
    let arrow_pos = list
        .iter()
        .position(|v| matches!(v, LispVal::Sym(s) if s == "->" || s == "→"));

    let (param_section, ret_section) = match arrow_pos {
        Some(pos) => (&list[..pos], Some(&list[pos + 1..])),
        None => (list.as_slice(), None),
    };

    // Detect format: grouped ((x :int) (y :str)) vs flat (x :int y :str)
    let grouped = param_section
        .first()
        .map_or(false, |e| matches!(e, LispVal::List(_)));

    let mut params = Vec::new();
    let mut param_types = Vec::new();

    if grouped {
        for (i, elem) in param_section.iter().enumerate() {
            let pair = match elem {
                LispVal::List(l) => l,
                other => {
                    return Err(format!(
                        "contract: grouped param {} must be a list, got {}",
                        i + 1,
                        other
                    ))
                }
            };
            if pair.is_empty() {
                return Err(format!("contract: grouped param {} is empty", i + 1));
            }
            let name = match &pair[0] {
                LispVal::Sym(s) => s.clone(),
                other => {
                    return Err(format!(
                        "contract: param name must be symbol, got {}",
                        other
                    ))
                }
            };
            let t = if pair.len() > 1 {
                parse_type(&pair[1])?
            } else {
                RlType::Any
            };
            params.push(name);
            param_types.push(t);
        }
    } else {
        // Flat format: x :int y :str
        let mut i = 0;
        while i + 1 < param_section.len() {
            let name = match &param_section[i] {
                LispVal::Sym(s) => s.clone(),
                other => {
                    return Err(format!(
                        "contract: param name must be symbol, got {}",
                        other
                    ))
                }
            };
            let t = parse_type(&param_section[i + 1])?;
            params.push(name);
            param_types.push(t);
            i += 2;
        }
        if i < param_section.len() {
            let name = match &param_section[i] {
                LispVal::Sym(s) => s.clone(),
                other => {
                    return Err(format!(
                        "contract: param name must be symbol, got {}",
                        other
                    ))
                }
            };
            params.push(name);
            param_types.push(RlType::Any);
        }
    }

    // Parse return type
    let ret_type = match ret_section {
        Some(section) if !section.is_empty() => Some(parse_type(&section[0])?),
        _ => None,
    };

    Ok((params, param_types, ret_type))
}

/// Strip `:: type-annotation` from a define form, returning a clean list.
/// Input: [define, (f x y), ::, int, ->, int, ->, int, (body)]
/// Output: [define, (f x y), (body)]
fn strip_type_annotation(dl: &[LispVal]) -> Vec<LispVal> {
    let mut clean = vec![dl[0].clone()]; // "define"

    if dl.len() < 2 {
        return dl.to_vec();
    }

    clean.push(dl[1].clone()); // name or (name params...)

    // Find :: and skip until body
    let mut i = 2;
    while i < dl.len() {
        if let LispVal::Sym(s) = &dl[i] {
            if s == "::" {
                // Skip :: and all following type tokens until we hit the body
                // The body is the last element
                let _ = i; // assignment below is dead but we keep the loop structure
                           // Skip type tokens (everything until the last element)
                let body = dl.last().cloned().unwrap_or(LispVal::Nil);
                clean.push(body);
                return clean;
            }
        }
        // If no :: found, keep as-is
        clean.push(dl[i].clone());
        i += 1;
    }

    clean
}
