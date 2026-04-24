#!/usr/bin/env python3
"""
Add bytecode fast path for map/filter/reduce in eval.rs.
When the function is a single-param lambda, try to compile it to bytecode
and run the compiled version instead of call_val per element.
"""
import re

with open('/tmp/lisp-rlm/src/eval.rs', 'r') as f:
    content = f.read()

print(f"Original: {len(content)} chars, {len(content.splitlines())} lines")

# ---- Replace map handler with fast-path version ----
old_map = '''            "map" => {
                let func = args.get(0).ok_or("map: need (f list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::List(vec![])),
                    Some(other) => return Err(format!("map: expected list, got {}", other)),
                    None => return Err("map: need (f list)".into()),
                };
                let mut result = Vec::with_capacity(lst.len());
                for elem in &lst {
                    result.push(call_val(func, &[elem.clone()], env)?);
                }
                Ok(LispVal::List(result))
            }'''

new_map = '''            "map" => {
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

assert old_map in content, "Could not find old map handler!"
content = content.replace(old_map, new_map, 1)

# ---- Replace filter handler with fast-path version ----
old_filter = '''            "filter" => {
                let func = args.get(0).ok_or("filter: need (pred list)")?;
                let lst = match args.get(1) {
                    Some(LispVal::List(l)) => l.clone(),
                    Some(LispVal::Nil) => return Ok(LispVal::List(vec![])),
                    Some(other) => return Err(format!("filter: expected list, got {}", other)),
                    None => return Err("filter: need (pred list)".into()),
                };
                let mut result = Vec::new();
                for elem in &lst {
                    if is_truthy(&call_val(func, &[elem.clone()], env)?) {
                        result.push(elem.clone());
                    }
                }
                Ok(LispVal::List(result))
            }'''

new_filter = '''            "filter" => {
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

assert old_filter in content, "Could not find old filter handler!"
content = content.replace(old_filter, new_filter, 1)

# Verify braces balanced
opens = content.count('{')
closes = content.count('}')
print(f"Braces: {opens} open, {closes} close")
assert opens == closes, f"Brace mismatch: {opens} vs {closes}"

# Verify no truncation
assert '[OUTPUT TRUNCATED' not in content

with open('/tmp/lisp-rlm/src/eval.rs', 'w') as f:
    f.write(content)

print(f"Written: {len(content)} chars, {len(content.splitlines())} lines")
print("Done!")
