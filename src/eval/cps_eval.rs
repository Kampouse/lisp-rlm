// ---------------------------------------------------------------------------
// Fully iterative evaluator: eval_step + handle_cont + catch_error
// ---------------------------------------------------------------------------

use crate::helpers::{is_builtin_name, is_truthy, match_pattern, parse_params};
use crate::parser::parse_all;
use crate::types::{get_stdlib_code, Env, EvalState, LispVal};

use super::continuation::{Cont, EvalResult, Step};
use super::dispatch_types::{format_type, parse_type, RlType};
use super::{dispatch_call, expand_quasiquote, lisp_eval};

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
        | LispVal::Macro { .. }
        | LispVal::Map(_) => Ok(Step::Done(expr.clone())),

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
                                let lam = LispVal::Lambda {
                                    params,
                                    rest_param: None,
                                    body: Box::new(body),
                                    closed_env: env.get_or_create_scope_snapshot(),
                                };
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

                    // ── lambda ──
                    "lambda" => {
                        let (params, rest_param) =
                            parse_params(list.get(1).ok_or("lambda: need params")?)?;
                        let body = list.get(2).ok_or("lambda: need body")?;
                        Ok(Step::Done(LispVal::Lambda {
                            params,
                            rest_param,
                            body: Box::new(body.clone()),
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

                        let lam = LispVal::Lambda {
                            params,
                            rest_param: None,
                            body: Box::new(body_expr.clone()),
                            closed_env: env.get_or_create_scope_snapshot(),
                        };

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
                    }
                }
            } else {
                // Head is not a symbol — function call (compound head)
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
                    if let Some(body) = parts.get(1) {
                        return Ok(Step::EvalNext {
                            expr: body.clone(),
                            conts: vec![],
                            new_env: None,
                        });
                    }
                    return Ok(Step::Done(LispVal::Nil));
                }
            }
            // Evaluate test
            let test_expr = parts[0].clone();
            let result_expr = parts.get(1).cloned();
            let remaining: Vec<LispVal> = clauses[i + 1..].to_vec();
            return Ok(Step::EvalNext {
                expr: test_expr,
                conts: vec![Cont::CondTest {
                    result_expr,
                    remaining,
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
    env: &mut Env,
) -> Result<Step, String> {
    if pairs.is_empty() {
        let snap = env.snapshot();
        // Push restore cont, then eval body as begin
        if body_exprs.is_empty() {
            return Ok(Step::Done(LispVal::Nil));
        }
        return Ok(Step::EvalNext {
            expr: body_exprs[0].clone(),
            conts: {
                let mut cs: Vec<Cont> = Vec::new();
                if body_exprs.len() > 1 {
                    cs.push(Cont::BeginSeq {
                        remaining: body_exprs[1..].to_vec(),
                    });
                }
                cs.push(Cont::LetRestore { snapshot: snap });
                cs
            },
            new_env: None,
        });
    }
    let (name, val_expr) = &pairs[0];
    let remaining = pairs[1..].to_vec();
    Ok(Step::EvalNext {
        expr: val_expr.clone(),
        conts: vec![Cont::LetBind {
            name: name.clone(),
            remaining_pairs: remaining,
            body_exprs,
        }],
        new_env: None,
    })
}

/// Handle a continuation with the value produced by evaluating a sub-expression.
pub fn handle_cont(
    cont: Cont,
    val: LispVal,
    env: &mut Env,
    _state: &mut EvalState,
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
        } => {
            if is_truthy(&val) {
                match result_expr {
                    Some(e) => Ok(Step::EvalNext {
                        expr: e,
                        conts: vec![],
                        new_env: None,
                    }),
                    None => Ok(Step::Done(val)),
                }
            } else if remaining.is_empty() {
                Ok(Step::Done(LispVal::Nil))
            } else {
                eval_cond_clauses(remaining, env)
            }
        }

        Cont::DefineSet { name } => {
            env.push(name.clone(), val.clone());
            env.propagate_to_scope_snapshot(&name, &val);
            Ok(Step::Done(LispVal::Nil))
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
        } => {
            env.push(name, val);
            if remaining_pairs.is_empty() {
                // All bindings done, eval body
                if body_exprs.is_empty() {
                    return Ok(Step::Done(LispVal::Nil));
                }
                let snap = env.snapshot();
                Ok(Step::EvalNext {
                    expr: body_exprs[0].clone(),
                    conts: {
                        let mut cs: Vec<Cont> = Vec::new();
                        if body_exprs.len() > 1 {
                            cs.push(Cont::BeginSeq {
                                remaining: body_exprs[1..].to_vec(),
                            });
                        }
                        cs.push(Cont::LetRestore { snapshot: snap });
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
                    }],
                    new_env: None,
                })
            }
        }

        Cont::LetRestore { snapshot } => {
            env.restore(snapshot);
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

        Cont::FinalVal => {
            _state
                .rlm_state
                .insert("Final".to_string(), LispVal::Bool(true));
            _state.rlm_state.insert("result".to_string(), val);
            Ok(Step::Done(LispVal::Bool(true)))
        }

        Cont::AssertCheck { message } => {
            if is_truthy(&val) {
                _state
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
            _state.rlm_state.insert(name, val);
            Ok(Step::Done(LispVal::Bool(true)))
        }
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
