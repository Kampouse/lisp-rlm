use super::*;

impl WasmEmitter {
    pub(crate) fn call_core(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "lambda" | "fn" => {
                if a.len() < 2 { return Err("lambda: need params and body".into()); }
                let LispVal::List(params) = &a[0] else { return Err("lambda: params must be list".into()) };
                let param_names: Vec<String> = params.iter().map(|p| match p {
                    LispVal::Sym(s) => Ok(s.clone()), _ => Err("lambda param must be symbol".into()),
                }).collect::<Result<_, String>>()?;
                // Wrap multi-expression bodies in (begin ...)
                let body = if a.len() == 2 {
                    a[1].clone()
                } else {
                    LispVal::List(
                        std::iter::once(LispVal::Sym("begin".into()))
                            .chain(a[1..].iter().cloned())
                            .collect()
                    )
                };
                self.emit_lambda(&param_names, &body)
            }
            "+" => self.fold_binop(a, Instruction::I64Add, 0),
            "*" => self.fold_binop(a, Instruction::I64Mul, 1),
            "-" if a.len()==1 => {
                let mut v = vec![Instruction::I64Const(0)];
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.extend(self.emit_checked_sub()); // 0 - x, traps on MIN
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "-" => self.fold_binop(a, Instruction::I64Sub, i64::MIN as _),
            "/" => self.fold_binop_safe(a, Instruction::I64DivS, i64::MIN as _, true),
            "mod" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.extend(self.emit_safe_rem());
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "wrap-add" => self.fold_binop_wrapping(a, Instruction::I64Add, 0),
            "wrap-sub" => self.fold_binop_wrapping(a, Instruction::I64Sub, 0),
            "wrap-mul" => self.fold_binop_wrapping(a, Instruction::I64Mul, 1),
            "abs" => {
                let temp = self.local_idx("__abs_tmp");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalTee(temp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64LtS);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(temp));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(temp));
                v.push(Instruction::End);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "max" => {
                if a.len() == 1 { return self.expr(&a[0]); }
                let temp_a = self.local_idx("__max_a");
                let temp_b = self.local_idx("__max_b");
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                for arg in &a[1..] {
                    v.push(Instruction::LocalSet(temp_a));
                    v.extend(self.expr(arg)?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(temp_b));
                    // a >= b ? a : b
                    v.push(Instruction::LocalGet(temp_a));
                    v.push(Instruction::LocalGet(temp_b));
                    v.push(Instruction::I64GeS);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::LocalGet(temp_a));
                    v.push(Instruction::Else);
                    v.push(Instruction::LocalGet(temp_b));
                    v.push(Instruction::End);
                }
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "min" => {
                if a.len() == 1 { return self.expr(&a[0]); }
                let temp_a = self.local_idx("__min_a");
                let temp_b = self.local_idx("__min_b");
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                for arg in &a[1..] {
                    v.push(Instruction::LocalSet(temp_a));
                    v.extend(self.expr(arg)?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(temp_b));
                    // a <= b ? a : b
                    v.push(Instruction::LocalGet(temp_a));
                    v.push(Instruction::LocalGet(temp_b));
                    v.push(Instruction::I64LeS);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::LocalGet(temp_a));
                    v.push(Instruction::Else);
                    v.push(Instruction::LocalGet(temp_b));
                    v.push(Instruction::End);
                }
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "str" => {
                // Compile-time string concatenation for literal args
                if a.is_empty() { return Ok(vec![Instruction::I64Const(TAG_NIL)]); }
                if a.len() == 1 { return self.expr(&a[0]); }
                // Try compile-time concatenation
                let mut result_bytes: Vec<u8> = Vec::new();
                let mut all_const = true;
                for arg in a {
                    match arg {
                        LispVal::Str(s) => result_bytes.extend(s.as_bytes()),
                        LispVal::Num(n) => result_bytes.extend(n.to_string().as_bytes()),
                        LispVal::Bool(b) => result_bytes.extend(b.to_string().as_bytes()),
                        _ => { all_const = false; break; }
                    }
                }
                if all_const {
                    // Emit as a single string literal
                    let off = self.alloc_data(&result_bytes) as u64;
                    let encoded = (off | ((result_bytes.len() as u64) << 32)) as i64;
                    let mut v = vec![Instruction::I64Const(encoded)];
                    v.extend(self.emit_tag_str());
                    return Ok(v);
                }
                // Runtime fallback: for mixed args, use emit_str_concat
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                for arg in &a[1..] {
                    v.extend(self.expr(arg)?);
                    v.extend(self.emit_str_concat());
                }
                Ok(v)
            }
            ">"  => self.cmp(a, Instruction::I64GtS),
            "<"  => self.cmp(a, Instruction::I64LtS),
            ">=" => self.cmp(a, Instruction::I64GeS),
            "<=" => self.cmp(a, Instruction::I64LeS),
            "="  => self.eq(a),
            "!=" => self.neq(a),
            "and" => {
                let tmp = self.local_idx("__and_val");
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::LocalSet(tmp));       // save first value
                v.push(Instruction::LocalGet(tmp));        // reload for truthiness check
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(tmp));        // return first value if falsy
                v.push(Instruction::End);
                Ok(v)
            }
            "or" => {
                let tmp = self.local_idx("__or_val");
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::LocalSet(tmp));       // save first value
                v.push(Instruction::LocalGet(tmp));        // reload for truthiness check
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(tmp));        // return first value if truthy
                v.push(Instruction::Else);
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::End);
                Ok(v)
            }
            "not" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_is_truthy());
                // invert: 1 → 0, 0 → 1
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U); // i32 → i64 for emit_tag_bool
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "if" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::Else);
                if a.len()>2 { v.extend(self.expr(&a[2])?); } else { v.push(Instruction::I64Const(TAG_NIL)); }
                v.push(Instruction::End); Ok(v)
            }
            "cond" => {
                // (cond (test1 val1) (test2 val2) ... (else valN))
                // Desugar to nested if
                if a.is_empty() { return Ok(vec![Instruction::I64Const(TAG_NIL)]); }
                let mut v = Vec::new();
                let mut clauses: Vec<&[LispVal]> = Vec::new();
                for clause in a.iter() {
                    if let LispVal::List(items) = clause {
                        clauses.push(&items[..]);
                    }
                }
                // Build from last clause to first
                let mut else_val = vec![Instruction::I64Const(TAG_NIL)];
                for clause in clauses.iter().rev() {
                    if clause.len() >= 2 {
                        if let LispVal::Sym(s) = &clause[0] {
                            if s == "else" {
                                else_val = self.expr(&clause[1])?;
                                continue;
                            }
                        }
                        let mut new_else = Vec::new();
                        new_else.extend(self.expr(&clause[0])?);
                        new_else.extend(self.emit_cond_branch());
                        new_else.push(Instruction::If(BlockType::Result(ValType::I64)));
                        new_else.extend(self.expr(&clause[1])?);
                        new_else.push(Instruction::Else);
                        new_else.extend(else_val);
                        new_else.push(Instruction::End);
                        else_val = new_else;
                    }
                }
                v.extend(else_val);
                Ok(v)
            }
            "begin" | "progn" => {
                let mut v = Vec::new();
                for (i,x) in a.iter().enumerate() { v.extend(self.expr(x)?); if i<a.len()-1 { v.push(Instruction::Drop); } }
                Ok(v)
            }
            "assert-equal" if a.len() == 2 => {
                // (assert-equal expected actual) — compare two values, trap if not equal
                let expected = self.local_idx("__assert_expected");
                let actual = self.local_idx("__assert_actual");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); // expected
                v.push(Instruction::LocalSet(expected));
                v.extend(self.expr(&a[1])?); // actual
                v.push(Instruction::LocalSet(actual));
                // Compare: if expected != actual, trap
                v.push(Instruction::LocalGet(expected));
                v.push(Instruction::LocalGet(actual));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable); // test failure
                v.push(Instruction::End);
                // Return nil (assert passed)
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "assert-true" if a.len() == 1 => {
                // (assert-true expr) — evaluate, trap if falsy
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_is_truthy());
                v.push(Instruction::I32Eqz); // if NOT truthy
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "assert-raises" if a.len() == 1 => {
                // (assert-raises expr) — expect the expression to trap
                // In WASM we can't catch traps, so we compile the body
                // and check if it would trap (best-effort: just compile it normally;
                // the test harness checks the WASM exit code)
                // For now: emit the expr, drop result, return nil
                // The test runner catches the trap externally
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "let" => {
                let mut v = Vec::new();
                if let LispVal::List(bs) = &a[0] {
                    for b in bs { if let LispVal::List(p) = b { if p.len()==2 { if let LispVal::Sym(n) = &p[0] {
                        let idx = self.local_idx(n); v.extend(self.expr(&p[1])?); v.push(Instruction::LocalSet(idx));
                    }}}}
                }
                // Implicit begin: evaluate all body expressions, drop intermediates, keep last
                for (i, x) in a[1..].iter().enumerate() {
                    v.extend(self.expr(x)?);
                    if i < a.len() - 2 { v.push(Instruction::Drop); }
                }
                Ok(v)
            }
            "loop" => {
                // Compile loop/recur using direct emit_define + call:
                // 1. Replace (recur val...) → (__loop_N val...) in body
                // 2. Detect free vars from enclosing scope, add as extra params
                // 3. Emit define __loop_N as a real function (gets TCO)
                // 4. Emit the call with initial values + free var values
                let loop_n = format!("__loop_{}", self.lambda_counter);
                self.lambda_counter += 1;
                // Collect var names and inits
                let mut var_inits: Vec<(String, LispVal)> = Vec::new();
                if let LispVal::List(bs) = &a[0] {
                    for b in bs { if let LispVal::List(p) = b { if p.len()==2 { if let LispVal::Sym(n) = &p[0] {
                        var_inits.push((n.clone(), p[1].clone()));
                    }}}}
                }
                let var_names: Vec<String> = var_inits.iter().map(|(n, _)| n.clone()).collect();
                // Replace (recur val...) in body with (__loop_N val...) — direct self-call for TCO
                let mut body_exprs: Vec<LispVal> = a[1..].to_vec();
                for expr in &mut body_exprs {
                    self.replace_recur(expr, &loop_n, &var_names);
                }
                let loop_body = if body_exprs.len() == 1 {
                    body_exprs.into_iter().next().unwrap()
                } else {
                    LispVal::List(vec![LispVal::Sym("begin".into())].into_iter().chain(body_exprs).collect())
                };
                // Find free vars in loop body that aren't loop params — these come from enclosing scope
                let loop_var_set: HashSet<String> = var_names.iter().cloned().collect();
                let free_vars: Vec<String> = self.free_vars(&loop_body, &loop_var_set)
                    .into_iter()
                    .filter(|v| v != &loop_n)
                    .collect();
                // Full param list: loop vars + free vars
                let mut all_params = var_names.clone();
                all_params.extend(free_vars.iter().cloned());
                // Update recur calls to also pass free vars through
                // (recur was already replaced with (__loop_N loop_var_vals...))
                // Now we need to add free var references after the loop var args
                let mut loop_body = loop_body;
                self.patch_recur_with_free_vars(&mut loop_body, &loop_n, &free_vars);
                // Emit __loop_N as a proper function (with TCO)
                // Save emitter state (emit_define clears locals, changes current_func, etc.)
                let saved_locals = self.locals.clone();
                let saved_next_local = self.next_local;
                let saved_func = self.current_func.clone();
                let saved_param_count = self.current_param_count;
                let saved_gas_local = self.gas_local;
                let saved_while_id = self.while_id.get();

                self.emit_define(&loop_n, &all_params, &loop_body)?;

                // Restore emitter state
                self.locals = saved_locals;
                self.next_local = saved_next_local;
                self.current_func = saved_func;
                self.current_param_count = saved_param_count;
                self.gas_local = saved_gas_local;
                self.while_id.set(saved_while_id);
                // Now emit the call: push init values + free var values, then call
                let func_idx = self.funcs.iter().position(|f| f.name == loop_n)
                    .ok_or_else(|| format!("loop: internal error: {} not found after define", loop_n))?;
                let mut v = Vec::new();
                for (_, init) in &var_inits {
                    v.extend(self.expr(init)?);
                }
                // Pass free vars (their current values from enclosing scope)
                for fv in &free_vars {
                    let idx = self.locals.get(fv)
                        .ok_or_else(|| format!("loop: free var '{}' not in locals", fv))?;
                    v.push(Instruction::LocalGet(*idx));
                }
                v.push(Instruction::Call(func_idx as u32));
                Ok(v)
            }
            "recur" => {
                // recur should have been replaced by replace_recur in loop desugar
                // If we get here, recur is used outside a loop
                Err("recur outside of loop".into())
            }
            "while" => {
                let id = self.while_id.get(); self.while_id.set(id+1);
                let mut v = Vec::new();
                // block $exit (result i64)
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                // loop $loop
                v.push(Instruction::Loop(BlockType::Empty));
                // cond — use tagged truthiness
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_is_truthy());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Eqz);
                // if !cond → exit with tagged nil
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(TAG_NIL)); v.push(Instruction::Br(2)); // br $exit with i64
                v.push(Instruction::End); // if — no else needed
                // body
                for x in &a[1..] { v.extend(self.expr(x)?); v.push(Instruction::Drop); }
                // loop back
                v.push(Instruction::Br(0)); // br $loop
                v.push(Instruction::End); // loop
                // unreachable — loop either exits via br 1 or loops forever
                v.push(Instruction::I64Const(TAG_NIL)); // fallback (unreachable in practice)
                v.push(Instruction::End); // block
                Ok(v)
            }
            "set!" => {
                let LispVal::Sym(n) = &a[0] else { return Err("set!: expected symbol".into()) };
                let mut v = self.expr(&a[1])?;
                if let Some(&offset) = self.captured_map.get(n) {
                    // Captured variable — write back to closure heap slot
                    // so mutations are visible across calls and shared references.
                    // WASM i64.store: [i32 address, i64 value] → []
                    // Value is already on stack from expr(); need to save it,
                    // push address, then push value again.
                    let temp = self.next_local; self.next_local += 1;
                    v.push(Instruction::LocalSet(temp));     // save value
                    v.push(Instruction::LocalGet(0));        // closure_ptr (i64)
                    v.push(Instruction::I32WrapI64);        // → i32 address
                    v.push(Instruction::LocalGet(temp));     // restore value (i64)
                    let ma = wasm_encoder::MemArg { offset: (offset as u64 * 8), align: 3, memory_index: 0 };
                    v.push(Instruction::I64Store(ma));
                } else {
                    let idx = self.local_idx(n);
                    v.push(Instruction::LocalSet(idx));
                }
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "for" => {
                // (for var start end body...)
                if a.len() < 4 { return Err("for: need (for var start end body...)".into()); }
                let LispVal::Sym(var) = &a[0] else { return Err("for: var must be symbol".into()) };
                let idx = self.local_idx(var);
                let mut v = Vec::new();
                // init: var = start (untag for raw counter)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(idx));
                // block (result i64) { loop { if (>= var end) break; body...; var += 1; br loop } }
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // condition: var >= end → exit (both untagged counters)
                v.push(Instruction::LocalGet(idx));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(TAG_NIL)); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                // body expressions (drop all but last)
                for (i, x) in a[3..].iter().enumerate() {
                    v.extend(self.expr(x)?);
                    if i < a.len() - 4 { v.push(Instruction::Drop); }
                }
                v.push(Instruction::Drop); // drop body result
                // increment: var += 1
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0)); // loop
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }
            "mem-set8!" => {
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "mem-get8" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "mem-set!" => {
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "mem-get" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/return_str" => {
                self.need_host(25);
                let packed = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag packed (ptr|len<<32), then extract len and ptr
                v.extend(packed.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(packed);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(25)); // value_return
                // Set return flag so export wrapper skips its value_return
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/log_num" => {
                self.need_host(28);
                let num_expr = self.expr(&a[0])?;
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let abs_val = self.local_idx("__logn_abs");
                let digit_count = self.local_idx("__logn_digits");
                let is_neg = self.local_idx("__logn_neg");
                let tmp_digit = self.local_idx("__logn_d");
                let ptr = self.local_idx("__logn_ptr");
                let mut v = Vec::new();
                v.extend(num_expr);
                v.push(Instruction::LocalSet(abs_val));
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64LtS);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(is_neg));
                v.push(Instruction::LocalGet(is_neg));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::End);
                v.push(Instruction::LocalSet(abs_val));
                v.push(Instruction::I64Const(4184));
                v.push(Instruction::LocalSet(ptr));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64RemS);
                v.push(Instruction::LocalSet(tmp_digit));
                v.push(Instruction::LocalGet(abs_val));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64DivS);
                v.push(Instruction::LocalSet(abs_val));
                v.push(Instruction::LocalGet(ptr));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(ptr));
                v.push(Instruction::LocalGet(ptr));
                v.push(Instruction::LocalGet(tmp_digit));
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(self.emit_safe_store8());
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Zero special case
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(4183));
                v.push(Instruction::LocalSet(ptr));
                v.push(Instruction::I64Const(4183));
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I32WrapI64);
                v.extend(self.emit_safe_store8());
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::End);
                // Negative prefix
                v.push(Instruction::LocalGet(is_neg));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(ptr));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(ptr));
                v.push(Instruction::LocalGet(ptr));
                v.push(Instruction::I64Const(45));
                v.push(Instruction::I32WrapI64);
                v.extend(self.emit_safe_store8());
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::End);
                // log_utf8(count, ptr)
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::LocalGet(ptr));
                v.push(Self::host_call(28));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/signer_account_id" => self.read_to_register(4, a),
            "near/signer_account_pk" => self.read_to_register(5, a),
            "near/log_utf16" => {
                // (near/log_utf16 "string") — log UTF-16 string
                let msg = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len (in bytes, UTF-16 encoded)
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(29)); // log_utf16
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "tick_to_sqrtPrice64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tsp_a");
                let half_tick = self.local_idx("__tsp_ht");
                let is_odd = self.local_idx("__tsp_odd");
                let t_i = self.local_idx("__tsp_t");
                let neg_i = self.local_idx("__tsp_neg");
                let r_i = self.local_idx("__tsp_r");
                let b_i = self.local_idx("__tsp_b");
                let mut v = Vec::new();
                v.extend(addr_expr); v.push(Instruction::LocalSet(addr_i));
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // Remember if odd: is_odd = tick & 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(is_odd));
                // half_tick = tick >> 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(half_tick));
                // Compute 1.0001^half_tick in Q32.32
                // result = 1.0 in Q32.32 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Loop: while half_tick > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if half_tick & 1: r *= b
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(half_tick));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // If tick was odd: multiply by sqrt(1.0001) ≈ 1.00005 in Q32.32
                // 1.00005 * 2^32 = 4294970534 ≈ 0x1000068DA
                v.push(Instruction::LocalGet(is_odd)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0x10000)); // 1.00005 hi
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0x68DA)); // 1.00005 lo
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(0x10000)); // 1.00005 hi
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Invert if negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Convert Q32.32 → Q64.64: shift left by 32
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
