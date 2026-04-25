//! Collection builtins: list, car, cdr, cons, len, append, nth, range, reverse, sort, zip, empty?
//! Also includes map, filter, reduce, find, some, every (higher-order).
//!
//! par-map and par-filter propagate the LLM provider into cloned envs.

use crate::helpers::*;
use crate::types::{Env, EvalState, LispVal};
use super::continuation::EvalResult;

/// Helper: call a function value with given args in the given env.
/// Resolves any TailCall from the call chain.
fn call_val(func: &LispVal, args: &[LispVal], env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    match super::call_val(func, args, env, state)? {
        EvalResult::Value(v) => Ok(v),
        EvalResult::TailCall { expr, env: tail_env } => {
            *env = tail_env;
            super::lisp_eval(&expr, env, state)
        }
    }
}

pub fn handle(name: &str, args: &[LispVal], env: &mut Env, state: &mut EvalState) -> Result<Option<LispVal>, String> {
    match name {
        "list" => Ok(Some(LispVal::List(args.to_vec()))),
        "car" => match args.first() {
            Some(LispVal::List(l)) if !l.is_empty() => Ok(Some(l[0].clone())),
            _ => Ok(Some(LispVal::Nil)),
        },
        "cdr" => match args.first() {
            Some(LispVal::List(l)) if l.len() > 1 => Ok(Some(LispVal::List(l[1..].to_vec()))),
            Some(LispVal::List(_)) => Ok(Some(LispVal::List(vec![]))),
            _ => Ok(Some(LispVal::Nil)),
        },
        "cons" => match args.get(1) {
            Some(LispVal::List(l)) => {
                let mut n = vec![args[0].clone()];
                n.extend(l.iter().cloned());
                Ok(Some(LispVal::List(n)))
            }
            _ => Ok(Some(LispVal::List(args.to_vec()))),
        },
        "len" => match args.first() {
            Some(LispVal::List(l)) => Ok(Some(LispVal::Num(l.len() as i64))),
            Some(LispVal::Str(s)) => Ok(Some(LispVal::Num(s.chars().count() as i64))),
            Some(LispVal::Nil) => Ok(Some(LispVal::Num(0))),
            _ => Err("len: need list or string".into()),
        },
        "append" => {
            let mut r = Vec::new();
            for a in args {
                if let LispVal::List(l) = a {
                    r.extend(l.iter().cloned());
                } else {
                    r.push(a.clone());
                }
            }
            Ok(Some(LispVal::List(r)))
        }
        "nth" => {
            let list_val = args.first().ok_or("nth: need list and index")?;
            let idx_raw = as_num(args.get(1).ok_or("nth: need index")?)?;
            if idx_raw < 0 {
                return Err(format!("nth: negative index {}", idx_raw));
            }
            let i = idx_raw as usize;
            match list_val {
                LispVal::List(l) => l
                    .get(i)
                    .cloned()
                    .ok_or("nth: index out of range".into())
                    .map(Some),
                _ => Err("nth: first arg must be a list".into()),
            }
        }
        "empty?" => match args.first() {
            Some(LispVal::Nil) => Ok(Some(LispVal::Bool(true))),
            Some(LispVal::List(ref v)) if v.is_empty() => {
                Ok(Some(LispVal::Bool(true)))
            }
            Some(_) => Ok(Some(LispVal::Bool(false))),
            None => Err("empty?: need 1 argument".into()),
        },
        "range" => {
            let start = as_num(args.first().ok_or("range: need 2 args")?)?;
            let end = as_num(args.get(1).ok_or("range: need 2 args")?)?;
            if start >= end {
                return Ok(Some(LispVal::List(vec![])));
            }
            Ok(Some(LispVal::List(
                (start..end).map(LispVal::Num).collect(),
            )))
        }
        "reverse" => match args.first() {
            Some(LispVal::List(l)) => Ok(Some(LispVal::List(l.iter().rev().cloned().collect()))),
            Some(LispVal::Nil) => Ok(Some(LispVal::List(vec![]))),
            Some(other) => Err(format!("reverse: expected list, got {}", other)),
            None => Err("reverse: need 1 argument".into()),
        },
        "sort" => {
            let mut vals = match args.first() {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => vec![],
                Some(other) => return Err(format!("sort: expected list, got {}", other)),
                None => return Err("sort: need 1 argument".into()),
            };
            vals.sort_by(|a, b| {
                let fa = match a {
                    LispVal::Num(n) => *n as f64,
                    LispVal::Float(f) => *f,
                    _ => return std::cmp::Ordering::Equal,
                };
                let fb = match b {
                    LispVal::Num(n) => *n as f64,
                    LispVal::Float(f) => *f,
                    _ => return std::cmp::Ordering::Equal,
                };
                fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(Some(LispVal::List(vals)))
        }
        "zip" => {
            let a = match args.first() {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => vec![],
                Some(other) => return Err(format!("zip: expected list, got {}", other)),
                None => return Err("zip: need 2 args".into()),
            };
            let b = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => vec![],
                Some(other) => return Err(format!("zip: expected list, got {}", other)),
                None => return Err("zip: need 2 args".into()),
            };
            Ok(Some(LispVal::List(
                a.iter()
                    .zip(b.iter())
                    .map(|(x, y)| LispVal::List(vec![x.clone(), y.clone()]))
                    .collect(),
            )))
        }

        // Higher-order collection operations
        "map" => {
            let func = args.first().ok_or("map: need (f list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::List(vec![]))),
                Some(other) => return Err(format!("map: expected list, got {}", other)),
                None => return Err("map: need (f list)".into()),
            };
            // Fast path: compile single-param lambda to bytecode
            if let LispVal::Lambda {
                params,
                rest_param: None,
                body,
                closed_env,
            } = func
            {
                if params.len() == 1 {
                    if let Some(cl) =
                        crate::bytecode::try_compile_lambda(params, body, closed_env, env)
                    {
                        if lst.is_empty() {
                            return Ok(Some(LispVal::List(vec![])));
                        }
                        if let Ok(first_result) =
                            crate::bytecode::run_compiled_lambda(&cl, &[lst[0].clone()])
                        {
                            let mut result = Vec::with_capacity(lst.len());
                            result.push(first_result);
                            for elem in &lst[1..] {
                                result.push(crate::bytecode::run_compiled_lambda(
                                    &cl,
                                    &[elem.clone()],
                                )?);
                            }
                            return Ok(Some(LispVal::List(result)));
                        }
                    }
                }
            }
            let mut result = Vec::with_capacity(lst.len());
            for elem in &lst {
                result.push(call_val(func, &[elem.clone()], env, state)?);
            }
            Ok(Some(LispVal::List(result)))
        }
        "filter" => {
            let func = args.first().ok_or("filter: need (pred list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::List(vec![]))),
                Some(other) => return Err(format!("filter: expected list, got {}", other)),
                None => return Err("filter: need (pred list)".into()),
            };
            if let LispVal::Lambda {
                params,
                rest_param: None,
                body,
                closed_env,
            } = func
            {
                if params.len() == 1 {
                    if let Some(cl) =
                        crate::bytecode::try_compile_lambda(params, body, closed_env, env)
                    {
                        if lst.is_empty() {
                            return Ok(Some(LispVal::List(vec![])));
                        }
                        if let Ok(first_result) =
                            crate::bytecode::run_compiled_lambda(&cl, &[lst[0].clone()])
                        {
                            let mut result = Vec::new();
                            if is_truthy(&first_result) {
                                result.push(lst[0].clone());
                            }
                            for elem in &lst[1..] {
                                if is_truthy(&crate::bytecode::run_compiled_lambda(
                                    &cl,
                                    &[elem.clone()],
                                )?) {
                                    result.push(elem.clone());
                                }
                            }
                            return Ok(Some(LispVal::List(result)));
                        }
                    }
                }
            }
            let mut result = Vec::new();
            for (idx, elem) in lst.iter().enumerate() {
                eprintln!("[filter] elem {}/{}", idx, lst.len());
                let pred = call_val(func, &[elem.clone()], env, state)?;
                eprintln!("[filter] pred result: {}", pred);
                if is_truthy(&pred) {
                    result.push(elem.clone());
                }
            }
            Ok(Some(LispVal::List(result)))
        }
        "reduce" => {
            let func = args.first().ok_or("reduce: need (f init list)")?;
            let mut acc = args.get(1).ok_or("reduce: need (f init list)")?.clone();
            let lst = match args.get(2) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(acc)),
                Some(other) => return Err(format!("reduce: expected list, got {}", other)),
                None => return Err("reduce: need (f init list)".into()),
            };
            for (i, elem) in lst.iter().enumerate() {
                let prev_acc = acc.clone();
                acc = call_val(func, &[prev_acc.clone(), elem.clone()], env, state)?;
                // Only warn on last iteration — accumulator didn't change through entire reduce
                if i == lst.len() - 1 && acc.to_string() == prev_acc.to_string() && lst.len() > 1 {
                    eprintln!(
                        "[WARN] reduce: accumulator unchanged after full pass. \
                         Your function may be ignoring the current element."
                    );
                }
            }
            Ok(Some(acc))
        }
        "find" => {
            let func = args.first().ok_or("find: need (pred list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::Nil)),
                Some(other) => return Err(format!("find: expected list, got {}", other)),
                None => return Err("find: need (pred list)".into()),
            };
            for elem in &lst {
                if is_truthy(&call_val(func, &[elem.clone()], env, state)?) {
                    return Ok(Some(elem.clone()));
                }
            }
            Ok(Some(LispVal::Nil))
        }
        "some" => {
            let func = args.first().ok_or("some: need (pred list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::Bool(false))),
                Some(other) => return Err(format!("some: expected list, got {}", other)),
                None => return Err("some: need (pred list)".into()),
            };
            for elem in &lst {
                if is_truthy(&call_val(func, &[elem.clone()], env, state)?) {
                    return Ok(Some(LispVal::Bool(true)));
                }
            }
            Ok(Some(LispVal::Bool(false)))
        }
        "every" => {
            let func = args.first().ok_or("every: need (pred list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::Bool(true))),
                Some(other) => return Err(format!("every: expected list, got {}", other)),
                None => return Err("every: need (pred list)".into()),
            };
            for elem in &lst {
                if !is_truthy(&call_val(func, &[elem.clone()], env, state)?) {
                    return Ok(Some(LispVal::Bool(false)));
                }
            }
            Ok(Some(LispVal::Bool(true)))
        }
        "par-map" => {
            let func = args.first().ok_or("par-map: need (f list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::List(vec![]))),
                Some(other) => return Err(format!("par-map: expected list, got {}", other)),
                None => return Err("par-map: need (f list)".into()),
            };
            if lst.is_empty() {
                return Ok(Some(LispVal::List(vec![])));
            }

            use std::sync::Arc;
            let func = Arc::new(func.clone());
            let provider = state.llm_provider.as_ref().map(|p| p.box_clone());
            let rt = &crate::eval::llm_provider::SHARED_RUNTIME;

            let results: Result<Vec<LispVal>, String> = rt.block_on(async {
                let mut tasks = Vec::with_capacity(lst.len());
                for elem in lst {
                    let f = Arc::clone(&func);
                    let mut task_env = env.clone();
                    let mut task_state = crate::types::EvalState::new();
                    if let Some(ref p) = provider {
                        task_state.llm_provider = Some(p.box_clone());
                    }
                    tasks.push(tokio::spawn(async move {
                        tokio::task::yield_now().await;
                        match super::call_val(&f, &[elem], &mut task_env, &mut task_state)? {
                            EvalResult::Value(v) => Ok(v),
                            EvalResult::TailCall { expr, env: tail_env } => {
                                let mut e = tail_env;
                                super::lisp_eval(&expr, &mut e, &mut task_state)
                            }
                        }
                    }));
                }
                let mut out = Vec::with_capacity(tasks.len());
                for task in tasks {
                    out.push(
                        task.await
                            .map_err(|e| format!("par-map: task failed: {}", e))??
                    );
                }
                Ok(out)
            });
            Ok(Some(LispVal::List(results?)))
        }
        "par-filter" => {
            let func = args.first().ok_or("par-filter: need (pred list)")?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) => return Ok(Some(LispVal::List(vec![]))),
                Some(other) => return Err(format!("par-filter: expected list, got {}", other)),
                None => return Err("par-filter: need (pred list)".into()),
            };
            if lst.is_empty() {
                return Ok(Some(LispVal::List(vec![])));
            }

            use std::sync::Arc;
            let func = Arc::new(func.clone());
            let provider = state.llm_provider.as_ref().map(|p| p.box_clone());
            let rt = &crate::eval::llm_provider::SHARED_RUNTIME;

            let results: Result<Vec<LispVal>, String> = rt.block_on(async {
                let mut tasks = Vec::with_capacity(lst.len());
                for elem in &lst {
                    let f = Arc::clone(&func);
                    let mut task_env = env.clone();
                    let mut task_state = crate::types::EvalState::new();
                    if let Some(ref p) = provider {
                        task_state.llm_provider = Some(p.box_clone());
                    }
                    let e = elem.clone();
                    tasks.push(tokio::spawn(async move {
                        tokio::task::yield_now().await;
                        let result = match super::call_val(&f, &[e], &mut task_env, &mut task_state)? {
                            EvalResult::Value(v) => v,
                            EvalResult::TailCall { expr, env: tail_env } => {
                                let mut env = tail_env;
                                super::lisp_eval(&expr, &mut env, &mut task_state)?
                            }
                        };
                        Ok::<bool, String>(super::is_truthy(&result))
                    }));
                }
                let mut out = Vec::new();
                for (i, task) in tasks.into_iter().enumerate() {
                    let keep = task
                        .await
                        .map_err(|e| format!("par-filter: task failed: {}", e))??;
                    if keep {
                        out.push(lst[i].clone());
                    }
                }
                Ok(out)
            });
            Ok(Some(LispVal::List(results?)))
        }

        _ => Ok(None),
    }
}
