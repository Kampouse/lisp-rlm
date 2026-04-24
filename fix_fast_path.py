#!/usr/bin/env python3
"""
Fix map/filter fast path: gracefully fall back when compiled lambda
hits unknown builtins (macros, user-defined functions, etc).

Strategy: attempt the fast path. If run_compiled_lambda returns an error
on the first element, fall back to call_val for all elements.
"""

with open('/tmp/lisp-rlm/src/eval.rs', 'r') as f:
    content = f.read()

print(f"Original: {len(content)} chars, {len(content.splitlines())} lines")

# ---- Replace map fast path with fallback version ----
old_map_fast = '''            "map" => {
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
                            let mut result = Vec::with_capacity(lst.len());
                            for elem in &lst {
                                result.push(crate::bytecode::run_compiled_lambda(&cl, &[elem.clone()])?);
                            }
                            return Ok(LispVal::List(result));
                        }
                    }
                }
                // Fallback: full eval per element
                let mut result = Vec::with_capacity(lst.len());
                for elem in &lst {
                    result.push(call_val(func, &[elem.clone()], env)?);
                }
                Ok(LispVal::List(result))
            }'''

new_map_fast = '''            "map" => {
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
            }'''

assert old_map_fast in content, "Could not find old map fast path!"
content = content.replace(old_map_fast, new_map_fast, 1)

# ---- Replace filter fast path with fallback version ----
old_filter_fast = '''            "filter" => {
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
                        if let Some(cl) = crate::bytecode::try_compile_lambda(
                            params, body, closed_env, env,
                        ) {
                            let mut result = Vec::new();
                            for elem in &lst {
                                if is_truthy(&crate::bytecode::run_compiled_lambda(&cl, &[elem.clone()])?) {
                                    result.push(elem.clone());
                                }
                            }
                            return Ok(LispVal::List(result));
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
            }'''

new_filter_fast = '''            "filter" => {
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
                        if let Some(cl) = crate::bytecode::try_compile_lambda(
                            params, body, closed_env, env,
                        ) {
                            // Try first element — if bytecode can't handle it, fall back
                            if let Ok(first_result) = crate::bytecode::run_compiled_lambda(&cl, &[lst[0].clone()]) {
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
            }'''

assert old_filter_fast in content, "Could not find old filter fast path!"
content = content.replace(old_filter_fast, new_filter_fast, 1)

# Verify braces balanced
opens = content.count('{')
closes = content.count('}')
print(f"Braces: {opens} open, {closes} close")
assert opens == closes, f"Brace mismatch: {opens} vs {closes}"

with open('/tmp/lisp-rlm/src/eval.rs', 'w') as f:
    f.write(content)

print(f"Written: {len(content)} chars, {len(content.splitlines())} lines")
print("Done!")
