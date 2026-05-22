use super::*;

impl WasmEmitter {
    pub(crate) fn call(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
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
            // Wrapping arithmetic (never traps, always wraps — use for hashing, bit tricks)
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

            // Memory
            // ── Higher-order loop macros (expand to while loops) ──

            // (range start end) → returns start as initial counter, used with map/filter/reduce
            // Actually: (for i start end body) — like a for loop
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

            // (range-reduce init start end accumulator body)
            // accumulator is a symbol, body can reference `it` (current) and accumulator
            // (range-reduce 0 1 100 acc (+ acc it))
            "range-reduce" => {
                if a.len() < 5 { return Err("range-reduce: need (range-reduce init start end acc_var body)".into()) }
                let LispVal::Sym(acc_var) = &a[3] else { return Err("reduce: acc must be symbol".into()) };
                let acc_idx = self.local_idx(acc_var);
                let it_idx = self.local_idx("__it");
                // Both acc and it are stored TAGGED so body can read them normally.
                // The body result is untagged for accumulation, then re-tagged.
                let mut v = Vec::new();
                // acc = init (tagged)
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(acc_idx));
                // it = start (tagged)
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(it_idx));
                // while loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // untag(it) >= untag(end) → exit with acc (already tagged)
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.emit_untag());
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // acc = body (untag body result, re-tag for storage)
                v.extend(self.expr(&a[4])?);
                v.extend(self.emit_untag());
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(acc_idx));
                // it += 1 (tagged: add 8 = 1<<3 since TAG_NUM=0)
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(8)); // tagged increment
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (map-into mem_offset start end body)
            // Writes (body it) into memory at mem_offset + (it-start)*8
            // Returns count
            "map-into" => {
                if a.len() < 4 { return Err("map-into: need (map-into offset start end body)".into()) }
                let it_idx = self.local_idx("__it");
                let off_idx = self.local_idx("__off");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                // off = mem_offset (untag), it = start (untag), count = 0
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(off_idx));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // it >= end → exit
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                // return count as tagged Num
                v.push(Instruction::LocalGet(count_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[off] = body(it) — store untagged value
                v.push(Instruction::LocalGet(off_idx));
                v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[3])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // off += 8, it += 1, count += 1
                v.push(Instruction::LocalGet(off_idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(off_idx));
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (filter-count start end pred) — count items where pred(it) is truthy
            "filter-count" => {
                if a.len() < 3 { return Err("filter-count: need (filter-count start end pred)".into()) }
                let it_idx = self.local_idx("__it");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // if pred(it): count += 1 (use tagged truthiness)
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL));
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

            // NEAR host calls — capture all sub-expressions first to avoid borrow conflicts
            "near/store" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Store tagged val at mem[STORAGE_BUF] — preserves type through storage round-trip
                v.push(Instruction::I32Const(STORAGE_BUF as i32)); v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=STORAGE_BUF, register_id=0) — idx 17
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // raw >> 32 = key_len
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // raw & 0xFFFF_FFFF = key_ptr
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/load" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_read(key_len, key_ptr, register_id=1) — idx 18
                // Note: storage_read return value is unreliable in view calls (returns 0
                // even when key doesn't exist). Use register_len to check if value was written.
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1)); // register 1
                v.push(Self::host_call(18)); v.push(Instruction::Drop);

                // register_len(1) — idx 1. Returns u64 length, or -1 if register not written.
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(1));
                // Check if register_len returned -1 (key not found)
                v.push(Instruction::I64Const(-1i64 as u64 as i64));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Key not found: return 0 (tagged as Num)
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // Key found: read_register(1, STORAGE_BUF) — idx 0
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                // Load the tagged value directly — tag preserved from store
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/remove" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_remove(key_len, key_ptr, register_id=0) — idx 19
                // Untag key first
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19));
                Ok(v)
            }
            "near/has_key" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_has_key(key_len, key_ptr) — idx 20
                // Untag key first
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                // Host returns 0/1 as u64 — tag as Bool
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "near/return" => {
                let val = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32)); v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // value_return(len=8, ptr=TEMP_MEM) — idx 25
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                // Set return flag so export wrapper skips its value_return
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            // (near/return_str packed_string) — returns variable-length string bytes
            // packed = low32=ptr, high32=len. Calls value_return(len, ptr) directly.
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
            "near/log" => {
                // (near/log "string") — log string
                // (near/log "prefix" num) — log string then number (two separate log calls)
                if a.len() == 1 {
                    let msg = self.expr(&a[0])?;
                    let mut v = Vec::new();
                    // Untag string to get encoded (ptr | (len << 32))
                    v.extend(msg.clone());
                    v.extend(self.emit_untag());
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                    v.extend(msg);
                    v.extend(self.emit_untag());
                    v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
                } else {
                    // Two separate log calls: first the string, then the number
                    let msg = self.expr(&a[0])?;
                    let num_expr = self.expr(&a[1])?;
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    let abs_val = self.local_idx("__logn_abs");
                    let digit_count = self.local_idx("__logn_digits");
                    let is_neg = self.local_idx("__logn_neg");
                    let tmp_digit = self.local_idx("__logn_d");
                    let ptr = self.local_idx("__logn_ptr");
                    let mut v = Vec::new();
                    // First: log the string
                    v.extend(msg.clone());
                    v.extend(self.emit_untag());
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                    v.extend(msg);
                    v.extend(self.emit_untag());
                    v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                    v.push(Self::host_call(28));
                    // Second: log the number (same technique as near/log_num)
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
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
                }
            }
            "near/panic" => {
                let msg = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(27)); // panic_utf8(len, ptr)
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/abort" => {
                // panic() — idx 26, traps unconditionally
                Ok(vec![Self::host_call(26), Instruction::I64Const(0)])
            }
            "abort" => {
                // WASM unreachable — always traps, no env import needed
                Ok(vec![Instruction::Unreachable])
            }
            // (near/log_num expr) — converts i64 to decimal string and logs via env.log_utf8
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
            // --- storage aliases (near/storage_*) using STORAGE_BUF at offset 8192 ---
            "near/storage_set" => {
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Store untagged value at STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.extend(val_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(ma));
                // Untag key: extract len and ptr
                v.extend(key_expr.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop); // storage_write

                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/storage_get" => {
                let key_expr = self.expr(&a[0])?;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Untag key: extract len and ptr
                v.extend(key_expr.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(0)); // register 0
                v.push(Self::host_call(18)); v.push(Instruction::Drop); // storage_read — discard return
 // discard unreliable return value
                // Use register_len to check if value was written
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(-1i64 as u64 as i64));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Const(STORAGE_BUF));
                    v.push(Self::host_call(0)); // read_register
                    v.push(Instruction::I32Const(STORAGE_BUF as i32));
                    v.push(Instruction::I64Load(ma));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/storage_has" => {
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Self::host_call(20)); // storage_has_key
                Ok(v)
            }
            "near/storage_remove" => {
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19)); // storage_remove
                Ok(v)
            }
            // (hof/map (lambda (x) body) start end [offset])
            "hof/map" => {
                if a.len() < 3 { return Err("hof/map: need (hof/map (lambda (x) body) start end [offset])".into()); }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] { LispVal::Num(n) => *n as i64, _ => return Err("hof/map: offset must be number".into()) }
                } else { 2048i64 };
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let tmp = self.local_idx("__hof_tmp");
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it) — pass tagged value to lambda
                v.push(Instruction::LocalGet(it_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?); v.push(Instruction::LocalSet(tmp));
                // Store untagged result
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            // (hof/filter (lambda (x) pred) start end [offset])
            "hof/filter" => {
                if a.len() < 3 { return Err("hof/filter: need (hof/filter (lambda (x) pred) start end [offset])".into()); }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] { LispVal::Num(n) => *n as i64, _ => return Err("hof/filter: offset must be number".into()) }
                } else { 2048i64 };
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it) — pass tagged value to lambda
                v.push(Instruction::LocalGet(it_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?);
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                // Store untagged it value
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            // (hof/reduce (lambda (acc x) body) init start end)
            "hof/reduce" => {
                if a.len() < 4 { return Err("hof/reduce: need (hof/reduce (lambda (acc x) body) init start end)".into()); }
                let (params, body) = Self::extract_lambda_2param(&a[0])?;
                let acc_idx = self.local_idx(&params[0]);
                let param_idx = self.local_idx(&params[1]);
                let it_idx = self.local_idx("__hof_it");
                let mut v = Vec::new();
                // acc = init (untagged), it = start (untagged)
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(acc_idx));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[3])?); v.extend(self.emit_untag()); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it), acc = tagged(acc)
                v.push(Instruction::LocalGet(it_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::LocalSet(param_idx));
                v.push(Instruction::LocalGet(acc_idx)); v.extend(self.emit_tag_num()); v.push(Instruction::LocalSet(acc_idx));
                // body result → untag for accumulation
                v.extend(self.expr(&body)?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(acc_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/json_get_int" => {
                if a.is_empty() { return Err("near/json_get_int requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => self.json_get_int(key),
                    _ => Err("near/json_get_int key must be a string literal".into()),
                }
            }
            "near/json_get_u128" => {
                if a.len() < 2 { return Err("near/json_get_u128 requires a string key and offset argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => {
                        let offset_expr = self.expr(&a[1])?;
                        self.json_get_u128(key, offset_expr)
                    }
                    _ => Err("near/json_get_u128 key must be a string literal".into()),
                }
            }
            "near/json_get_str" => {
                if a.is_empty() { return Err("near/json_get_str requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => self.json_get_str(key),
                    _ => Err("near/json_get_str key must be a string literal".into()),
                }
            }
            "json/get" => {
                if a.is_empty() { return Err("json/get requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => self.json_get_auto(key),
                    _ => Err("json/get key must be a string literal".into()),
                }
            }
            "near/json_return_int" => {
                let val_expr = self.expr(&a[0])?;
                self.json_return_int(val_expr)
            }
            "near/json_return_str" => {
                let packed_expr = self.expr(&a[0])?;
                self.json_return_str(packed_expr)
            }
            "json-return" => {
                self.need_host(25);
                let val_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.extend(val_expr);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(1)); v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            
            "json-get" => {
                if a.is_empty() { return Err("json-get requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = if a.len() > 1 {
                            // (json-get "key" buffer) — scan the provided tagged string
                            let buf_expr = self.expr(&a[1])?;
                            let mut buf_setup = Vec::new();
                            // Untag to get payload, extract len, then extract ptr
                            buf_setup.extend(buf_expr.clone());
                            buf_setup.push(Instruction::I64Const(3)); buf_setup.push(Instruction::I64ShrU); // payload
                            buf_setup.push(Instruction::I64Const(32)); buf_setup.push(Instruction::I64ShrU); // len
                            // payload & 0xFFFFFFFF = ptr, we need buf = ptr
                            let buf_val = self.alloc_data(&[]); // dummy — we compute at runtime
                            // Actually we need to compute buf at runtime from the tagged string
                            // Setup: push len from payload >> 32, but buf needs to be ptr
                            // We'll make buf_setup push the length, and pass buf=0 as sentinel
                            // Actually let's do it differently: extract ptr and len at runtime
                            let mut setup = Vec::new();
                            setup.extend(buf_expr.clone());
                            // Untag: >> 3 to get payload
                            setup.push(Instruction::I64Const(3)); setup.push(Instruction::I64ShrU);
                            // Now payload = (len << 32) | ptr
                            // Extract len: payload >> 32
                            setup.push(Instruction::I64Const(32)); setup.push(Instruction::I64ShrU);
                            // len is now on stack — but json_get_from_buf expects (ilen) as setup
                            // We also need the ptr. Store payload in a temp, compute both.
                            let tmp = self.local_idx("__jgs_tmp");
                            let _buf_ptr = self.local_idx("__jgs_bptr");
                            setup.extend(buf_expr);
                            setup.push(Instruction::I64Const(3)); setup.push(Instruction::I64ShrU);
                            setup.push(Instruction::LocalSet(tmp));
                            // len = tmp >> 32
                            setup.push(Instruction::LocalGet(tmp));
                            setup.push(Instruction::I64Const(32)); setup.push(Instruction::I64ShrU);
                            // buf_ptr = tmp & 0xFFFFFFFF (but we need a fixed buf value for json_get_from_buf)
                            // Problem: json_get_from_buf takes a fixed buf address. The ptr is runtime.
                            // We need a version that takes buf from a local, not a constant.
                            // Quick fix: copy the string to a fixed buffer first, then scan it.
                            let _ = buf_val;
                            // Copy string to INPUT_BUF (NEAR) or STDIN_BUF (WASI), then scan
                            let target_buf = if self.wasi_mode { 32768i64 } else { INPUT_BUF };
                            let src_ptr_l = self.local_idx("__jgs_sp");
                            let copy_i = self.local_idx("__jgs_ci");
                            let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                            // src_ptr = tmp & 0xFFFFFFFF
                            setup.push(Instruction::LocalGet(tmp));
                            setup.push(Instruction::I64Const(0xFFFFFFFF)); setup.push(Instruction::I64And);
                            setup.push(Instruction::LocalSet(src_ptr_l));
                            // Copy src[i] -> target_buf[i] for i in 0..len
                            // We need len on stack first. Already pushed tmp >> 32 above.
                            // Store len to ilen local
                            let mut copy_setup = Vec::new();
                            copy_setup.push(Instruction::LocalGet(tmp));
                            copy_setup.push(Instruction::I64Const(32)); copy_setup.push(Instruction::I64ShrU);
                            // Copy loop
                            copy_setup.push(Instruction::I64Const(0)); copy_setup.push(Instruction::LocalSet(copy_i));
                            copy_setup.push(Instruction::Block(BlockType::Empty));
                            copy_setup.push(Instruction::Loop(BlockType::Empty));
                            copy_setup.push(Instruction::LocalGet(copy_i)); copy_setup.push(Instruction::LocalGet(tmp));
                            copy_setup.push(Instruction::I64Const(32)); copy_setup.push(Instruction::I64ShrU);
                            copy_setup.push(Instruction::I64GeU); copy_setup.push(Instruction::BrIf(1));
                            // target_buf[i] = src[i]
                            copy_setup.push(Instruction::I64Const(target_buf));
                            copy_setup.push(Instruction::LocalGet(copy_i)); copy_setup.push(Instruction::I64Add);
                            copy_setup.push(Instruction::I32WrapI64);
                            copy_setup.push(Instruction::LocalGet(src_ptr_l));
                            copy_setup.push(Instruction::LocalGet(copy_i)); copy_setup.push(Instruction::I64Add);
                            copy_setup.push(Instruction::I32WrapI64);
                            copy_setup.push(Instruction::I32Load8U(ma8.clone()));
                            copy_setup.push(Instruction::I32Store8(ma8.clone()));
                            copy_setup.push(Instruction::LocalGet(copy_i)); copy_setup.push(Instruction::I64Const(1));
                            copy_setup.push(Instruction::I64Add); copy_setup.push(Instruction::LocalSet(copy_i));
                            copy_setup.push(Instruction::Br(0));
                            copy_setup.push(Instruction::End); copy_setup.push(Instruction::End);
                            // Now scan from target_buf with the length
                            self.json_get_from_buf(key, "int", target_buf, &mut copy_setup)?
                        } else if self.wasi_mode {
                            self.json_get_wasi(key, "int")?
                        } else {
                            self.json_get_with_scanner(key, "int")?
                        };
                        v.extend(self.emit_tag_num());
                        Ok(v)
                    }
                    _ => Err("json-get key must be a string literal".into()),
                }
            }
            "json-get-str" => {
                if a.is_empty() { return Err("json-get-str requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = if self.wasi_mode { self.json_get_wasi(key, "str")? } else { self.json_get_with_scanner(key, "str")? };
                        v.extend(self.emit_tag_str());
                        Ok(v)
                    }
                    _ => Err("json-get-str key must be a string literal".into()),
                }
            }
            "json-get-float" => {
                if a.is_empty() { return Err("json-get-float requires a string key argument".into()); }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = if self.wasi_mode { self.json_get_wasi(key, "float")? } else { self.json_get_with_scanner(key, "float")? };
                        v.extend(self.emit_tag_num());
                        Ok(v)
                    }
                    _ => Err("json-get-float key must be a string literal".into()),
                }
            }
            "json-return" => {
                self.need_host(25);
                let val_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.extend(val_expr);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(1)); v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "borsh-serialize" => {
                // (borsh-serialize "SchemaName" field1 field2 ...)
                if a.len() < 2 { return Err("borsh-serialize requires schema name and value(s)".into()); }
                let schema_name = match &a[0] {
                    LispVal::Str(s) => s.clone(),
                    LispVal::Sym(s) => s.clone(),
                    _ => return Err("borsh-serialize: schema name must be string or symbol".into()),
                };
                self.emit_borsh_serialize(&schema_name, &a[1..])
            }
            "borsh-deserialize" => {
                // (borsh-deserialize "SchemaName" bytes-expr)
                if a.len() < 2 { return Err("borsh-deserialize requires schema name and bytes expr".into()); }
                let schema_name = match &a[0] {
                    LispVal::Str(s) => s.clone(),
                    LispVal::Sym(s) => s.clone(),
                    _ => return Err("borsh-deserialize: schema name must be string or symbol".into()),
                };
                let bytes_expr = self.expr(&a[1])?;
                self.emit_borsh_deserialize(&schema_name, bytes_expr)
            }
            "array" => {
                // (array elem0 elem1 ...) → TAG_ARRAY
                // Allocate on compile-time heap: [count, elem0, elem1, ...]
                let count = a.len() as u32;
                let slots_needed = 1 + count; // count + elements
                let ptr = self.heap_ptr;
                self.heap_ptr += slots_needed * 8;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Store count at ptr[0]
                v.push(Instruction::I64Const(ptr as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(count as i64));
                v.push(Instruction::I64Store(ma));
                // Evaluate and store each element
                for (i, elem) in a.iter().enumerate() {
                    // I64Store expects [i32 addr, i64 val] — push address first
                    v.push(Instruction::I64Const((ptr + ((i as u32 + 1) * 8)) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.expr(elem)?);
                    v.push(Instruction::I64Store(ma));
                }
                // Return tagged array ptr
                v.push(Instruction::I64Const(((ptr as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            // ── TAG_ARRAY list primitives ──
            // (vec-length arr) → tagged number (element count)
            "vec-length" => {
                if a.len() != 1 { return Err("vec-length: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__vl_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                // Untag: >> TAG_BITS → raw heap ptr
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count from ptr[0]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Tag as number
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            // (vec-nth arr idx) → element at index (tagged value)
            "vec-nth" => {
                if a.len() != 2 { return Err("vec-nth: expected 2 args".into()); }
                let arr_tmp = self.local_idx("__vn_arr");
                let idx_tmp = self.local_idx("__vn_idx");
                let count_tmp = self.local_idx("__vn_count");
                let result_tmp = self.local_idx("__vn_result");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save array
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Compile and save index (untag if tagged number)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag()); // untag the index to raw i64
                v.push(Instruction::LocalSet(idx_tmp));
                // Bounds check: idx < count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); // load count
                v.push(Instruction::LocalSet(count_tmp));
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(count_tmp));
                v.push(Instruction::I64LtU); // idx < count (unsigned)
                v.push(Instruction::If(BlockType::Empty));
                // In bounds: load element at arr + (1 + idx) * 8
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8)); // skip count slot
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(result_tmp));
                v.push(Instruction::Else);
                // Out of bounds: return nil
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::LocalSet(result_tmp));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(result_tmp));
                Ok(v)
            }
            // (vec-set! arr idx val) → void (modifies array in place, bounds checked)
            "vec-set!" => {
                if a.len() != 3 { return Err("vec-set!: expected 3 args".into()); }
                let arr_tmp = self.local_idx("__vs_arr");
                let idx_tmp = self.local_idx("__vs_idx");
                let val_tmp = self.local_idx("__vs_val");
                let count_tmp = self.local_idx("__vs_count");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save array
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Compile and save index
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(idx_tmp));
                // Compile and save value
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::LocalSet(val_tmp));
                // Bounds check: idx < count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); // load count
                v.push(Instruction::LocalSet(count_tmp));
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(count_tmp));
                v.push(Instruction::I64LtU); // idx < count (unsigned)
                v.push(Instruction::If(BlockType::Empty));
                // In bounds: store at arr_ptr + (1 + idx) * 8
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8)); // skip count slot
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64); // addr as i32
                v.push(Instruction::LocalGet(val_tmp)); // tagged value
                v.push(Instruction::I64Store(ma)); // [i32 addr, i64 val]
                v.push(Instruction::End);
                // Return nil
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            // (vec-push arr val) → new array (copy-on-push, appends val)
            "vec-push" => {
                if a.len() != 2 { return Err("vec-push: expected 2 args".into()); }
                let old_arr = self.local_idx("__vp_old");
                let new_arr = self.local_idx("__vp_new");
                let old_count = self.local_idx("__vp_oc");
                let word_idx = self.local_idx("__vp_wi");
                let val_tmp = self.local_idx("__vp_val");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save old array
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(old_arr));
                // Compile and save value to push
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(val_tmp));
                // Load old count
                v.push(Instruction::LocalGet(old_arr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); // count
                v.push(Instruction::LocalSet(old_count));
                // Allocate new array: (1 + old_count + 1) * 8 bytes
                // = (old_count + 2) * 8
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                // Stack: alloc_size → emit_runtime_alloc reads top of stack? No — it takes n_bytes as param
                // Need to compute size and pass to alloc. But emit_runtime_alloc is a fixed-size alloc.
                // For dynamic size, inline the alloc logic with overflow guard:
                let rha_tmp = self.local_idx("__vp_rha");
                let rha_new = self.local_idx("__vp_rhan");
                v.push(Instruction::LocalSet(rha_tmp)); // save alloc_size
                // Read current runtime heap ptr
                v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(new_arr)); // new_arr = old heap ptr
                // Compute new ptr
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::LocalGet(rha_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rha_new));
                // Guard: new pointer < memory limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Const(mem_limit));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                // OK: advance heap ptr
                v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::Else);
                // Overflow: trap
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // Copy loop: copy old_count + 1 words (count + all old elements)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(word_idx));
                // Block → Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // Guard: word_idx < old_count + 1
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64LtU);
                // I64LtU returns i32 — no I32WrapI64 needed
                v.push(Instruction::If(BlockType::Empty));
                // Compute dest addr: new_arr + word_idx * 8
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                // Load word from old array: old_arr + word_idx * 8
                v.push(Instruction::LocalGet(old_arr));
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Stack: [i32 dest_addr, i64 loaded_word] → I64Store
                v.push(Instruction::I64Store(ma));
                // word_idx++
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(word_idx));
                // Br(1) targets the Loop to continue
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // close If
                v.push(Instruction::End); // close Loop
                v.push(Instruction::End); // close Block
                // Write new count: new_arr[0] = old_count + 1
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Write new element: new_arr[1 + old_count] = val_tmp
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I64Const(8)); // skip count
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // Return tagged new array
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "vec?" => {
                if a.len() != 1 { return Err("vec?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7)); // tag mask
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Eq);      // i32 result
                v.push(Instruction::I64ExtendI32U); // widen to i64 for tagging
                v.extend(self.emit_tag(TAG_BOOL)); // tag the bool
                Ok(v)
            }
            "near/current_account_id" => self.read_to_register(3, a),
            "near/signer_account_id" => self.read_to_register(4, a),
            "near/predecessor_account_id" => self.read_to_register(6, a),
            "near/input" => self.read_to_register(7, a),
            "near/block_index" => { let mut v = vec![Self::host_call(8)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/block_timestamp" => { let mut v = vec![Self::host_call(9)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/epoch_height" => { let mut v = vec![Self::host_call(10)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/prepaid_gas" => { let mut v = vec![Self::host_call(15)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/used_gas" => { let mut v = vec![Self::host_call(16)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/attached_deposit" => self.read_u128_low(14),
            "near/attached_deposit_high" => self.read_u128_high(14),
            // (near/deposit-gte lo hi) → tagged bool
            // Calls attached_deposit (writes 16 bytes to TEMP_MEM), compares against compile-time u128(lo|hi<<64)
            "near/deposit-gte" => {
                let lo_val = match &a[0] {
                    LispVal::Num(n) => *n as u64,
                    _ => return Err("near/deposit-gte: lo must be a number literal".into()),
                };
                let hi_val = if a.len() > 1 {
                    match &a[1] {
                        LispVal::Num(n) => *n as u64,
                        _ => return Err("near/deposit-gte: hi must be a number literal".into()),
                    }
                } else { 0u64 };
                let mut v = Vec::new();
                // Call attached_deposit host (writes 16 bytes to TEMP_MEM)
                v.push(Instruction::I64Const(TEMP_MEM as i64));
                v.push(Self::host_call(14));
                // Compare: deposit >= threshold
                // deposit_lo = I64Load(TEMP_MEM+0), deposit_hi = I64Load(TEMP_MEM+8)
                // threshold_lo = lo_val, threshold_hi = hi_val
                // if dep_hi < threshold_hi → false
                // if dep_hi > threshold_hi → true
                // else dep_lo >= threshold_lo
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); // dep_hi
                v.push(Instruction::I64Const(hi_val as i64)); // threshold_hi
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(0)); // dep < threshold → false
                v.push(Instruction::Else);
                    v.push(Instruction::I32Const(TEMP_MEM as i32));
                    v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); // dep_hi
                    v.push(Instruction::I64Const(hi_val as i64));
                    v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::I64Const(1)); // dep > threshold → true
                    v.push(Instruction::Else);
                        // Highs equal, compare low
                        v.push(Instruction::I32Const(TEMP_MEM as i32));
                        v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); // dep_lo
                        v.push(Instruction::I64Const(lo_val as i64));
                        v.push(Instruction::I64GeU);
                        v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::End);
                v.push(Instruction::End);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/account_balance" => self.read_u128_low(12),
            "near/sha256" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(21)); // sha256
                // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/keccak256" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(22)); // keccak256
                // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ed25519_verify" => {
                // (near/ed25519_verify signature message public_key) → bool
                // All three args are byte strings (tagged Str)
                // NEAR host: ed25519_verify(sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) → u64 — idx 24
                let sig = self.expr(&a[0])?;
                let msg = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                // sig (param0, param1)
                v.extend(sig.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // sig_ptr
                // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // msg_ptr
                // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(24)); // ed25519_verify — returns u64 directly (1=valid, 0=invalid)
                // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/p256_verify" => {
                // (near/p256_verify signature message public_key) → bool
                // NEAR host: p256_verify(sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) → u64 — idx 55
                // sig: 64 bytes (r||s), msg: prehashed digest, pk: 33 bytes (compressed SEC1)
                // ⚠ Requires protocol 85+ (p256_verify_host_fn). Fails with "unknown import" on older protocols.
                eprintln!("⚠️  near/p256_verify requires protocol 85+ (p256_verify_host_fn). Will fail on older protocols.");
                let sig = self.expr(&a[0])?;
                let msg = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                // sig (param0, param1)
                v.extend(sig.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // sig_ptr
                // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // msg_ptr
                // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(55)); // p256_verify — returns u64 directly (1=valid, 0=invalid)
                // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/signer_account_pk" => self.read_to_register(5, a),
            "near/storage_usage" => { let mut v = vec![Self::host_call(11)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/account_locked_balance" => self.read_u128_low(13),
            "near/account_locked_balance_high" => self.read_u128_high(13),
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
            "near/random_seed" => self.read_to_register(23, a),

            // ── Cross-contract call primitives ──

            // (near/promise_create account_id method args gas deposit) → i64 promise_idx
            // NEAR host: promise_create(account_id_len, account_id_ptr, method_len, method_ptr, args_len, args_ptr, amount_ptr, gas) → u64 — idx 30
            // NOTE: amount is u128 passed as POINTER to memory (16 bytes LE), NOT raw i64
            // We write deposit (as low 64 bits of u128) to TEMP_MEM and pass TEMP_MEM as amount_ptr
            "near/promise_create" => {
                if a.len() != 5 { return Err("near/promise_create: need 5 args (account_id, method, args, gas, deposit)".into()); }
                let acct = self.expr(&a[0])?;
                let meth = self.expr(&a[1])?;
                let args_val = self.expr(&a[2])?;
                let gas = self.expr(&a[3])?;
                let dep = self.expr(&a[4])?;
                let mut v = Vec::new();
                // Write deposit u128 to TEMP_MEM (16 bytes: low 64 bits at offset 0, high 64 bits at offset 8)
                // First zero out the full 16 bytes
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // Write deposit low 64 bits to TEMP_MEM (addr first, then val)
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.extend(dep); v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // account_id (len, ptr)
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method (len, ptr)
                v.extend(meth.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(meth); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // args (len, ptr)
                v.extend(args_val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args_val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount_ptr (TEMP_MEM where u128 deposit was written)
                v.push(Instruction::I64Const(TEMP_MEM));
                // gas (tagged Num → untagged i64)
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(30)); // promise_create → returns u64
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (near/promise_then promise_idx account_id method args gas deposit) → i64 promise_idx
            // NEAR host: promise_then(promise_idx, account_id_len, account_id_ptr, method_len, method_ptr, args_len, args_ptr, amount_ptr, gas) → u64 — idx 31
            // NOTE: amount is u128 passed as POINTER to memory (16 bytes LE), NOT raw i64
            "near/promise_then" => {
                if a.len() != 6 { return Err("near/promise_then: need 6 args (promise_idx, account_id, method, args, gas, deposit)".into()); }
                let pidx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let meth = self.expr(&a[2])?;
                let args_val = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let dep = self.expr(&a[5])?;
                let mut v = Vec::new();
                // Write deposit u128 to TEMP_MEM
                // Stack: [addr_i32, val_i64] for i64.store
                // First zero out high 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // Zero out low 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Write deposit low 64 bits to TEMP_MEM (addr first, then val)
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.extend(dep); v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // promise_idx (untagged Num)
                v.extend(pidx); v.extend(self.emit_untag());
                // account_id (len, ptr)
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method (len, ptr)
                v.extend(meth.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(meth); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // args (len, ptr)
                v.extend(args_val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args_val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount_ptr (TEMP_MEM)
                v.push(Instruction::I64Const(TEMP_MEM));
                // gas
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(31)); // promise_then → returns u64
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (near/promise_and promise_idx_a promise_idx_b) → i64 promise_idx
            "near/promise_and" => {
                if a.len() != 2 { return Err("near/promise_and: need 2 args".into()); }
                let pa = self.expr(&a[0])?;
                let pb = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(pa); v.extend(self.emit_untag());
                v.extend(pb); v.extend(self.emit_untag());
                v.push(Self::host_call(32));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (near/promise_return promise_idx) → nil
            "near/promise_return" => {
                if a.len() != 1 { return Err("near/promise_return: need 1 arg".into()); }
                let pidx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(pidx); v.extend(self.emit_untag());
                v.push(Self::host_call(35)); // promise_return
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // (near/promise_result) → tagged Str — read result of cross-contract call in callback
            // Calls promise_result(0, 0) → write to register 0
            // Calls register_len(0) → length
            // Calls read_register(0, TEMP_MEM) → copy to memory
            "near/promise_result" => {
                self.need_host(34); self.need_host(0); self.need_host(1);
                let mut v = Vec::new();
                // promise_result(0, 0) — result_idx=0, register_id=0 → u64 (PromiseResult enum: 0=NotReady, 1=Successful, 2=Failed)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(34)); // promise_result — returns u64, drop it
                v.push(Instruction::Drop);
                // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                // register_len(0)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len
                // Pack as tagged Str: (len << 32) | TEMP_MEM
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/call target method args gas deposit) → nil — high-level cross-contract call
            // Creates promise, resolves current function's return with the promise result.
            // The caller receives the raw return value of the target contract's method.
            // NOTE: amount is u128 passed as POINTER to memory (16 bytes LE)
            "near/call" => {
                if a.len() != 5 { return Err("near/call: need 5 args (target, method, args, gas, deposit)".into()); }
                let acct = self.expr(&a[0])?;
                let meth = self.expr(&a[1])?;
                let args_val = self.expr(&a[2])?;
                let gas = self.expr(&a[3])?;
                let dep = self.expr(&a[4])?;
                let mut v = Vec::new();
                // Write deposit u128 to TEMP_MEM (zero high 64, write low 64)
                // Stack: [addr_i32, val_i64] for i64.store
                // First zero out high 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // Zero out low 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Write deposit (low 64 bits) to TEMP_MEM (addr first, then val)
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.extend(dep); v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // account_id (len, ptr)
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method (len, ptr)
                v.extend(meth.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(meth); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // args (len, ptr)
                v.extend(args_val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args_val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount_ptr (TEMP_MEM)
                v.push(Instruction::I64Const(TEMP_MEM));
                // gas (untagged Num)
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(30)); // promise_create → promise_idx on stack
                v.push(Self::host_call(35)); // promise_return(promise_idx) — forward result to caller
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // keccak512(data_str) — 64-byte digest as tagged Str
            "near/keccak512" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(52));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // ripemd160(data_str) — 20-byte digest as tagged Str
            "near/ripemd160" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(53));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/ecrecover hash sig v malleability_flag) → Num (1=success, 0=failure)
            // On success, result is in register 0 — use near/ecrecover_result to read it
            "near/ecrecover" => {
                let hash = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let v_val = self.expr(&a[2])?;
                let malleability = self.expr(&a[3])?;
                let mut vv = Vec::new();
                vv.extend(hash.clone()); vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32)); vv.push(Instruction::I64ShrU);
                vv.extend(hash); vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64); vv.push(Instruction::I64ExtendI32U);
                vv.extend(sig.clone()); vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32)); vv.push(Instruction::I64ShrU);
                vv.extend(sig); vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64); vv.push(Instruction::I64ExtendI32U);
                vv.extend(v_val);
                vv.extend(malleability);
                vv.push(Instruction::I64Const(0)); // register_id
                vv.push(Self::host_call(54));
                vv.extend(self.emit_tag_num());
                Ok(vv)
            }

            // (near/p256_verify msg sig pk) → Num (1=valid, 0=invalid)
            "near/p256_verify" => {
                let msg = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(msg.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(msg); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(sig.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(sig); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(55));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // ── Alt BN128 ──

            // (near/alt_bn128_g1_multiexp data_str) → tagged Str (result in register)
            "near/alt_bn128_g1_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(56));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/alt_bn128_g1_sum data_str) → tagged Str
            "near/alt_bn128_g1_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(57));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/alt_bn128_pairing_check data_str) → Num (1=valid, 0=invalid)
            "near/alt_bn128_pairing_check" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(58));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // ── BLS12-381 ──

            // BLS12-381 helper: call host(idx) with (data_len, data_ptr, register_id=0), read_register, return tagged Str
            // Used by functions that write result to register
            // (near/bls12381_p1_sum data_str) → tagged Str
            "near/bls12381_p1_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(59));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_p2_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(60));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_g1_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(61));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_g2_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(62));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_map_fp_to_g1" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(63));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_map_fp2_to_g2" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(64));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/bls12381_pairing_check data_str) → Num (1=valid, 0=invalid)
            "near/bls12381_pairing_check" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(65));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            "near/bls12381_p1_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(66));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            "near/bls12381_p2_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(67));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // ── Promises / Cross-contract calls ──

            // (near/promise_create account_id method args amount gas) → promise_index: i64
            // All args are packed strings except amount (i64) and gas (i64)
            "near/promise_create" => {
                // promise_create(account_id_len, account_id_ptr, method_name_len, method_name_ptr,
                //                arguments_len, arguments_ptr, amount_ptr, gas) → i64  (idx 30)
                let account = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let mut v = Vec::new();
                // account_id: untag → len >> 32, ptr & 0xFFFF_FFFF
                v.extend(account.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method_name: untag → len >> 32, ptr
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // arguments: untag → len >> 32, ptr
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount: untag, store at mem[0], pass ptr=0
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); // amount_ptr
                // gas: untag for host
                v.extend(gas);
                v.extend(self.emit_untag());
                v.push(Self::host_call(30)); // returns promise_index
                v.extend(self.emit_tag_num()); // tag return
                Ok(v)
            }

            // (near/promise_then promise_idx account_id method args amount gas) → new_promise_idx
            "near/promise_then" => {
                let pidx = self.expr(&a[0])?;
                let account = self.expr(&a[1])?;
                let method = self.expr(&a[2])?;
                let args = self.expr(&a[3])?;
                let amount = self.expr(&a[4])?;
                let gas = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(pidx); v.extend(self.emit_untag()); // untag promise idx
                v.extend(account.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0));
                v.extend(gas);
                v.push(Self::host_call(31));
                Ok(v)
            }

            // (near/promise_and promise_idx1 promise_idx2 ...) → combined_promise_idx
            "near/promise_and" => {
                // promise_and(promise_idx_ptr, promise_idx_count) → i64  (idx 32)
                // Store all promise indices at mem offset 64, then pass ptr+count
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() {
                    v.push(Instruction::I32Const((64 + i * 8) as i32));
                    v.extend(self.expr(x)?);
                    v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                }
                v.push(Instruction::I64Const(64)); // ptr
                v.push(Instruction::I64Const(a.len() as i64)); // count
                v.push(Self::host_call(32));
                Ok(v)
            }

            // (near/promise_results_count) → count: i64
            "near/promise_results_count" => {
                Ok(vec![Self::host_call(33), Instruction::I64Const(TAG_BITS), Instruction::I64Shl])
            }

            // (near/promise_result idx) → packed result string
            // promise_result(result_idx, register_id=0) → void, then read_register
            "near/promise_result" => {
                self.need_host(34); self.need_host(0); self.need_host(1);
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(34));
                // Read register to TEMP_MEM, get length, return packed
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM as i64)); v.push(Instruction::I64Or);
                Ok(v)
            }

            // (near/promise_return promise_idx) — return promise result to caller
            "near/promise_return" => {
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(35));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // ── Promise batch actions (host funcs 39-49) ──

            // (near/promise_batch_create account_ptr account_len) → promise_id
            "near/promise_batch_create" => {
                let ptr = self.expr(&a[0])?;
                let len = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(len); v.extend(ptr);
                v.push(Self::host_call(39));
                Ok(v)
            }

            // (near/promise_batch_then promise_idx account_ptr account_len) → promise_id
            "near/promise_batch_then" => {
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(len); v.extend(ptr);
                v.push(Self::host_call(40));
                Ok(v)
            }

            // (near/promise_batch_action_create_account promise_idx)
            "near/promise_batch_action_create_account" => {
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(41));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_deploy_contract promise_idx code_ptr code_len)
            "near/promise_batch_action_deploy_contract" => {
                let idx = self.expr(&a[0])?;
                let code_ptr = self.expr(&a[1])?;
                let code_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(code_len); v.extend(code_ptr);
                v.push(Self::host_call(42));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_function_call promise_idx method_ptr method_len args_ptr args_len amount_ptr gas)
            // NEAR API: (promise_index, method_name_len, method_name_ptr, arguments_len, arguments_ptr, amount_ptr, gas)
            "near/promise_batch_action_function_call" => {
                let idx = self.expr(&a[0])?;
                let method_ptr = self.expr(&a[1])?;
                let method_len = self.expr(&a[2])?;
                let args_ptr = self.expr(&a[3])?;
                let args_len = self.expr(&a[4])?;
                let amount_ptr = self.expr(&a[5])?;
                let gas = self.expr(&a[6])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(method_len); v.extend(method_ptr);
                v.extend(args_len); v.extend(args_ptr);
                v.extend(amount_ptr); v.extend(gas);
                v.push(Self::host_call(43));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_transfer promise_idx amount_ptr amount_len)
            "near/promise_batch_action_transfer" => {
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(amount_ptr); v.extend(amount_len);
                v.push(Self::host_call(44));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_stake promise_idx amount_ptr amount_len pk_ptr pk_len)
            "near/promise_batch_action_stake" => {
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let pk_ptr = self.expr(&a[3])?;
                let pk_len = self.expr(&a[4])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(amount_ptr); v.extend(amount_len);
                v.extend(pk_ptr); v.extend(pk_len);
                v.push(Self::host_call(45));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_add_key_with_full_access promise_idx pk_ptr pk_len nonce)
            "near/promise_batch_action_add_key_with_full_access" => {
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let nonce = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len); v.extend(nonce);
                v.push(Self::host_call(46));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_add_key_with_function_call promise_idx pk_ptr pk_len nonce method_ptr method_len allowance)
            "near/promise_batch_action_add_key_with_function_call" => {
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let nonce = self.expr(&a[3])?;
                let method_ptr = self.expr(&a[4])?;
                let method_len = self.expr(&a[5])?;
                let allowance = self.expr(&a[6])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len); v.extend(nonce);
                v.extend(method_ptr); v.extend(method_len); v.extend(allowance);
                v.push(Self::host_call(47));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_delete_key promise_idx pk_ptr pk_len)
            "near/promise_batch_action_delete_key" => {
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len);
                v.push(Self::host_call(48));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_delete_account promise_idx beneficiary_ptr beneficiary_len)
            "near/promise_batch_action_delete_account" => {
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(ptr); v.extend(len);
                v.push(Self::host_call(49));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // ── High-level promise wrappers (accept tagged strings) ──

            // (near/batch account_str) → promise_id
            // Creates a promise batch from a tagged string account ID
            "near/batch" => {
                if a.len() != 1 { return Err("near/batch: expected 1 arg".into()); }
                self.need_host(39);
                let account = self.expr(&a[0])?;
                let mut v = Vec::new();
                // account untagged: (len << 32) | ptr
                v.extend(account.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(account);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(39));
                Ok(v)
            }

            // (near/batch-create-account promise_id) → nil
            "near/batch-create-account" => {
                if a.len() != 1 { return Err("near/batch-create-account: expected 1 arg".into()); }
                self.need_host(41);
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(self.emit_untag());
                v.push(Self::host_call(41));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/batch-deploy promise_id code_str) → nil
            // Deploy tagged string as contract code
            "near/batch-deploy" => {
                if a.len() != 2 { return Err("near/batch-deploy: expected 2 args".into()); }
                self.need_host(42);
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // code untagged: (len << 32) | ptr
                v.extend(code.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // code_len
                v.extend(code);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // code_ptr
                v.push(Self::host_call(42));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/batch-transfer promise_id amount_ptr amount_len) → nil
            // amount_ptr and amount_len are tagged nums, untagged before host call
            "near/batch-transfer" => {
                if a.len() != 3 { return Err("near/batch-transfer: expected 3 args".into()); }
                self.need_host(44);
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(self.emit_untag());
                v.extend(amount_ptr); v.extend(self.emit_untag());
                v.extend(amount_len); v.extend(self.emit_untag());
                v.push(Self::host_call(44));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/batch-call promise_id method_str args_str amount_ptr gas) → nil
            // amount_ptr is raw pointer to 16-byte u128 LE, gas is tagged num
            // NEAR API: (promise_index, method_name_len, method_name_ptr, arguments_len, arguments_ptr, amount_ptr, gas)
            "near/batch-call" => {
                if a.len() != 5 { return Err("near/batch-call: expected 5 args".into()); }
                self.need_host(43);
                let idx = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount_ptr = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let mut v = Vec::new();
                // promise_index (untag)
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // method_name_len
                v.extend(method.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // method_len
                // method_name_ptr
                v.extend(method);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // method_ptr
                // arguments_len
                v.extend(args.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // args_len
                // arguments_ptr
                v.extend(args);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // args_ptr
                // amount_ptr (raw, untag)
                v.extend(amount_ptr); v.extend(self.emit_untag());
                // gas (untag)
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(43));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/batch-add-key promise_id pk_str nonce) → nil
            // Adds full access key with pk as tagged string and nonce as tagged num
            "near/batch-add-key" => {
                if a.len() != 3 { return Err("near/batch-add-key: expected 3 args".into()); }
                self.need_host(46);
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonce = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // pk ptr/len
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                // nonce
                v.extend(nonce); v.extend(self.emit_untag());
                v.push(Self::host_call(46));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // ── Global contracts ──

            // (near/deploy_contract code_ptr code_len) — deploys code to current account
            "near/deploy_contract" => {
                let code_ptr = self.expr(&a[0])?;
                let code_len = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Untag: extract ptr and len from tagged string
                v.extend(code_len.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // code_len
                v.extend(code_ptr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // code_ptr
                v.push(Self::host_call(50)); // deploy_contract(len, ptr)
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/current_code_hash) — returns 32-byte hash as tagged Str
            "near/current_code_hash" => self.read_to_register(51, a),

            // (near/promise_set_refund_to promise_idx account_id_str)
            "near/promise_set_refund_to" => {
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(68));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_state_init promise_idx code_str amount_u128_ptr)
            "near/promise_batch_action_state_init" => {
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(69));
                v.extend(self.emit_tag_num()); Ok(v)
            }

            // (near/promise_batch_action_state_init_by_account_id promise_idx account_id_str amount_u128_ptr)
            "near/promise_batch_action_state_init_by_account_id" => {
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(70));
                v.extend(self.emit_tag_num()); Ok(v)
            }

            // (near/set_state_init_data_entry promise_idx action_index key_str value_str)
            "near/set_state_init_data_entry" => {
                let pidx = self.expr(&a[0])?;
                let aidx = self.expr(&a[1])?;
                let key = self.expr(&a[2])?;
                let val = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(pidx); v.extend(aidx);
                v.extend(key.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(71));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/current_contract_code) — returns WASM bytecode as tagged Str
            // current_contract_code returns u64 status AND writes to register
            "near/current_contract_code" => {
                let mut v = Vec::new();
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(72));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/refund_to_account_id) — returns account ID as tagged Str
            "near/refund_to_account_id" => self.read_to_register(73, a),

            // (near/promise_batch_action_function_call_weight promise_idx method_str args_str amount gas gas_weight)
            "near/promise_batch_action_function_call_weight" => {
                let idx = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let weight = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amount); v.extend(gas); v.extend(weight);
                v.push(Self::host_call(74));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_deploy_global_contract promise_idx code_str)
            "near/promise_batch_action_deploy_global_contract" => {
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(75));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            "near/promise_batch_action_deploy_global_contract_by_account_id" => {
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(76));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_use_global_contract promise_idx code_hash_str)
            "near/promise_batch_action_use_global_contract" => {
                let idx = self.expr(&a[0])?;
                let hash = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(hash.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(hash); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(77));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            "near/promise_batch_action_use_global_contract_by_account_id" => {
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(78));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_transfer_to_gas_key promise_idx pk_str amount_ptr)
            "near/promise_batch_action_transfer_to_gas_key" => {
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(79));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_add_gas_key_with_full_access promise_idx pk_str num_nonces)
            "near/promise_batch_action_add_gas_key_with_full_access" => {
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonces = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(nonces);
                v.push(Self::host_call(80));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_batch_action_add_gas_key_with_function_call promise_idx pk_str num_nonces allowance_ptr receiver_id_str method_names_str)
            "near/promise_batch_action_add_gas_key_with_function_call" => {
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonces = self.expr(&a[2])?;
                let allow = self.expr(&a[3])?;
                let recv = self.expr(&a[4])?;
                let methods = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(nonces); v.extend(allow);
                v.extend(recv.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(recv); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(methods.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(methods); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(81));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/promise_yield_create method_str args_str gas gas_weight) → Num (promise index)
            "near/promise_yield_create" => {
                let method = self.expr(&a[0])?;
                let args = self.expr(&a[1])?;
                let gas = self.expr(&a[2])?;
                let weight = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(gas); v.extend(weight);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(82));
                v.extend(self.emit_tag_num()); Ok(v)
            }

            // (near/promise_yield_resume data_id_str payload_str) → Num (0=success)
            "near/promise_yield_resume" => {
                let data_id = self.expr(&a[0])?;
                let payload = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(data_id.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data_id); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(payload.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(payload); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(83));
                v.extend(self.emit_tag_num()); Ok(v)
            }

            // (near/validator_stake account_id_str stake_ptr) — writes stake to stake_ptr
            "near/validator_stake" => {
                let acct = self.expr(&a[0])?;
                let stake = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(stake);
                v.push(Self::host_call(84));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (near/validator_total_stake) → Num (low 128 bits)
            "near/validator_total_stake" => self.read_u128_low(85),

            // ── Iterator support ──

            // (near/iter_prefix prefix_ptr prefix_len) → iterator_id: i64
            // storage_iter_prefix writes prefix to register, then calls host(36)
            "near/iter_prefix" => {
                let prefix = self.expr(&a[0])?;
                let prefix_len = self.expr(&a[1])?;
                let mut v = Vec::new();
                // write_register(register_id=0, prefix_ptr, prefix_len)
                // Store prefix data at mem[0] first — prefix is a packed string or raw ptr+len
                // For packed string input: extract ptr and len
                // prefix is packed (low32=ptr, high32=len), prefix_len is explicit
                // Actually: prefix_ptr and prefix_len are separate args
                // Write prefix data to register: write_register(register_id=0, len=prefix_len, ptr=prefix_ptr)
                // write_register(idx 2): (register_id, data_len, data_ptr)
                v.push(Instruction::I64Const(0)); // register_id = 0
                v.extend(prefix_len.clone());
                v.extend(prefix);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr as i64
                // Swap to get (register_id, data_ptr, data_len) — nope, write_register is (register_id, data_len, data_ptr)
                // Actually HOST_FUNCS[2] = write_register: (I64, I64, I64) = (register_id, data_len, data_ptr)
                // We pushed: reg_id=0, prefix_len, prefix_ptr. That's correct order.
                v.push(Self::host_call(2)); // write_register — returns void, no drop
                // storage_iter_prefix(prefix_len, register_id=0) — idx 36
                // But wait: HOST_FUNCS[36] = storage_iter_prefix: (I64, I64) = (prefix_len, register_id)
                // We need to pass the length again and register_id
                v.extend(prefix_len.clone());
                v.push(Instruction::I64Const(0)); // register_id = 0
                v.push(Self::host_call(36));
                Ok(v)
            }

            // (near/iter_range start_ptr start_len end_ptr end_len) → iterator_id: i64
            "near/iter_range" => {
                let start = self.expr(&a[0])?;
                let start_len = self.expr(&a[1])?;
                let end = self.expr(&a[2])?;
                let end_len = self.expr(&a[3])?;
                let mut v = Vec::new();
                // Write start to register 0
                v.push(Instruction::I64Const(0)); // register_id
                v.extend(start_len.clone());
                v.extend(start); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2)); // write_register — void
                // Write end to register 1
                v.push(Instruction::I64Const(1)); // register_id
                v.extend(end_len.clone());
                v.extend(end); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2)); // write_register — void
                // storage_iter_range(start_len, register_id=0, end_len, register_id=1) — idx 37
                v.extend(start_len);
                v.push(Instruction::I64Const(0));
                v.extend(end_len);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(37));
                Ok(v)
            }

            // (near/iter_next iter_id key_ptr val_ptr) → i64 (1 if found, 0 if done)
            "near/iter_next" => {
                let iter_id = self.expr(&a[0])?;
                let key_ptr = self.expr(&a[1])?;
                let val_ptr = self.expr(&a[2])?;
                let mut v = Vec::new();
                // storage_iter_next(iter_id, key_register_id, value_register_id) — idx 38
                v.extend(iter_id);
                v.extend(key_ptr);
                v.extend(val_ptr);
                v.push(Self::host_call(38));
                Ok(v)
            }

            // ── u128 Arithmetic (two i64s: low at addr, high at addr+8) ──
            // Scratch area: 128-191 (4 i64 slots at offsets 128,136,144,152)

            // (u128/store addr low high) — store u128 at addr
            "u128/store" => {
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                // store low at addr
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // store high at addr+8
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (u128/load addr) → low 64 bits
            "u128/load" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (u128/load_high addr) → high 64 bits
            "u128/load_high" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (u128/add dst_addr src_addr) — dst += src
            // Uses scratch at 128: dst_lo_result, 136: dst_hi, 144: carry
            "u128/add" => {
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128a");
                let src_i = self.local_idx("__u128b");
                let lo_i = self.local_idx("__u128lo");
                let hi_i = self.local_idx("__u128hi");
                let c_i = self.local_idx("__u128c");
                let mut v = Vec::new();
                // Save addresses
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load dst_low, src_low
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // lo_i = dst_low + src_low
                v.push(Instruction::LocalGet(lo_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(lo_i));
                // Carry: if result < src_low (unsigned), carry=1
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // hi = dst_high + src_high + carry
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(hi_i));
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (u128/sub dst_addr src_addr) — dst -= src
            "u128/sub" => {
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128sa");
                let src_i = self.local_idx("__u128sb");
                let lo_i = self.local_idx("__u128slo");
                let hi_i = self.local_idx("__u128shi");
                let b_i = self.local_idx("__u128borrow");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load dst_low into lo_i
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                // Load src_low into b_i
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(b_i));
                // borrow = lo_i < b_i (unsigned)
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(b_i));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // lo_i = lo_i - src_low
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(lo_i));
                // hi = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(hi_i));
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (u128/mul dst_addr val_i64) — dst *= val_i64 (unsigned)
            // Simplified: dst_lo * val, dst_hi = dst_hi * val + (dst_lo * val) >> 64
            // Uses scratch 128-159: stores intermediate (low_result at 128, high_result at 136)
            // We use i64.mul for low part and handle overflow via comparison
            "u128/mul" => {
                let dst = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128ma");
                let val_i = self.local_idx("__u128mv");
                let dl_i = self.local_idx("__u128mdl");
                let dh_i = self.local_idx("__u128mdh");
                let rl_i = self.local_idx("__u128mrl");
                let rh_i = self.local_idx("__u128mrh");
                let t_i = self.local_idx("__u128mt");
                let carry_i = self.local_idx("__u128mc");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // Load dst_lo, dst_hi
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dl_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dh_i));
                // rl = dl * val (i64.mul, wraps on overflow)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));
                // carry from low mul: if rl < dl (assuming val >= 2, but edge cases...)
                // Better: split dl into high32 and low32, multiply separately
                // Simpler approach: carry = (dl * val) >> 64 ≈ (dl >> 32) * val + ...
                // Approximation using (dl >> 32) * (val) + ((dl & 0xFFFFFFFF) * (val >> 32))
                // This gives the high 64 bits of the 128-bit product of low halves
                // carry = (dl >> 32) * val (shifted left 0, but this is 96-bit...)
                // Actually: carry = ((dl >> 32) * val) + (((dl & 0xFFFFFFFF) * val) >> 32)
                // But we need >>64 not >>32. Let's do:
                // carry = (dl >> 32) * (val >> 32) is wrong too.
                // Correct approach for full carry:
                // carry = dl_hi * val_lo + dl_lo * val_hi + (dl_lo * val_lo >> 64)
                // But we can't easily get >> 64 of a 64x64->128 mul in WASM i64.
                //
                // PRAGMATIC: For DeFi amounts, values are typically < 2^53 (exact i64).
                // We use: carry = (dl != 0 && val != 0 && rl < dl) as rough carry estimate
                // This is WRONG for large values. Let me use the split approach properly.
                //
                // Split: dl = (dl_hi << 32) | dl_lo where dl_hi = dl >> 32, dl_lo = dl & 0xFFFF_FFFF
                // full_lo = dl_lo * val_lo  (fits in 64 bits since both < 2^32)
                // mid1 = dl_hi * val_lo
                // mid2 = dl_lo * val_hi
                // rl = full_lo + ((mid1 + mid2) << 32)   — but this can overflow too
                //
                // SIMPLEST CORRECT: Use the comparison trick.
                // If dl != 0 and val != 0 and rl / dl != val, there was overflow.
                // But division is expensive and can trap.
                //
                // Let me just do: carry = 0 for now, and document that mul is correct
                // only when the product of the low halves fits in 64 bits.
                // For NEAR FT amounts (u128 low part usually < 2^60), multiplying by
                // prices < 2^20, this is fine.
                //
                // Actually the simplest correct approach for full 64x64->128:
                // We can't do it with just i64 ops without splitting into 32-bit halves.
                // Let's do the 32-bit split:

                // dl_hi = dl >> 32, dl_lo = dl & 0xFFFFFFFF
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(t_i)); // t = dl_hi

                // carry = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(carry_i));

                // rl = dl_lo * val_lo (both < 2^32, product < 2^64)
                // rl = (dl & 0xFFFF_FFFF) * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));

                // carry += (dl_lo * val_lo) >> 32
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += dl_hi * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(t_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += (dl & 0xFFFF_FFFF) * (val >> 32)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // rl &= 0xFFFF_FFFF (keep only low 32 bits)
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(rl_i));

                // Now carry has bits [32..95] of the 128-bit low product
                // rh = dh * val + carry + (dl_hi * (val >> 32) shifted)
                // Actually carry already accumulated everything above bit 32.
                // rh = dh * val + carry
                v.push(Instruction::LocalGet(dh_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rh_i));

                // Store results
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (u128/lt addr1 addr2) → i64 (0 or 1)
            "u128/lt" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128lt1");
                let a2_i = self.local_idx("__u128lt2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // Compare high first: if a1_hi < a2_hi → 1; if a1_hi > a2_hi → 0; else compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // a1_hi < a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // Check a1_hi > a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64GtU); // a1_hi > a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // Highs equal, compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End);
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/eq addr1 addr2) → i64 (0 or 1)
            "u128/eq" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128eq1");
                let a2_i = self.local_idx("__u128eq2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // high_eq = a1_hi == a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                // I64Eq returns i32, which If consumes directly
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // low_eq = a1_lo == a2_lo
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/is_zero addr) → i64
            "u128/is_zero" => {
                let mut v = self.expr(&a[0])?;
                let addr_i = self.local_idx("__u128zz");
                v.push(Instruction::LocalSet(addr_i));
                // low == 0 && high == 0
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/from_yocto "amount" offset) — compile-time parse, store hi:lo, return offset
            "u128/from_yocto" => {
                if a.len() != 2 { return Err("u128/from_yocto: expected (\"amount\" offset)".into()); }
                let offset_expr = self.expr(&a[1])?;
                let (lo, hi) = match &a[0] {
                    LispVal::Str(s) => Self::parse_u128(s)?,
                    _ => return Err("u128/from_yocto: first arg must be a string literal".into()),
                };
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(lo));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            "u128/new" => {
                if a.len() != 3 { return Err("u128/new: expected (hi lo offset)".into()); }
                let hi_e = self.expr(&a[0])?;
                let lo_e = self.expr(&a[1])?;
                let off_e = self.expr(&a[2])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(lo_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(hi_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            "u128/from_i64" => {
                if a.len() != 2 { return Err("u128/from_i64: expected (n offset)".into()); }
                let n_e = self.expr(&a[0])?;
                let off_e = self.expr(&a[1])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(n_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            "u128/to_i64" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            "u128/store_storage" => {
                if a.len() != 2 { return Err("u128/store_storage: expected (\"key\" src)".into()); }
                let key = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let os = self.local_idx("__u128_s");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(src); v.push(Instruction::LocalSet(os));
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Store(ma));
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Const(STORAGE_U128_BUF)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            "u128/load_storage" => {
                if a.len() != 2 { return Err("u128/load_storage: expected (\"key\" dst)".into()); }
                let key = self.expr(&a[0])?;
                let dst = self.expr(&a[1])?;
                let od = self.local_idx("__u128_d");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(od));
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(STORAGE_U128_BUF));
                v.push(Self::host_call(0)); v.push(Instruction::Drop);
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(od));
                Ok(v)
            }

            // ── CLMM / Uniswap V3 Primitives ──
            // All use 64.64 fixed-point (Q64.64): value = raw >> 64, raw = value << 64
            // Price stored as sqrtPriceX96 equivalent: Q64.96

            // (fp/mul a b) → i64 — Q32.32 multiply: (a * b) >> 32
            // 64x32 multiply: split a into hi/lo 16-bit parts vs 32-bit b
            // a*b = (a_hi * b)<<16 | a_lo*b, then >> 32
            "fp/mul" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpm_a");
                let b_i = self.local_idx("__fpm_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                // result = (a >> 16) * (b >> 16) + ((a & 0xFFFF) * b) >> 32
                // For Q32.32: just use (a * b) >> 32
                // a*b won't overflow if a < 2^48 and b < 2^16... but they can be larger
                // Safe method: (a >> 16) * b doesn't overflow if a < 2^48 and b < 2^16
                // For our use: a and b are Q32.32, max ~2^32 each, so a>>16 is ~2^16, *b ~2^48 fine
                // But for larger values, need full split:
                // result = ((a >> 16) * (b >> 16)) + (((a >> 16) * (b & 0xFFFF)) >> 16) + (((a & 0xFFFF) * (b >> 16)) >> 16)
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // a_hi * b_hi
                // + (a_hi * b_lo) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // + (a_lo * b_hi) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // a_lo * b_lo is negligible after >> 32 for Q32.32 precision
                Ok(v)
            }

            // (fp/div a b) → i64 — Q32.32 divide: (a << 32) / b
            "fp/div" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpd_a");
                let b_i = self.local_idx("__fpd_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                Ok(v)
            }

            // (fp/to_int x) → i64 — Q32.32 → integer: x >> 32
            "fp/to_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // (fp/from_int x) → i64 — integer → Q32.32: x << 32
            "fp/from_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                Ok(v)
            }

            // (fp/one) → i64 — 1.0 in Q32.32 = 1 << 32
            "fp/one" => {
                Ok(vec![Instruction::I64Const(1), Instruction::I64Const(32), Instruction::I64Shl])
            }

            // ── Q64.64 fixed-point (NEAR standard, dual-i64 in memory) ──
            // Layout: mem[addr] = low 64 bits, mem[addr+8] = high 64 bits
            // Value = (high << 64 | low) / 2^64 = high + low/2^64

            // (fp64/set_int addr val) — store integer as Q64.64
            "fp64/set_int" => {
                let addr = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // mem[addr] = 0 (low)
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // mem[addr+8] = val (high)
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (fp64/get_int addr) → i64 — integer part = mem[addr+8]
            "fp64/get_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (fp64/get_frac addr) → i64 — fractional part = mem[addr]
            "fp64/get_frac" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (fp64/set addr lo hi) — store raw Q64.64 parts
            "fp64/set" => {
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (fp64/add dst_addr src_addr) — dst += src (both Q64.64 in memory)
            "fp64/add" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64_dl");
                let dh = self.local_idx("__fp64_dh");
                let sl = self.local_idx("__fp64_sl");
                let sh = self.local_idx("__fp64_sh");
                let carry = self.local_idx("__fp64_c");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // dst_low += src_low, detect carry
                v.push(Instruction::LocalGet(sl)); v.push(Instruction::LocalGet(dl)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dl));
                // carry = 1 if dl < sl (overflow)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high += src_high + carry
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::LocalGet(dh)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (fp64/mul dst_addr src_addr) — dst *= src (Q64.64, full 128-bit multiply via 32-bit splits)
            // result = (a * b) >> 64, where a={dl,dh}, b={sl,sh}
            // Uses 32-bit splits for each 64x64 multiply to get full 128-bit precision
            "fp64/mul" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fm_dl");
                let dh = self.local_idx("__fm_dh");
                let sl = self.local_idx("__fm_sl");
                let sh = self.local_idx("__fm_sh");
                // temps for 32-bit split multiply: mulh(x,y) → hi64(x*y)
                let x_lo = self.local_idx("__fm_xlo");
                let x_hi = self.local_idx("__fm_xhi");
                let y_lo = self.local_idx("__fm_ylo");
                let y_hi = self.local_idx("__fm_yhi");
                let ll = self.local_idx("__fm_ll");
                let lh = self.local_idx("__fm_lh");
                let hl = self.local_idx("__fm_hl");
                let hh = self.local_idx("__fm_hh");
                let mid = self.local_idx("__fm_mid");
                let mc = self.local_idx("__fm_mc");
                let _lo = self.local_idx("__fm_lo");
                let lc = self.local_idx("__fm_lc");
                let _hi = self.local_idx("__fm_hi");
                // Cross-term storage
                let cross1_lo = self.local_idx("__fm_c1l");
                let cross1_hi = self.local_idx("__fm_c1h");
                let cross2_lo = self.local_idx("__fm_c2l");
                let cross2_hi = self.local_idx("__fm_c2h");
                let albl_hi = self.local_idx("__fm_abh");
                let rl = self.local_idx("__fm_rl");
                let rh = self.local_idx("__fm_rh");
                let tmp = self.local_idx("__fm_tmp");
                let tmp2 = self.local_idx("__fm_tmp2");
                let mut v = Vec::new();

                // Helper macro-like: emit code to compute hi=high64(x*y), lo=low64(x*y)
                // Stack should have x, y when called. Uses x_lo,x_hi,y_lo,y_hi,ll,lh,hl,hh,mid,mc,lo,lc,hi
                // After: hi and lo locals are set. Nothing on stack.
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi: u32, lo: u32| {
                    // x_lo = x & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(x_lo));
                    // x_hi = x >> 32
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(x_hi));
                    // y_lo = y & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(y_lo));
                    // y_hi = y >> 32
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(y_hi));
                    // ll = x_lo * y_lo
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(ll));
                    // lh = x_lo * y_hi
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(lh));
                    // hl = x_hi * y_lo
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hl));
                    // hh = x_hi * y_hi
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hh));
                    // mid = lh + hl, mid_carry = mid < lh
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::LocalGet(hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(mid));
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(mc));
                    // lo = ll + (mid << 32), lo_carry = lo < ll
                    v.push(Instruction::LocalGet(ll));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(lo));
                    v.push(Instruction::LocalGet(ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(lc));
                    // hi = hh + (mid >> 32) + (mc << 32) + lc
                    v.push(Instruction::LocalGet(hh));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi));
                    // lo result
                };

                // Load dst {dl, dh}
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // Load src {sl, sh}
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));

                // Step 1: Compute high64(dl*sl) → albl_hi (we only need high part)
                emit_mul128(&mut v, dl, sl, albl_hi, tmp);

                // Step 2: Compute full 128-bit ah*bl → {cross1_lo, cross1_hi}
                emit_mul128(&mut v, dh, sl, cross1_hi, cross1_lo);

                // Step 3: Compute full 128-bit al*bh → {cross2_lo, cross2_hi}
                emit_mul128(&mut v, dl, sh, cross2_hi, cross2_lo);

                // Step 4: cross = cross1 + cross2 (128-bit add)
                // cross_lo = cross1_lo + cross2_lo, carry_a
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::LocalGet(cross2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2)); // tmp2 = carry_a
                // tmp = cross_lo
                // cross_hi = cross1_hi + cross2_hi + carry_a
                v.push(Instruction::LocalGet(cross1_hi)); v.push(Instruction::LocalGet(cross2_hi)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(mid));
                // mid = cross_hi, tmp = cross_lo

                // Step 5: result_lo = cross_lo + albl_hi (may carry)
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(albl_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(rl));
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp)); // tmp = carry_b

                // Step 6: result_hi = dh*sh + cross_hi + carry_b
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rh));

                // Store result to dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (fp64/lt addr1 addr2) → i64 — compare Q64.64 values
            "fp64/lt" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let h1 = self.local_idx("__fplt_h1");
                let h2 = self.local_idx("__fplt_h2");
                let mut v = Vec::new();
                // Compare high parts first
                v.extend(a1.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h1));
                v.extend(a2.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h2));
                // if h1 < h2: return 1
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // if h1 > h2: return 0
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // High equal, compare low
                v.extend(a1); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(a2); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // returns i32
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End); // inner if
                v.push(Instruction::End); // outer if
                Ok(v)
            }

            // (fp64/is_zero addr) → i64
            "fp64/is_zero" => {
                let addr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // high == 0?
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (fp64/sub dst_addr src_addr) — dst -= src (Q64.64, subtract with borrow)
            "fp64/sub" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64s_dl");
                let dh = self.local_idx("__fp64s_dh");
                let sl = self.local_idx("__fp64s_sl");
                let sh = self.local_idx("__fp64s_sh");
                let borrow = self.local_idx("__fp64s_b");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // borrow = dl < sl (unsigned)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(borrow));
                // dst_low -= src_low
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dl));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(borrow)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (fp64/div dst_addr src_addr) — dst /= src (Q64.64, Newton reciprocal + full-precision mul)
            // a/b = a * (1/b), compute reciprocal via Newton, then multiply
            "fp64/div" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fpd_d");
                let src_i = self.local_idx("__fpd_s");
                let ah = self.local_idx("__fpd_ah");
                let al = self.local_idx("__fpd_al");
                let bh = self.local_idx("__fpd_bh");
                let bl = self.local_idx("__fpd_bl");
                // Newton state: x_lo, x_hi (reciprocal estimate)
                let x_lo = self.local_idx("__fpd_xl");
                let x_hi = self.local_idx("__fpd_xh");
                // Temp for b*x
                let tx_lo = self.local_idx("__fpd_txl");
                let tx_hi = self.local_idx("__fpd_txh");
                // Temp for correction = 2.0 - b*x
                let cl = self.local_idx("__fpd_cl");
                let ch = self.local_idx("__fpd_ch");
                // mul128 temps (shared with mul)
                let m_xlo = self.local_idx("__fm_xlo");
                let m_xhi = self.local_idx("__fm_xhi");
                let m_ylo = self.local_idx("__fm_ylo");
                let m_yhi = self.local_idx("__fm_yhi");
                let m_ll = self.local_idx("__fm_ll");
                let m_lh = self.local_idx("__fm_lh");
                let m_hl = self.local_idx("__fm_hl");
                let m_hh = self.local_idx("__fm_hh");
                let m_mid = self.local_idx("__fm_mid");
                let m_mc = self.local_idx("__fm_mc");
                let m_lo = self.local_idx("__fm_lo");
                let m_lc = self.local_idx("__fm_lc");
                let _m_hi = self.local_idx("__fm_hi");
                // Cross-term temps for mul
                let c1_lo = self.local_idx("__fpd_c1l");
                let c1_hi = self.local_idx("__fpd_c1h");
                let c2_lo = self.local_idx("__fpd_c2l");
                let c2_hi = self.local_idx("__fpd_c2h");
                let ab_hi = self.local_idx("__fpd_abh");
                let rl = self.local_idx("__fpd_rl");
                let rh = self.local_idx("__fpd_rh");
                let tmp = self.local_idx("__fpd_tmp");
                let tmp2 = self.local_idx("__fpd_tmp2");
                let mut v = Vec::new();

                // emit_mul128: computes hi=high64(x*y), lo=low64(x*y)
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi_dst: u32, lo_dst: u32| {
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_xlo));
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_xhi));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_ylo));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_yhi));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_ll));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_lh));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hl));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hh));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::LocalGet(m_hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_mid));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_mc));
                    v.push(Instruction::LocalGet(m_ll));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_lo));
                    v.push(Instruction::LocalGet(m_ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_lc));
                    v.push(Instruction::LocalGet(m_hh));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi_dst));
                    v.push(Instruction::LocalGet(m_lo)); v.push(Instruction::LocalSet(lo_dst));
                };

                // emit_fp64_mul: full Q64.64 multiply of {a_lo,a_hi} * {b_lo,b_hi} → {dst_lo,dst_hi}
                let emit_fp64_mul = |v: &mut Vec<Instruction<'static>>, a_lo: u32, a_hi: u32, b_lo: u32, b_hi: u32, dst_lo: u32, dst_hi: u32| {
                    // high64(a_lo * b_lo) → ab_hi (don't need low)
                    emit_mul128(v, a_lo, b_lo, ab_hi, tmp);
                    // full 128: a_hi * b_lo → {c1_lo, c1_hi}
                    emit_mul128(v, a_hi, b_lo, c1_hi, c1_lo);
                    // full 128: a_lo * b_hi → {c2_lo, c2_hi}
                    emit_mul128(v, a_lo, b_hi, c2_hi, c2_lo);
                    // cross = c1 + c2 (128-bit add)
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::LocalGet(c2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2));
                    v.push(Instruction::LocalGet(c1_hi)); v.push(Instruction::LocalGet(c2_hi)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(m_mid));
                    // result_lo = cross_lo + ab_hi
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(ab_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(dst_lo));
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // result_hi = a_hi*b_hi + cross_hi + carry
                    v.push(Instruction::LocalGet(a_hi)); v.push(Instruction::LocalGet(b_hi)); v.push(Instruction::I64Mul);
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_hi));
                };

                v.extend(da); v.push(Instruction::LocalSet(dst_i));
                v.extend(sa); v.push(Instruction::LocalSet(src_i));
                // Load a = dst (numerator)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(al));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ah));
                // Load b = src (denominator)
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bh));

                // Initial reciprocal estimate: x0 ≈ 1/b in Q64.64
                // For Q64.64 value b = bh + bl/2^64, 1/b ≈ 2^64/bh (for bh > 0)
                // As Q64.64: 1/b ≈ {2^64/bh, 0} if 1/b < 1, or {0, 2^64/bh} if 1/b >= 1
                // Since 2^64 doesn't fit in i64, use (2^64-1)/bh as approximation
                // If bh == 1: x0 = {0, 1} (exact reciprocal ≈ 1.0)
                // If bh >= 2: x0 = {(2^64-1)/bh, 0} (reciprocal < 1.0, stored in low word)
                // If bh == 0: b < 1.0, 1/b > 1.0. x0 = {0, (2^64-1)/bl}
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 0: reciprocal > 1.0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 1
                // Check if bh == 1
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 1: x0 = {0, 1} (≈ 1.0)
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 2: x0 = {(2^64-1)/bh, 0}
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::End); // bh == 1
                v.push(Instruction::End); // bh == 0

                // Newton iterations: x = x * (2 - b*x), 3 iterations
                for _ in 0..3 {
                    // t = b * x (Q64.64 multiply)
                    emit_fp64_mul(&mut v, bl, bh, x_lo, x_hi, tx_lo, tx_hi);
                    // correction = 2.0 - t (Q64.64 subtraction)
                    // cl = 0 - tx_lo (with borrow)
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(tx_lo)); v.push(Instruction::I64Sub); v.push(Instruction::LocalTee(cl));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtU); // borrow if cl wrapped (cl > 0 when it should be 0-tx_lo)
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // ch = 2 - tx_hi - borrow
                    v.push(Instruction::I64Const(2));
                    v.push(Instruction::LocalGet(tx_hi)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(ch));
                    // x = x * correction (Q64.64 multiply)
                    emit_fp64_mul(&mut v, x_lo, x_hi, cl, ch, x_lo, x_hi);
                }

                // Final: result = a * x (Q64.64 multiply)
                emit_fp64_mul(&mut v, al, ah, x_lo, x_hi, rl, rh);

                // Store result to dst
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }


            // ── fp64/sqrt: Q64.64 square root via 128-bit Newton's method ──
            // (fp64/sqrt dst src) — reads Q64.64 from src, writes sqrt(src) to dst
            // Computes isqrt(V) for V = vh*2^64+vl (128-bit), then stores as Q64.64
            "fp64/sqrt" => {
                // Q64.64 Newton: r = (r + V/r) / 2, iterated
                // Work directly in Q64.64 with {rl, rh} as the estimate
                // V/r approximated with high-word division (Newton is self-correcting)
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fsqrt_d");
                let src_i = self.local_idx("__fsqrt_s");
                let vh = self.local_idx("__fsqrt_vh");
                let vl = self.local_idx("__fsqrt_vl");
                let rh = self.local_idx("__fsqrt_rh");
                let rl = self.local_idx("__fsqrt_rl");
                let _prev_rh = self.local_idx("__fsqrt_prh");
                let qh = self.local_idx("__fsqrt_qh");
                let ql = self.local_idx("__fsqrt_ql");
                let sum_l = self.local_idx("__fsqrt_sl");
                let sum_h = self.local_idx("__fsqrt_sh");
                let tmp = self.local_idx("__fsqrt_tmp");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load Q64.64 value V = {vl, vh}
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vh));

                // Handle V == 0
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Eqz);
                v.push(Instruction::I32And);
                v.push(Instruction::If(BlockType::Empty));
                // V == 0: result = 0
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // Initial guess: r = isqrt(vh) as Q64.64 {0, isqrt(vh)}
                // Use 64-bit Newton to compute isqrt(vh)
                let r64 = self.local_idx("__fsqrt_r64");
                let p64 = self.local_idx("__fsqrt_p64");
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vh));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);

                // Handle isqrt(vh) == 0 (vh was 0 or 1)
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // r64 == 0: do isqrt(vl) instead, result = isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vl));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Store isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // r64 > 0: initial Q64.64 guess r = {0, r64}
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rl));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(rh));

                // Q64.64 Newton: r = (r + V/r) / 2, 6 iterations
                // V/r uses high-word division with refinement: q_hi = vh/rh, q_lo estimated
                for _ in 0..6 {
                    // V/r: simplified Q64.64 division
                    // If rh == 0: q = {0xFFFFFFFFFFFFFFFF / max(rl,1), 0} (rough)
                    // Else: q_hi = vh / rh, q_lo from remainder refinement
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    // rh == 0: rough estimate
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::Else);
                    v.push(Instruction::I64Const(-1)); v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::End);
                    v.push(Instruction::Else);
                    // rh > 0: q_hi = vh / rh, remainder for q_lo refinement
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(qh));
                    // remainder_hi = vh % rh
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64RemU);
                    v.push(Instruction::LocalSet(tmp));
                    // q_lo ≈ (remainder_hi << 32 + (vl >> 32)) / rh << 32 ... simplified:
                    // q_lo ≈ (remainder_hi * 2^64) / rh, but use 64-bit approx:
                    // q_lo = (remainder_hi << 32 | vl >> 32) / rh ... but this might overflow
                    // Simpler: q_lo = ((tmp << 32) + (vl >> 32)) / rh
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::End);

                    // sum = r + q (Q64.64 add with carry)
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::LocalGet(ql)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(sum_l));
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::LocalGet(qh)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(sum_h));

                    // r = sum >> 1 (Q64.64 right shift by 1)
                    // new_rl = (sum_l >> 1) | (sum_h << 63)
                    // new_rh = sum_h >> 1
                    v.push(Instruction::LocalGet(sum_l)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(63)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or); v.push(Instruction::LocalSet(rl));
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(rh));
                }

                // Store result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // r64 == 0
                v.push(Instruction::End); // V == 0
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // ── tick_to_price64: Q64.64 via Q32.32 + shift ──
            // (tick_to_price64 addr tick) — writes Q64.64 1.0001^tick to mem[addr..addr+15]
            // Uses proven Q32.32 binary exponentiation, then shifts left by 32 for Q64.64
            "tick_to_price64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tp64_a");
                let t_i = self.local_idx("__tp64_t");
                let neg_i = self.local_idx("__tp64_neg");
                let r_i = self.local_idx("__tp64_r");
                let b_i = self.local_idx("__tp64_b");
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
                // result = 1.0 in Q32.32 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Binary exponentiation loop (same proven Q32.32 mul with 16-bit split)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // Q32.32 mul: r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
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
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Convert Q32.32 → Q64.64: shift left by 32
                // Store lo = (r << 32) at addr
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Store hi = (r >> 32) at addr+8
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }


            // ── tick_to_sqrtPrice64: sqrt(1.0001^tick) in Q64.64 ──
            // (tick_to_sqrtPrice64 addr tick) — writes Q64.64 sqrtPrice to mem[addr]
            // sqrtPrice = sqrt(1.0001^tick) = 1.0001^(tick/2)
            // Uses Q32.32 binary exponentiation with tick/2, then shifts to Q64.64
            // This avoids the full price → sqrt pipeline and gives better precision
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

            // (sqrt x) → i64 — integer square root via Newton's method
            // For CLMM: use on price values. Returns floor(sqrt(x))
            "sqrt" => {
                let x = self.expr(&a[0])?;
                let x_i = self.local_idx("__sq_x");
                let r_i = self.local_idx("__sq_r");
                let prev_i = self.local_idx("__sq_p");
                let mut v = Vec::new();
                v.extend(x); v.push(Instruction::LocalSet(x_i));
                // if x == 0: return 0
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::I64Eqz); // → i32
                v.push(Instruction::I32Eqz); // invert: x != 0 → enter then branch
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Initial guess: x >> 1 (rough sqrt)
                // Better: r = 1 << ((64 - clz(x)) / 2)
                // Simple: r = x, iterate r = (r + x/r) / 2
                v.push(Instruction::LocalGet(x_i)); v.push(Instruction::LocalSet(r_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::LocalSet(prev_i));
                // r = (r + x/r) / 2
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::I64DivU); // x / r
                v.push(Instruction::I64Add); // r + x/r
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); // / 2
                v.push(Instruction::LocalSet(r_i));
                // if r >= prev: converged, break
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(prev_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2)); // exit outer block
                v.push(Instruction::End);
                v.push(Instruction::Br(0)); // loop
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(r_i)); // return prev (last decreasing value)
                // Actually if r >= prev, prev is the answer (converged from above)
                // But we want the one that stopped decreasing
                v.pop(); // remove the LocalGet r_i
                v.push(Instruction::LocalGet(prev_i)); // prev was last decreasing
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0)); // x == 0 case
                v.push(Instruction::End);
                Ok(v)
            }

            // (fp/sqrt x) → i64 — Q64.64 square root: sqrt(x << 64) >> 32 = sqrt(x) << 32
            // Returns Q64.64 fixed-point sqrt
            "fp/sqrt" => {
                // sqrt(Q64.64) = sqrt(x * 2^64) = sqrt(x) * 2^32
                // = sqrt(x) << 32 in Q64.64
                // Use integer sqrt of (x >> 32) then << 48... 
                // Actually: want sqrt(x) where x is Q64.64
                // = isqrt(x) if x were the full number
                // Since x = (real_val) << 64, sqrt(x) = sqrt(real_val) << 32
                // = isqrt(x >> 64) << 32 ... no
                // Better: isqrt(x >> 32) << 16 ... losing precision
                // Best: isqrt(x) then shift. x is Q64.64 so ~64 bits of fraction
                // isqrt(x) gives sqrt with ~32 bits of fraction implicitly
                // But x can be up to 128 bits. Use two-part method:
                // Split x = hi << 64 | lo
                // sqrt = sqrt(hi) << 32 + adjustment
                // For CLMM: we mostly sqrt prices ~1-1000, so hi is small
                // Just use: (sqrt(x >> 32)) << 16 as approximation? No.
                // Correct approach: isqrt(x) where x is treated as uint128
                // We can do: r = isqrt(high * 2^64 + low)
                // ≈ isqrt(high) << 32 + low / (2 * isqrt(high) << 32)
                // For simplicity: (sqrt (x >> 32)) << 16 gives OK precision for CLMM
                // Actually the correct Q64.64 sqrt: 
                //   result = isqrt(x) where we need 128-bit isqrt
                //   Split: a = x >> 64, b = x & ((1<<64)-1)
                //   r = isqrt(a) << 32
                //   remainder = a - r^2 (in high bits)  
                //   r = (r << 64 + b) correction via Newton
                // Simplest correct: compute integer sqrt of (x >> 32), then << 16
                // This gives Q64.32 result, need to shift to Q64.64: << 32 more = << 48
                // NO. Let me think again.
                // Q64.64 value V represents real number v = V / 2^64
                // We want sqrt(v) * 2^64 = sqrt(V/2^64) * 2^64 = sqrt(V) * 2^32
                // So: fp/sqrt(x) = isqrt(x) * 2^32 ... but isqrt of a Q64.64 number
                // that's at most ~2^127 gives result ~2^63, then * 2^32 overflows
                // 
                // Better: fp/sqrt(x) = isqrt(x) >> 0, since isqrt(Q64.64) already has
                // the right scale? No.
                //
                // Simplest: fp/sqrt(x) = isqrt(x) for Q64.64 input
                // If x = 1.0 = 2^64, isqrt(2^64) = 2^32 = 0.5 in Q64.64... wrong
                // We want sqrt(1.0) = 1.0 = 2^64
                // So: fp/sqrt(x) = isqrt(x << 64) ... but that overflows
                //
                // Practical CLMM: use Q64 for sqrt price, separate from Q64.64
                // (fp/sqrt x) = (sqrt x) gives integer sqrt, caller manages scaling
                // Just delegate to integer sqrt
                // User does: (fp/from_int (sqrt (fp/to_int price_approx)))
                // Or: (sqrt x) << 32 for Q64.64 sqrt of integer
                // I'll just delegate — fp/sqrt is an alias for careful scaling
                let inner = &LispVal::List(vec![
                    LispVal::Sym("sqrt".into()), a[0].clone()
                ]);
                // After sqrt, shift left by 32 to get Q64.64 result from Q64.0 input
                // Wait — sqrt of Q64.64 = isqrt(x) which gives wrong scale
                // For Q64.64 input x representing value X: x = X * 2^64
                // sqrt(x) in same format = sqrt(X) * 2^64 = sqrt(X * 2^64) * 2^32
                // = isqrt(x) * 2^32
                let mut v = self.expr(inner)?;
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                Ok(v)
            }

            // (clz x) → i64 — count leading zeros (for tick bitmap)
            // WASM doesn't have clz for i64, use i64.clz
            "clz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Clz);
                Ok(v)
            }

            // (ctz x) → i64 — count trailing zeros
            "ctz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Ctz);
                Ok(v)
            }

            // (popcnt x) → i64 — population count (for tick bitmap)
            "popcnt" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Popcnt);
                Ok(v)
            }

            // (bit_get x idx) → i64 — get bit at index (0 or 1)
            "bit_get" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.extend(idx);
                v.push(Instruction::I64ShrU); // x >> idx
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And); // & 1
                Ok(v)
            }

            // (bit_set x idx) → i64 — set bit at index
            "bit_set" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Or); // x | (1 << idx)
                Ok(v)
            }

            // (bit_clr x idx) → i64 — clear bit at index
            "bit_clr" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Const(-1i64)); // all ones
                v.push(Instruction::I64Xor); // ~(1 << idx)
                v.push(Instruction::I64And); // x & ~(1 << idx)
                Ok(v)
            }

            // (tick_to_price tick) → i64 — 1.0001^tick in Q64.64
            // Uses binary exponentiation with Q64.64 multiply
            // 1.0001^tick = exp(tick * ln(1.0001))
            // ln(1.0001) ≈ 0.000099995 in Q64.64 ≈ 0x29C3E3
            // For small ticks (|tick| < 887272), iterative multiply works
            // We do: result = 1.0; for each bit of tick, square base; if bit set, multiply
            "tick_to_price" => {
                // Binary exponentiation: 1.0001^tick in Q32.32
                let tick = self.expr(&a[0])?;
                let t_i = self.local_idx("__ttp_t");
                let r_i = self.local_idx("__ttp_r");
                let b_i = self.local_idx("__ttp_b");
                let neg_i = self.local_idx("__ttp_neg");
                let _c_i = self.local_idx("__ttp_c");
                let mut v = Vec::new();
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // result = 1.0 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Loop: while tick > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b (Q32.32 mul with 16-bit split)
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // r_hi * b_hi
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End); // if
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
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative: r = (1<<48) / r << ... actually just (1<<32) * (1<<16) / r
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // 1/r ≈ (1 << 48) / r, then >> 16 to get back to Q32.32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(r_i));
                Ok(v)
            }

            // (price_to_tick price_q64) → i64 — inverse of tick_to_price (approximate)
            // tick = log(price) / log(1.0001)
            // log(1.0001) ≈ 0.000099995 ≈ Q64.64: 0x29C3E3
            // log(price) via binary log: find msb, iterate
            // For CLMM: usually price is from tick_to_price, so exact inverse via lookup
            // Approximation: tick ≈ (price_q64 - 1<<64) * 10000 (first-order Taylor)
            "price_to_tick" => {
                let p = self.expr(&a[0])?;
                let p_i = self.local_idx("__ptp_p");
                let mut v = Vec::new();
                v.extend(p); v.push(Instruction::LocalSet(p_i));
                // First order: tick ≈ (p - 1.0) / log(1.0001) ≈ (p - 1<<64) * 10000
                // More precisely: (p - (1<<64)) >> 64 * 10000 gives integer approximation
                v.push(Instruction::LocalGet(p_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(64)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU); // to integer
                v.push(Instruction::I64Const(10000));
                v.push(Instruction::I64Mul);
                Ok(v)
            }

            // (liquidity_amount0 sqrt_price_a sqrt_price_b liquidity) → Q64.64
            // amount0 = L * (1/sqrtPa - 1/sqrtPb) for Pa < Pb
            // = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
            "liq_amount0" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la0_a"); let spb_i = self.local_idx("__la0_b"); let liq_i = self.local_idx("__la0_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // numerator = liq * (spb - spa) — Q64.64 mul
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(liq_i)); // reuse as numerator
                // denominator = spa * spb — Q64.64 mul
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                // result = numerator / denominator
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64DivU);
                Ok(v)
            }

            // (liquidity_amount1 sqrt_price_a sqrt_price_b liquidity) → Q64.64
            // amount1 = L * (sqrtPb - sqrtPa)
            "liq_amount1" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la1_a"); let spb_i = self.local_idx("__la1_b"); let liq_i = self.local_idx("__la1_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // liq * (spb - spa) — Q64.64 multiply
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // ── String Operations (packed: low32=ptr, high32=len) ──

            // ── Q64.64 Memory-based CLMM operations ──

            // (liq_amount0_64 dst spa_addr spb_addr liq_addr)
            // amount0 = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
            // All Q64.64 in memory. Writes Q64.64 result to dst.
            "liq_amount0_64" => {
                // (liq_amount0_64 dst spa_addr spb_addr liq_addr)
                // amount0 = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
                // All Q64.64 memory. Uses high-word arithmetic for CLMM (prices ≈ 1.0)
                let dst = self.expr(&a[0])?;
                let spa_a = self.expr(&a[1])?;
                let spb_a = self.expr(&a[2])?;
                let liq_a = self.expr(&a[3])?;
                let dst_i = self.local_idx("__la0_d");
                let spa_lo = self.local_idx("__la0_sl");
                let spa_hi = self.local_idx("__la0_sh");
                let spb_lo = self.local_idx("__la0_bl");
                let spb_hi = self.local_idx("__la0_bh");
                let liq_hi = self.local_idx("__la0_lh");
                let diff_lo = self.local_idx("__la0_dl");
                let diff_hi = self.local_idx("__la0_dh");
                let num_hi = self.local_idx("__la0_nh");
                let den_hi = self.local_idx("__la0_dnh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                // Load all values upfront into locals
                v.extend(spa_a); v.push(Instruction::LocalSet(spa_lo)); // spa addr
                v.extend(spb_a); v.push(Instruction::LocalSet(spb_lo)); // spb addr
                v.extend(liq_a); v.push(Instruction::LocalSet(liq_hi)); // liq addr
                // Load spa Q64.64
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(diff_lo)); // spa low temporarily
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                v.push(Instruction::LocalGet(diff_lo)); v.push(Instruction::LocalSet(spa_lo)); // proper spa_lo
                // Load spb Q64.64
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_lo)); // spb low
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64); // need spb addr for hi
                // Wait, spb_lo is now the spb value, not addr. Need separate addr local.
                // Let me restructure with addr locals
                v.clear();
                // Redo with proper addr locals
                let dst2 = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la0_as");
                let addr_spb = self.local_idx("__la0_ab");
                let addr_liq = self.local_idx("__la0_al");
                v.extend(dst2); v.push(Instruction::LocalSet(dst_i));
                // Store addresses in locals
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load spa
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                // Load spb
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_hi));
                // Load liq
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_hi));
                // diff_hi = spb_hi - spa_hi
                v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff_hi));
                // numerator = liq_hi * diff_hi
                v.push(Instruction::LocalGet(liq_hi)); v.push(Instruction::LocalGet(diff_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(num_hi));
                // denominator = spa_hi * spb_hi (both ≈ 1, so ≈ 1)
                v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(den_hi));
                // result = numerator / denominator
                v.push(Instruction::LocalGet(num_hi)); v.push(Instruction::LocalGet(den_hi)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(num_hi));
                // Store: lo=0, hi=result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(num_hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            "liq_amount1_64" => {
                // (liq_amount1_64 dst spa_addr spb_addr liq_addr)
                // amount1 = L * (sqrtPb - sqrtPa)
                let dst = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la1_as");
                let addr_spb = self.local_idx("__la1_ab");
                let addr_liq = self.local_idx("__la1_al");
                let dst_i = self.local_idx("__la1_d");
                let spa_h = self.local_idx("__la1_sh");
                let spb_h = self.local_idx("__la1_bh");
                let liq_h = self.local_idx("__la1_lh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load high words
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_h));
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_h));
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_h));
                // result_hi = liq_h * (spb_h - spa_h)
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::LocalGet(spb_h)); v.push(Instruction::LocalGet(spa_h)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(liq_h));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (price64_to_tick addr) → i64
            // Reads Q64.64 price from addr, returns tick = log(price) / log(1.0001)
            // Uses binary log: find MSB, iterate for fractional bits
            "price64_to_tick" => {
                // (price64_to_tick addr) → i64
                // Linear approximation: tick ≈ (price-1) * 10001
                // Good for ±500 ticks (< 0.5% error), acceptable for CLMM range queries
                // For wider range: iterate with tick_to_price64 refinement
                let pa = self.expr(&a[0])?;
                let addr_i = self.local_idx("__p2t_a");
                let ph = self.local_idx("__p2t_ph");
                let pl = self.local_idx("__p2t_pl");
                let diff = self.local_idx("__p2t_d");
                let tick = self.local_idx("__p2t_t");
                let mut v = Vec::new();
                v.extend(pa); v.push(Instruction::LocalSet(addr_i));
                // Load Q64.64 and convert to Q32.32
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ph));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(pl));
                // q32 = (ph << 32) | (pl >> 32)
                v.push(Instruction::LocalGet(ph)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(pl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or);
                // diff = q32 - (1<<32)
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff));
                // tick = diff * 10001 >> 32
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Const(10001)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                // Quadratic correction for larger range: subtract diff^2 * 5002 >> 64
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(5002)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(tick));
                v.push(Instruction::LocalGet(tick)); Ok(v)
            }


            // (str_len s) → i64 — extract high 32 bits
            "str_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // (str_cat s1 s2) → packed string (allocates new memory)
            // Uses __stralloc counter for bump allocation
            "str_cat" => {
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__sc1");
                let s2_i = self.local_idx("__sc2");
                let l1_i = self.local_idx("__scl1");
                let l2_i = self.local_idx("__scl2");
                let dst_i = self.local_idx("__scdst");
                let i_i = self.local_idx("__sci");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Save tagged strings, then untag to get raw packed (len<<32|ptr)
                v.extend(s1); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(s2_i));
                // Extract lengths from raw packed: len = raw >> 32
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l2_i));
                // Extract pointers: ptr = raw & 0xFFFFFFFF
                let ptr1_i = self.local_idx("__scp1");
                let ptr2_i = self.local_idx("__scp2");
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr2_i));
                // Allocate destination buffer
                let alloc_base = self.next_data_offset.max(2048);
                v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(dst_i));
                // ── Copy s1 bytes: dst[0..l1] = s1_ptr[0..l1] ──
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break if i >= l1
                // dst[i] = ptr1[i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(ptr1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block

                // ── Copy s2 bytes: dst[l1..l1+l2] = s2_ptr[0..l2] ──
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break if i >= l2
                // dst[l1+i] = ptr2[i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(ptr2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block

                // Return (total_len << 32) | dst — tagged as Str
                v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                // Bump allocator for next allocation
                let new_off = (alloc_base + 4096) & !7;
                self.next_data_offset = new_off;
                Ok(v)
            }

            // (str_eq s1 s2) → i64 (0 or 1)
            "str_eq" => {
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__se1");
                let s2_i = self.local_idx("__se2");
                let l1_i = self.local_idx("__sel1");
                let i_i = self.local_idx("__sei");
                let res_i = self.local_idx("__seres");
                let mut v = Vec::new();
                v.extend(s1); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.push(Instruction::LocalSet(s2_i));
                // l1 = s1 >> 32
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                // if l1 != (s2 >> 32) → 0
                v.push(Instruction::LocalGet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Compare byte by byte
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(res_i)); // assume equal
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if s1_ptr[i] != s2_ptr[i]: res=0, break
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Ne);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(res_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(res_i));
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (u32-to-bytes n) → tagged string of 4 bytes little-endian
            "u32-to-bytes" => {
                let val_expr = self.expr(&a[0])?;
                let val_i = self.local_idx("__u32b_val");
                let buf_i = self.local_idx("__u32b_buf");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 1, align: 0, memory_index: 0 };
                let ma2 = wasm_encoder::MemArg { offset: 2, align: 0, memory_index: 0 };
                let ma3 = wasm_encoder::MemArg { offset: 3, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate arg, untag, store in val_i
                v.extend(val_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(val_i));
                if !self.p2_mode && !self.wasi_mode {
                    // NEAR: allocate from FP_GLOBAL
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::LocalSet(buf_i));
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::GlobalSet(FP_GLOBAL));
                } else {
                    // P2/WASI: compile-time allocation
                    let alloc_base = self.next_data_offset.max(3072);
                    self.next_data_offset = (alloc_base + 8) & !7;
                    v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(buf_i));
                }
                // byte 0: store (val & 0xFF) at buf+0
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // byte 1: store ((val >> 8) & 0xFF) at buf+1
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma1));
                // byte 2: store ((val >> 16) & 0xFF) at buf+2
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma2));
                // byte 3: store ((val >> 24) & 0xFF) at buf+3
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(24)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma3));
                // Build tagged string: (4 << 32) | buf_addr, then tag with TAG_STR
                v.push(Instruction::I64Const(4)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (bytes-to-u32 s) → tagged number: read 4 little-endian bytes from string
            "bytes-to-u32" => {
                let s_expr = self.expr(&a[0])?;
                let packed_i = self.local_idx("__b32u_packed");
                let ptr_i = self.local_idx("__b32u_ptr");
                let b0_i = self.local_idx("__b32u_b0");
                let b1_i = self.local_idx("__b32u_b1");
                let b2_i = self.local_idx("__b32u_b2");
                let b3_i = self.local_idx("__b32u_b3");
                let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 1, align: 0, memory_index: 0 };
                let ma2 = wasm_encoder::MemArg { offset: 2, align: 0, memory_index: 0 };
                let ma3 = wasm_encoder::MemArg { offset: 3, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate arg, untag string tag, store packed (len<<32|ptr)
                v.extend(s_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(packed_i));
                // Extract ptr = low 32 bits of packed
                v.push(Instruction::LocalGet(packed_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr_i));
                // b0 = I32Load8U(ptr+0)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma0));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b0_i));
                // b1 = I32Load8U(ptr+1)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b1_i));
                // b2 = I32Load8U(ptr+2)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma2));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b2_i));
                // b3 = I32Load8U(ptr+3)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma3));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b3_i));
                // result = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
                v.push(Instruction::LocalGet(b0_i));
                v.push(Instruction::LocalGet(b1_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.push(Instruction::LocalGet(b2_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.push(Instruction::LocalGet(b3_i)); v.push(Instruction::I64Const(24)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (str_to_int s) → i64 — parse decimal string
            "str_to_int" => {
                let s = self.expr(&a[0])?;
                let s_i = self.local_idx("__sti_s");
                let len_i = self.local_idx("__sti_len");
                let i_i = self.local_idx("__sti_i");
                let acc_i = self.local_idx("__sti_acc");
                let ch_i = self.local_idx("__sti_ch");
                let neg_i = self.local_idx("__sti_neg");
                let mut v = Vec::new();
                v.extend(s); v.push(Instruction::LocalSet(s_i));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Check for leading '-'
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // if ch == '-' (45)
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(45)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(i_i)); // skip '-'
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // ch = s_ptr[i]
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // acc = acc * 10 + (ch - 48)
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0)); // fallback
                v.push(Instruction::End); // block
                // Apply negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Sub); // 0 - acc
                v.push(Instruction::Else);
                // Identity — but we need the value on stack. It's already there from the block.
                // Hmm, the block result is already on the stack. The if consumes it.
                // We need to save it to a local first.
                v.pop(); // remove the Else we just added
                // Save block result, then branch
                v.push(Instruction::LocalSet(acc_i)); // save
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::End);
                Ok(v)
            }

            // ── String operations ──

            // (str-len s) → length of string in bytes (tagged num)
            "str-len" => {
                if a.len() != 1 { return Err("str-len: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                // Untag string → raw = (len << 32) | ptr
                v.extend(self.emit_untag());
                // Extract len: raw >> 32
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (str-ptr s) → raw i64 pointer (untagged number)
            // Extracts the low 32 bits of the untagged string descriptor
            "str-ptr" => {
                if a.len() != 1 { return Err("str-ptr: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                // Low 32 bits = ptr: wrap to i32 then extend back to i64
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (str-slice s start end) → substring from start to end (exclusive)
            // NEAR mode: copies into frame buffer (safe against dangling pointers)
            // P2 mode: zero-copy (adjusts ptr+len, no allocation)
            // Bounds check: start ≤ end ≤ original string length
            "str-slice" => {
                if a.len() != 3 { return Err("str-slice: expected 3 args (string, start, end)".into()); }
                if self.p2_mode || self.wasi_mode {
                    // P2/WASI: zero-copy (no FP_GLOBAL available)
                    let raw_i = self.local_idx("__ss_raw");
                    let start_i = self.local_idx("__ss_start");
                    let end_i = self.local_idx("__ss_end");
                    let orig_len_i = self.local_idx("__ss_olen");
                    let mut v = Vec::new();
                    v.extend(self.expr(&a[0])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw_i));
                    v.push(Instruction::LocalGet(raw_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(orig_len_i));
                    v.extend(self.expr(&a[1])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(start_i));
                    v.extend(self.expr(&a[2])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(end_i));
                    v.push(Instruction::LocalGet(end_i)); v.push(Instruction::LocalGet(orig_len_i)); v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                    v.push(Instruction::LocalGet(start_i)); v.push(Instruction::LocalGet(end_i)); v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                    v.push(Instruction::LocalGet(raw_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalGet(start_i)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(end_i)); v.push(Instruction::LocalGet(start_i)); v.push(Instruction::I64Sub);
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    v.extend(self.emit_tag_num());
                    v.push(Instruction::I64Const(5)); v.push(Instruction::I64Or);
                    return Ok(v);
                }
                // NEAR mode: copy-based str-slice
                let raw_i = self.local_idx("__ss_raw");
                let start_i = self.local_idx("__ss_start");
                let end_i = self.local_idx("__ss_end");
                let new_len_i = self.local_idx("__ss_nlen");
                let src_ptr_i = self.local_idx("__ss_srcp");
                let dst_i = self.local_idx("__ss_dst");
                let dst_save_i = self.local_idx("__ss_dst_save");
                let qwords_i = self.local_idx("__ss_qw");
                let remain_i = self.local_idx("__ss_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                // Get raw string descriptor: untag → (len << 32) | ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(raw_i));
                // Evaluate and store start/end
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(start_i));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(end_i));
                // new_len = end - start
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(new_len_i));
                // Bounds check: end > (raw >> 32) → trap
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::LocalGet(raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: start > end → trap
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // src_ptr = (raw & 0xFFFFFFFF) + start
                v.push(Instruction::LocalGet(raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(src_ptr_i));
                // Allocate dst from FP_GLOBAL
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalSet(dst_save_i)); // save original dst
                // Bounds check: FP + new_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Advance FP: aligned up to 8
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalGet(new_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                v.push(Instruction::GlobalSet(FP_GLOBAL));
                // Word copy: qwords = new_len / 8, remain = new_len & 7
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                // Word loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder byte copy
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(ma8));
                v.push(Instruction::I64Store8(ma8));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Build result: (new_len << 32) | dst_save, tagged as Str
                v.push(Instruction::LocalGet(new_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_save_i));
                v.push(Instruction::I64Or);
                // Tag as Str
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Const(5)); // TAG_STR
                v.push(Instruction::I64Or);
                Ok(v)
            }

            // (str-contains-byte s byte_val) → bool
            // Checks if string s contains byte with value byte_val (0-255)
            "str-contains-byte" => {
                if a.len() != 2 { return Err("str-contains-byte: expected 2 args".into()); }
                let str_i = self.local_idx("__scb_str");
                let byte_i = self.local_idx("__scb_byte");
                let len_i = self.local_idx("__scb_len");
                let ptr_i = self.local_idx("__scb_ptr");
                let idx_i = self.local_idx("__scb_idx");
                let found_i = self.local_idx("__scb_found");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 16) & !7;
                let mut v = Vec::new();
                // Eval string, untag, store raw
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(str_i));
                // Eval byte value, untag, store
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(byte_i));
                // Extract len and ptr
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(ptr_i));
                // found = 0, idx = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(found_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx_i));
                // Loop: while idx < len && !found
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if idx >= len, break
                v.push(Instruction::LocalGet(idx_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break
                // load byte at ptr + idx
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(idx_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U);
                // compare with target byte
                v.push(Instruction::LocalGet(byte_i));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(found_i));
                v.push(Instruction::Br(1)); // break outer block (found)
                v.push(Instruction::End);
                // idx++
                v.push(Instruction::LocalGet(idx_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx_i));
                v.push(Instruction::Br(0)); // continue loop
                v.push(Instruction::End); // end loop
                v.push(Instruction::End); // end block
                // Return found as tagged bool
                v.push(Instruction::LocalGet(found_i));
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (strlcpy dst_ptr src_ptr len) → copies len bytes from src_ptr to dst_ptr
            // Uses 8-byte word copies via I64Load/I64Store (byte-by-byte I32Load8U is broken on NEAR)
            // dst_ptr/src_ptr are tagged nums (raw i64 pointers)
            "strlcpy" => {
                if a.len() != 3 { return Err("strlcpy: expected 3 args (dst_ptr src_ptr len)".into()); }
                let src_i = self.local_idx("__slc_src");
                let dst_i = self.local_idx("__slc_dst");
                let len_i = self.local_idx("__slc_len");
                let qwords_i = self.local_idx("__slc_qw");
                let remain_i = self.local_idx("__slc_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                // Evaluate args, untag, store
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(dst_i));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(src_i));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(len_i));
                // Bounds check: src_ptr + len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: dst_ptr + len ≤ mem_limit
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // qwords = len / 8
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(qwords_i));
                // remain = len & 7
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(remain_i));
                // Word copy loop: while qwords > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                // dst[i64.load src]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder tail: while remain > 0, copy 1 byte via I64Load8U + I64Store8
                // (Note: I64Load8U is different from I32Load8U — worth testing)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return total length (like strlcpy returns strlen(src))
                v.push(Instruction::LocalGet(len_i));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (strlcat dst_ptr src_ptr dst_offset len) → appends len bytes from src_ptr to dst_ptr+dst_offset
            // Wrapper around strlcpy that offsets dst by the existing content length
            "strlcat" => {
                if a.len() != 4 { return Err("strlcat: expected 4 args (dst_ptr src_ptr dst_offset len)".into()); }
                // Just emit: (strlcpy (i64.add dst_ptr dst_offset) src_ptr len)
                // We inline it to avoid a recursive call
                let src_i = self.local_idx("__slt_src");
                let dst_i = self.local_idx("__slt_dst");
                let off_i = self.local_idx("__slt_off");
                let len_i = self.local_idx("__slt_len");
                let qwords_i = self.local_idx("__slt_qw");
                let remain_i = self.local_idx("__slt_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(dst_i));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(src_i));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(off_i));
                v.extend(self.expr(&a[3])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(len_i));
                // Bounds check: src_ptr + len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: dst_ptr + offset + len ≤ mem_limit
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // dst += offset
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                // qwords = len / 8
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(remain_i));
                // Word copy loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder tail
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return total length
                v.push(Instruction::LocalGet(len_i));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (str-cat a b) → concatenated byte string
            // NEAR mode: allocates from FP_GLOBAL with frame discipline
            // P2 mode: allocates from heap_ptr (compiler bump allocator)
            "str-cat" => {
                if a.len() != 2 { return Err("str-cat: expected 2 args".into()); }
                if self.p2_mode || self.wasi_mode {
                    // P2/WASI: use heap_ptr bump allocator (no FP_GLOBAL)
                    let a_raw_i = self.local_idx("__sc_a");
                    let b_raw_i = self.local_idx("__sc_b");
                    let a_len_i = self.local_idx("__sc_a_len");
                    let a_ptr_i = self.local_idx("__sc_a_ptr");
                    let b_len_i = self.local_idx("__sc_b_len");
                    let b_ptr_i = self.local_idx("__sc_b_ptr");
                    let total_len_i = self.local_idx("__sc_total");
                    let dst_i = self.local_idx("__sc_dst");
                    let dst_save_i = self.local_idx("__sc_dst_save");
                    let qwords_i = self.local_idx("__sc_qw");
                    let remain_i = self.local_idx("__sc_rem");
                    let mut v = Vec::new();
                    let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(a_raw_i));
                    v.push(Instruction::LocalGet(a_raw_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(a_len_i));
                    v.push(Instruction::LocalGet(a_raw_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(a_ptr_i));
                    v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(b_raw_i));
                    v.push(Instruction::LocalGet(b_raw_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(b_len_i));
                    v.push(Instruction::LocalGet(b_raw_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_ptr_i));
                    v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(total_len_i));
                    // Use heap_ptr as bump allocator for P2
                    let hp = self.heap_ptr as i64;
                    self.heap_ptr = (self.heap_ptr as i64 + 4096) as u32; // reserve page for runtime
                    v.push(Instruction::I64Const(hp)); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalSet(dst_save_i));
                    // Copy A (word loop + remainder)
                    v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                    v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                    v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64Load(ma)); v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                    v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);
                    v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64Load8U(ma8)); v.push(Instruction::I64Store8(ma8));
                    v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                    v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);
                    // Copy B (word loop + remainder)
                    v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                    v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                    v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64Load(ma)); v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                    v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);
                    v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64Load8U(ma8)); v.push(Instruction::I64Store8(ma8));
                    v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                    v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);
                    // Build result
                    v.push(Instruction::LocalGet(total_len_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(dst_save_i)); v.push(Instruction::I64Or);
                    v.extend(self.emit_tag_num());
                    v.push(Instruction::I64Const(5)); v.push(Instruction::I64Or);
                    return Ok(v);
                }
                // NEAR mode: frame-based allocation
                let a_raw_i = self.local_idx("__sc_a");
                let b_raw_i = self.local_idx("__sc_b");
                let a_len_i = self.local_idx("__sc_a_len");
                let a_ptr_i = self.local_idx("__sc_a_ptr");
                let b_len_i = self.local_idx("__sc_b_len");
                let b_ptr_i = self.local_idx("__sc_b_ptr");
                let dst_i = self.local_idx("__sc_dst");
                let dst_save_i = self.local_idx("__sc_dst_save");
                let total_len_i = self.local_idx("__sc_total");
                let qwords_i = self.local_idx("__sc_qw");
                let remain_i = self.local_idx("__sc_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                // Evaluate a, extract raw descriptor
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a_raw_i));
                v.push(Instruction::LocalGet(a_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(a_len_i));
                v.push(Instruction::LocalGet(a_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(a_ptr_i));
                // Evaluate b, extract raw descriptor
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(b_raw_i));
                v.push(Instruction::LocalGet(b_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(b_len_i));
                v.push(Instruction::LocalGet(b_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(b_ptr_i));
                // total_len = a_len + b_len
                v.push(Instruction::LocalGet(a_len_i));
                v.push(Instruction::LocalGet(b_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(total_len_i));
                // Allocate dst at FP_GLOBAL (frame pointer)
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalSet(dst_save_i));
                // Bounds check: FP + total_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(total_len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Round up FP advance to 8-byte boundary for safe word copies
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalGet(total_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And); // align up to 8
                v.push(Instruction::GlobalSet(FP_GLOBAL));
                // ── Copy A: word-copy loop (I64Load/I64Store) ──
                // qwords = a_len / 8, remain = a_len & 7
                v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                // Word loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder bytes via I64Load8U/I64Store8
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // ── Copy B: same word-copy loop, dst is now at a_len offset (strlcat) ──
                v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder bytes for B
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Build result: ((total_len << 32) | dst_save) << 3 | TAG_STR
                v.push(Instruction::LocalGet(total_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_save_i));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Const(5)); // TAG_STR
                v.push(Instruction::I64Or);
                Ok(v)
            }

            // (str-repeat s n) → s repeated n times
            "str-repeat" => {
                if a.len() != 2 { return Err("str-repeat: expected 2 args".into()); }
                let src_i = self.local_idx("__sr_src");
                let count_i = self.local_idx("__sr_count");
                let src_len_i = self.local_idx("__sr_src_len");
                let src_ptr_i = self.local_idx("__sr_src_ptr");
                let dst_i = self.local_idx("__sr_dst");
                let rep_i = self.local_idx("__sr_rep");
                let off_i = self.local_idx("__sr_off");
                let j_i = self.local_idx("__sr_j");
                let alloc_base = self.next_data_offset.max(3072);
                // We'll allocate at alloc_base; advance next_data_offset later
                let mut v = Vec::new();
                // Eval string arg
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                // Eval count arg
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(count_i));
                // Extract src len and ptr
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // Total size = src_len * count
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::LocalGet(count_i));
                v.push(Instruction::I64Mul);
                // Allocate that many bytes
                v.push(Instruction::LocalSet(off_i));
                let total_size_local = off_i;
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Advance next_data_offset
                let _new_offset = format!("{} + total_size rounded up", alloc_base);
                // We'll fix next_data_offset after we know total_size... but it's runtime.
                // For now, allocate a generous fixed buffer and advance by a worst-case amount.
                // Actually, since count is often a literal, we can handle that. For runtime count,
                // use a generous upper bound.
                // Use a 4096-byte buffer at alloc_base.
                self.next_data_offset = (alloc_base + 4096) & !7;
                // rep = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rep_i));
                // outer loop: for rep in 0..count
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(rep_i));
                v.push(Instruction::LocalGet(count_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break
                // j = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                // inner loop: copy src byte by byte
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break inner
                // dst[rep*src_len + j] = src[j]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rep_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                // Load src[j]
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0)); // continue inner
                v.push(Instruction::End); // end inner loop
                v.push(Instruction::End); // end inner block
                // rep++
                v.push(Instruction::LocalGet(rep_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rep_i));
                v.push(Instruction::Br(0)); // continue outer
                v.push(Instruction::End); // end outer loop
                v.push(Instruction::End); // end outer block
                // Return tagged string: (total_size << 32) | alloc_base, tagged as Str
                v.push(Instruction::LocalGet(total_size_local));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (hex-encode bytes_str) → hex string (lowercase, 2 chars per byte)
            "hex-encode" => {
                if a.len() != 1 { return Err("hex-encode: expected 1 arg".into()); }
                let src_i = self.local_idx("__he_src");
                let src_len_i = self.local_idx("__he_src_len");
                let src_ptr_i = self.local_idx("__he_src_ptr");
                let dst_i = self.local_idx("__he_dst");
                let i_i = self.local_idx("__he_i");
                let b_i = self.local_idx("__he_b");
                let off_i = self.local_idx("__he_off"); // src byte offset
                let shift_i = self.local_idx("__he_shift");
                let hex_byte_i = self.local_idx("__he_hb");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 4096) & !7;
                let hex_table_off = self.alloc_data(b"0123456789abcdef");
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                // Extract len and ptr
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // dst = alloc_base (8-byte aligned)
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Zero the dst buffer first (clear 512 bytes = max 256 input bytes → 512 hex chars)
                // Write 64 zero-words (512 / 8 = 64)
                for off in (0..512).step_by(8) {
                    v.push(Instruction::I64Const((alloc_base + off) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Store(ma8));
                }
                // i = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load source byte via I64Load word-read
                // off = src_ptr + i
                v.push(Instruction::LocalGet(src_ptr_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(off_i));
                // shift = (off & 7) * 8
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalSet(shift_i));
                // b = (i64.load(off & ~7) >> shift) & 0xFF
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::LocalGet(shift_i));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(b_i));
                // Lookup hi nibble: hex_table[b >> 4]
                // hex_table_off + (b >> 4) is the byte address
                // Load word at (hex_table_off + hi) & ~7, shift by ((hex_table_off + hi) & 7) * 8
                {
                    v.push(Instruction::LocalGet(b_i));
                    v.push(Instruction::I64Const(4)); v.push(Instruction::I64ShrU); // hi = b >> 4
                    v.push(Instruction::I64Const(hex_table_off as i64));
                    v.push(Instruction::I64Add); // hex_table_off + hi
                    v.push(Instruction::LocalSet(off_i)); // reuse off_i
                    // shift
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    // load & extract
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                    // Save hex byte before computing dst offset
                    v.push(Instruction::LocalSet(hex_byte_i));
                    // Store at dst + 2*i — read-modify-write using I64Store
                    // dst_off = dst + 2*i
                    v.push(Instruction::LocalGet(dst_i));
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i)); // reuse
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i)); // dst byte shift
                    // Read-modify-write
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    // word is on stack, hex_byte saved in local
                    // Clear bit: word & ~(0xFF << shift_i)
                    v.push(Instruction::I64Const(0xFF));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl); // 0xFF << shift
                    v.push(Instruction::I64Const(-1i64 as u64 as i64));
                    v.push(Instruction::I64Xor); // ~(0xFF << shift)
                    v.push(Instruction::I64And); // word & ~mask
                    // Set bit: | (hex_byte << shift_i)
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    // I64Store needs [i32 addr, i64 val] — save val, push addr, push val
                    v.push(Instruction::LocalSet(hex_byte_i)); // reuse hex_byte_i as temp
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::I64Store(ma8));
                }
                // Lookup lo nibble: hex_table[b & 0xF]
                {
                    v.push(Instruction::LocalGet(b_i));
                    v.push(Instruction::I64Const(15)); v.push(Instruction::I64And); // lo = b & 0xF
                    v.push(Instruction::I64Const(hex_table_off as i64));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                    // Save hex byte before computing dst offset
                    v.push(Instruction::LocalSet(hex_byte_i));
                    // Store at dst + 2*i + 1
                    v.push(Instruction::LocalGet(dst_i));
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::I64Const(0xFF));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(-1i64 as u64 as i64));
                    v.push(Instruction::I64Xor);
                    v.push(Instruction::I64And);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    // I64Store needs [i32 addr, i64 val] — save val, push addr, push val
                    v.push(Instruction::LocalSet(hex_byte_i)); // reuse as temp
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::I64Store(ma8));
                }
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return: (src_len*2 << 32) | alloc_base, tagged Str
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl); // * 2
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (base64-decode str) → decoded byte string
            "base64-decode" => {
                if a.len() != 1 { return Err("base64-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b64d_src");
                let src_len_i = self.local_idx("__b64d_src_len");
                let src_ptr_i = self.local_idx("__b64d_src_ptr");
                let dst_i = self.local_idx("__b64d_dst");
                let i_i = self.local_idx("__b64d_i");
                let out_len_i = self.local_idx("__b64d_out_len");
                let a_i = self.local_idx("__b64d_a");
                let b_i = self.local_idx("__b64d_b");
                let c_i = self.local_idx("__b64d_c");
                let d_i = self.local_idx("__b64d_d");
                let val_i = self.local_idx("__b64d_val");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base64 decode table: 256 bytes, 0-63 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: process 4 chars at a time
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i + 3 >= src_len, break
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // a = table[src[i]]: load src[i] as i32, add table_off, load8_u, extend to i64
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma)); // loads src[i] (the char)
                v.push(Instruction::I32Add); // table_off + char_value
                v.push(Instruction::I32Load8U(ma)); // loads table[char] (decoded 0-63)
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(a_i));
                // b = table[src[i+1]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // c = table[src[i+2]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(2)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // d = table[src[i+3]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(3)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(d_i));
                // byte1 = (a << 2) | (b >> 4) — all i64
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // byte2 = ((b & 0xF) << 4) | (c >> 2)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(15));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte2
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // byte3 = ((c & 0x3) << 6) | d
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(d_i));
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte3
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // i += 4
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (base64url-decode str) → decoded byte string (URL-safe, no padding)
            "base64url-decode" => {
                if a.len() != 1 { return Err("base64url-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b64ud_src");
                let src_len_i = self.local_idx("__b64ud_src_len");
                let src_ptr_i = self.local_idx("__b64ud_src_ptr");
                let dst_i = self.local_idx("__b64ud_dst");
                let i_i = self.local_idx("__b64ud_i");
                let out_len_i = self.local_idx("__b64ud_out_len");
                let a_i = self.local_idx("__b64ud_a");
                let b_i = self.local_idx("__b64ud_b");
                let c_i = self.local_idx("__b64ud_c");
                let d_i = self.local_idx("__b64ud_d");
                let val_i = self.local_idx("__b64ud_val");
                let remain_i = self.local_idx("__b64ud_remain");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base64url decode table: 256 bytes, 0-63 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: process groups of 2-4 chars (no padding)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // remain = src_len - i
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(remain_i));
                // Load a = table[src[i]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(a_i));
                // Load b = table[src[i+1]] (we know remain >= 2 because we checked i < src_len, but need at least 2 chars)
                // Actually: since i < src_len, we need to check remain >= 2 — but base64url groups are at least 2.
                // If remain == 1, treat as single char (shouldn't happen in valid base64url, but handle gracefully: skip)
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64LeU);
                v.push(Instruction::BrIf(1)); // break if <= 1 char left (invalid, but safe)
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // byte0 = (a << 2) | (b >> 4)
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte0
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // If remain >= 3: load c and emit byte1
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64LtU);
                // If remain < 3, skip to end-of-group
                v.push(Instruction::If(BlockType::Empty));
                // remain < 3 → just update i and continue
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                // remain >= 3: load c
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(2)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // byte1 = ((b & 0xF) << 4) | (c >> 2)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(15));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // If remain >= 4: load d and emit byte2
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                // remain < 4 → update i += 3 and continue
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                // remain >= 4: load d
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(3)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(d_i));
                // byte2 = ((c & 0x3) << 6) | d
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(d_i));
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte2
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // i += 4
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::End); // end if remain >= 4 else
                v.push(Instruction::End); // end if remain >= 3 else
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (base58-decode str) → decoded byte string (Bitcoin-style)
            "base58-decode" => {
                if a.len() != 1 { return Err("base58-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b58d_src");
                let src_len_i = self.local_idx("__b58d_src_len");
                let src_ptr_i = self.local_idx("__b58d_src_ptr");
                let dst_i = self.local_idx("__b58d_dst");
                let i_i = self.local_idx("__b58d_i");
                let j_i = self.local_idx("__b58d_j");
                let out_len_i = self.local_idx("__b58d_out_len");
                let carry_i = self.local_idx("__b58d_carry");
                let decoded_i = self.local_idx("__b58d_decoded");
                let tmp_i = self.local_idx("__b58d_tmp");
                let leading_i = self.local_idx("__b58d_leading");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base58 decode table: 256 bytes, 0-57 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // dst = alloc_base + src_len (worst case: output can be up to src_len bytes)
                // Actually we need dst to be separate from the src data. Use alloc_base as the dst buffer.
                // Zero out the dst buffer first (we'll use up to src_len + a few bytes)
                // Actually, just use alloc_base directly. We'll keep output in little-endian there.
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Zero out the buffer area we'll use (src_len + 32 bytes to be safe)
                // For simplicity, zero 256 bytes — enough headroom
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(256));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // out_len = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                // i = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: for each input char
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // decoded = table[src[i]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(decoded_i));
                // carry = decoded
                v.push(Instruction::LocalGet(decoded_i));
                v.push(Instruction::LocalSet(carry_i));
                // Inner loop: j = 0..out_len-1
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= out_len, break
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // tmp = dst[j] (as i64)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U);
                // carry += tmp * 58
                v.push(Instruction::I64Const(58));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));
                // dst[j] = carry & 0xFF
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Const(255));
                v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // carry >>= 8
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(carry_i));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                // While carry > 0: dst[out_len] = carry & 0xFF; out_len++; carry >>= 8
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1)); // break if carry == 0
                // dst[out_len] = carry & 0xFF
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Const(255));
                v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // out_len++
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // carry >>= 8
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(carry_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // carry loop
                v.push(Instruction::End); // carry block
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                // Post-processing: count leading '1' chars (0x31) in input
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(leading_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // if src[i] != 0x31, break
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Const(0x31));
                v.push(Instruction::I32Ne);
                v.push(Instruction::BrIf(1));
                // leading++
                v.push(Instruction::LocalGet(leading_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(leading_i));
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Now we need to:
                // 1. Reverse the output buffer (little-endian → big-endian)
                //    Reverse bytes at [dst .. dst+out_len-1]
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= out_len/2, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64ShrU); // out_len / 2
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // tmp = dst[i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp_i));
                // dst[i] = dst[out_len - 1 - i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                // Load dst[out_len - 1 - i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                // dst[out_len - 1 - i] = tmp
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // reverse loop
                v.push(Instruction::End); // reverse block
                // 2. Prepend leading zeros: shift the reversed data right by 'leading' bytes
                //    Use a secondary buffer at alloc_base + 512 for the final result
                //    Actually, we can do it in-place by shifting from the end
                //    Final layout: [leading zero bytes] [reversed data]
                //    We need to move data from offset 0 to offset 'leading'
                //    Since we already reversed (big-endian), move bytes from end to start
                //    Use backward copy to avoid overwriting
                //    New out_len = leading + old_out_len
                //    dst[out_len-1 - k] = dst[old_out_len-1 - k] for k = 0..old_out_len-1
                //    First, shift data right by 'leading' bytes
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= out_len, break
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // dst[leading + out_len - 1 - j] = dst[out_len - 1 - j]
                // Store address: dst + leading + out_len - 1 - j
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(leading_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                // Load address: dst + out_len - 1 - j
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // copy loop
                v.push(Instruction::End); // copy block
                // Fill leading zeros: dst[0..leading-1] = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(leading_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // zero loop
                v.push(Instruction::End); // zero block
                // out_len += leading
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::LocalGet(leading_i));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }

            // (near/store-bytes key value) → nil
            // Stores tagged string value under string key using NEAR storage_write.
            // value must be a tagged string — stores the actual byte content.
            "near/store-bytes" => {
                if a.len() != 2 { return Err("near/store-bytes: expected 2 args".into()); }
                self.need_host(17);
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Extract val ptr and len
                let val_raw_i = self.local_idx("__sb_vr");
                v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(val_raw_i));
                // Bounds check: val_ptr + val_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // val_ptr
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // val_len
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // storage_write(key_len, key_ptr, val_len, val_ptr, register_id=0)
                // Pass val_ptr directly to storage_write (no copy needed)
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // val_len
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // val_ptr
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // (near/load-bytes key) → tagged string or nil
            // Loads variable-length bytes from NEAR storage, returns as tagged string.
            "near/load-bytes" => {
                if a.len() != 1 { return Err("near/load-bytes: expected 1 arg".into()); }
                self.need_host(18); self.need_host(0); self.need_host(1);
                let key = self.expr(&a[0])?;
                let len_i = self.local_idx("__lb_len");
                let buf_i = self.local_idx("__lb_buf");
                let mut v = Vec::new();
                // storage_read(key_len, key_ptr, register_id=1)
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                // register_len(1) → save to local
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(1));
                v.push(Instruction::LocalSet(len_i));
                // Check if -1 (not found)
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(-1i64 as u64 as i64));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return nil
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                if !self.p2_mode && !self.wasi_mode {
                    // NEAR: allocate from FP_GLOBAL (bump by max storage value size)
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::LocalSet(buf_i));
                    // read_register(1, buf)
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalGet(buf_i));
                    v.push(Self::host_call(0));
                    // Bump FP by actual length (rounded up to 8) so next allocs don't overlap
                    // FP += (len + 7) & ~7
                    v.push(Instruction::GlobalGet(FP_GLOBAL));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Const(-8)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::GlobalSet(FP_GLOBAL));
                    // Return tagged string: (len << 32) | buf
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(buf_i));
                    v.push(Instruction::I64Or);
                } else {
                    // P2/WASI: compile-time allocation
                    let alloc_base = self.next_data_offset.max(3072);
                    self.next_data_offset = (alloc_base + 8192) & !7;
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Const(alloc_base as i64));
                    v.push(Self::host_call(0));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(alloc_base as i64));
                    v.push(Instruction::I64Or);
                }
                v.extend(self.emit_tag_str());
                v.push(Instruction::End);
                Ok(v)
            }

            // (int_to_str n) → packed string
            "int_to_str" => {
                let n = self.expr(&a[0])?;
                let n_i = self.local_idx("__its_n");
                let neg_i = self.local_idx("__its_neg");
                let tmp_i = self.local_idx("__its_tmp");
                let len_i = self.local_idx("__its_len");
                let dst_i = self.local_idx("__its_dst");
                let dig_i = self.local_idx("__its_dig");
                let i_i = self.local_idx("__its_i");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 64) & !7;
                let mut v = Vec::new();
                v.extend(n); v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(dst_i));
                // Handle negative: if n < 0, neg=1, n = -n
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::End);
                // Handle n == 0
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // Write '0' at dst, len=1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(48)); // '0'
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::Else);
                // Extract digits in reverse: write to dst+31 backward
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(31)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(tmp_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // dig = n % 10
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64RemU); v.push(Instruction::LocalSet(dig_i));
                // n /= 10
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64DivU); v.push(Instruction::LocalSet(n_i));
                // mem[tmp] = '0' + dig
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dig_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(tmp_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Now digits are at [tmp+8 .. dst+31], need to move to dst[0..len-1]
                // Copy forward
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // dst[i] = (tmp+8+len-1-i)  ... actually source is at dst + (31 - len + 1 + i) = dst + 32 - len + i
                // We wrote backward from dst+31, so digits start at tmp+8 (= dst+31-len+1 = dst+32-len)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Sub); // dst+32 ... wait
                // Actually: we started at tmp=dst+31, wrote at tmp, then tmp-=8 each step.
                // After len digits: tmp = dst+31 - (len-1)*8 ... no wait, we sub 8 not 1!
                // BUG: we're storing bytes but subtracting 8. Should subtract 1.
                // Let me fix: use I32Store8 so we should subtract 1 from the pointer.
                // Actually I used I32Store8 which stores a single byte, but tmp is i64 and I subtract 8.
                // That's wrong — should subtract 1 for byte addressing.
                v.push(Instruction::End); // end the broken block early
                v.push(Instruction::End); // end if/else
                // This is getting messy. Let me restart int_to_str with a cleaner approach.
                // Actually, let me just rewrite the whole thing properly.
                return self.int_to_str_clean(&a);
            }

            // ── Array Operations ──
            // Layout: length at (offset-8), elements at offset + idx*8

            // (arr_new offset size) — zero-fill
            "arr_new" => {
                let offset_expr = self.expr(&a[0])?;
                let size_expr = self.expr(&a[1])?;
                let off_i = self.local_idx("__an_off");
                let sz_i = self.local_idx("__an_sz");
                let i_i = self.local_idx("__an_i");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off_i));
                v.extend(size_expr); v.push(Instruction::LocalSet(sz_i));
                // Store length at offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(sz_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Zero-fill loop
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(sz_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[offset + i*8] = 0
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (arr_get offset idx) → i64
            "arr_get" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (arr_set offset idx val)
            "arr_set" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let val = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (arr_len offset) → i64 — reads from offset-8
            "arr_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (arr_push offset val) — append, increment length
            "arr_push" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__ap_off");
                let len_i = self.local_idx("__ap_len");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // Load current length from offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(len_i));
                // Store val at offset + len*8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Increment length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (arr_sort offset) — bubble sort in-place
            "arr_sort" => {
                // Bubble sort: arr[offset..offset+n*8]
                // Length stored at offset-8
                let off = self.expr(&a[0])?;
                let off_i = self.local_idx("__as_off");
                let n_i = self.local_idx("__as_n");
                let i_i = self.local_idx("__as_i");
                let j_i = self.local_idx("__as_j");
                let tmp_i = self.local_idx("__as_tmp");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // n = mem[(offset-8)]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                // Outer loop: i = 0..n-1
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n-1: br 2 (exit)
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // j = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                // Inner loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= n-i-1: br 2
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Sub); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // tmp = arr[j], load arr[j+1]
                // Compare: if arr[j] > arr[j+1], swap
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(tmp_i)); // tmp = arr[j]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); // arr[j+1]
                // stack: arr[j+1]; tmp_i = arr[j]
                // if arr[j] > arr[j+1] → swap
                v.push(Instruction::LocalGet(tmp_i)); // tmp, arr[j+1] on stack
                v.push(Instruction::I64LtS); // arr[j+1] < arr[j] i.e. arr[j] > arr[j+1]
                v.push(Instruction::If(BlockType::Empty));
                // arr[j] = arr[j+1]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // arr[j+1] = tmp
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // if swap
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }

            // (arr_find offset val) → index or -1 (linear search)
            "arr_find" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__af_off");
                let val_i = self.local_idx("__af_val");
                let n_i = self.local_idx("__af_n");
                let i_i = self.local_idx("__af_i");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // Load length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(-1)); v.push(Instruction::Br(2)); // not found
                v.push(Instruction::End);
                // if arr[i] == val → return i
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::Br(2)); // found
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(-1)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }

            // Self-passing call: (me me args...) — Y-combinator pattern
            // When callee is a local and first arg is the same local, it's self-passing
            _ if self.locals.contains_key(op) && !a.is_empty() && matches!(&a[0], LispVal::Sym(s) if s == op) => {
                // Find which user function this local refers to by checking current_func
                // The pattern (me me args...) means: call current function with (me, args...)
                let pos = self.funcs.iter().position(|f| Some(f.name.as_str()) == self.current_func.as_deref())
                    .ok_or_else(|| "self-passing call outside of function".to_string())?;
                let mut v = Vec::new();
                // Push all args including the self-reference
                for x in a { v.extend(self.expr(x)?); }
                // Call current function (which has the self-param)
                v.push(Instruction::Call(USER_BASE | pos as u32));
                Ok(v)
            }

            // ── HTTP GET (OutLayer host function) ──
            "http-get" => {
                // (http-get "https://api.example.com/data") -> string or nil
                if a.is_empty() { return Err("http-get requires a URL string argument".into()); }
                if !self.wasi_mode { return Err("http-get is only available on OutLayer (WASI) target".into()); }
                if self.p2_mode { self.need_wasi_http = true; } else { self.need_outlayer = true; }

                // For P2 mode: parse the URL string literal from the source and register it
                // so that a dedicated WASM function is generated for this URL.
                let url_sentinel = if self.p2_mode {
                    // Extract URL string from the Lisp source argument
                    let url_str = match &a[0] {
                        crate::types::LispVal::Str(s) => Some(s.clone()),
                        _ => {
                            // Non-literal URL — fall back to sentinel 103 (first HTTP fn)
                            // This shouldn't happen in well-formed P2 code
                            eprintln!("⚠️ http-get with non-literal URL in P2 mode, using sentinel 103");
                            None
                        }
                    };
                    if let Some(url) = url_str {
                        if !url.is_empty() {
                            // Parse URL into (authority, path)
                            let (authority, path) = parse_url(&url);
                            // Check if this exact (authority, path) is already registered
                            let idx = if let Some(existing) = self.http_urls.iter().position(|(a, p)| a == &authority && p == &path) {
                                existing
                            } else {
                                self.http_urls.push((authority, path));
                                self.http_urls.len() - 1
                            };
                            103 + idx as u32
                        } else {
                            103u32
                        }
                    } else {
                        103u32
                    }
                } else {
                    103u32 // P1 mode: single sentinel
                };

                let url_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let errno_l = self.local_idx("__http_err");
                let len_l = self.local_idx("__http_len");
                let dst_l = self.local_idx("__http_dst");
                let mut v = Vec::new();

                // outlayer.http_get(url_ptr, url_len, response_buf, response_buf_len, response_len_ptr)
                // URL ptr/len from tagged string
                v.extend(url_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // url_ptr
                v.extend(url_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // url_len
                // response_buf at 98304, buf_len = 65536, response_len_ptr at 163840
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                // Call http_get (sentinel 103 + url_index for P2, or 103 for P1)
                v.push(Instruction::Call(url_sentinel));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(errno_l));
                // if errno != 0 → nil
                v.push(Instruction::LocalGet(errno_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Load response length
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                // Copy response to heap using byte-by-byte copy (NEAR doesn't support memory.copy)
                let copy_dst_i = self.local_idx("__resp_cdst");
                let copy_src_i = self.local_idx("__resp_csrc");
                let copy_len_i = self.local_idx("__resp_clen");
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(copy_dst_i)); // dst
                v.push(Instruction::I64Const(98304)); v.push(Instruction::LocalSet(copy_src_i)); // src
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::LocalSet(copy_len_i)); // len
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(copy_len_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(copy_src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(copy_dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(copy_src_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(copy_src_i));
                v.push(Instruction::LocalGet(copy_dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(copy_dst_i));
                v.push(Instruction::LocalGet(copy_len_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(copy_len_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Advance heap
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                // Tagged string: ((dst | (len << 32)) << 3) | TAG_STR
                v.push(Instruction::I64Const(self.heap_ptr as i64 - 65536)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End); // if
                Ok(v)
            }

            // ── Storage operations (OutLayer host functions) ──
            "storage-set" => {
                // (storage-set "key" "value") -> bool
                if a.len() < 2 { return Err("storage-set requires (key value)".into()); }
                if !self.wasi_mode { return Err("storage-set is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                // key ptr/len
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // val ptr/len
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // Call storage_set (sentinel 110)
                v.push(Instruction::Call(110));
                // Return true (errno == 0) as tagged bool
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U); // convert bool i32 to i64
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get" => {
                // (storage-get "key") -> string or nil
                if a.is_empty() { return Err("storage-get requires a key".into()); }
                if !self.wasi_mode { return Err("storage-get is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let errno_l = self.local_idx("__sg_err");
                let len_l = self.local_idx("__sg_len");
                let dst_l = self.local_idx("__sg_dst");
                let i_l = self.local_idx("__sg_i");
                let mut v = Vec::new();
                // key ptr/len
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // response buf at 98304, buf_len=65536, len_ptr at 163840
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(111));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(errno_l));
                v.push(Instruction::LocalGet(errno_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-has" => {
                // (storage-has "key") -> bool
                if a.is_empty() { return Err("storage-has requires a key".into()); }
                if !self.wasi_mode { return Err("storage-has is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(112));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num()); // 0 or 1 as tagged num (also truthy as bool)
                Ok(v)
            }
            "storage-delete" => {
                // (storage-delete "key") -> bool
                if a.is_empty() { return Err("storage-delete requires a key".into()); }
                if !self.wasi_mode { return Err("storage-delete is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(113));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-increment" => {
                // (storage-increment "key" delta) -> i64 (new value)
                if a.len() < 2 { return Err("storage-increment requires (key delta)".into()); }
                if !self.wasi_mode { return Err("storage-increment is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let delta_expr = self.expr(&a[1])?;
                let delta_expr2 = self.expr(&a[1])?;
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // key ptr/len
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // delta_lo, delta_hi from untagged delta
                v.extend(delta_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); // untag
                v.push(Instruction::I32WrapI64); // delta_lo
                v.extend(delta_expr2);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // delta_hi
                // result_lo_ptr, result_hi_ptr (use heap)
                let res_lo = self.heap_ptr;
                let res_hi = self.heap_ptr + 8;
                self.heap_ptr += 16;
                v.push(Instruction::I32Const(res_lo as i32));
                v.push(Instruction::I32Const(res_hi as i32));
                v.push(Instruction::Call(114));
                v.push(Instruction::Drop); // ignore errno for now
                // Load result as i64 from (res_lo, res_hi)
                v.push(Instruction::I32Const(res_lo as i32));
                v.push(Instruction::I64Load(ma8));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // ── Env context (OutLayer host functions) ──
            "env/signer" => {
                if !self.wasi_mode { return Err("env/signer is only available on OutLayer".into()); }
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let len_l = self.local_idx("__env_len");
                let dst_l = self.local_idx("__env_dst");
                let i_l = self.local_idx("__env_i");
                let mut v = Vec::new();
                v.push(Instruction::I32Const(98304)); // buf
                v.push(Instruction::I32Const(65536)); // buf_len
                v.push(Instruction::I32Const(163840)); // len_ptr
                v.push(Instruction::Call(120));
                v.push(Instruction::I64ExtendI32U);
                // If errno != 0, return nil
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "env/predecessor" => {
                if !self.wasi_mode { return Err("env/predecessor is only available on OutLayer".into()); }
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let len_l = self.local_idx("__env_len2");
                let dst_l = self.local_idx("__env_dst2");
                let i_l = self.local_idx("__env_i2");
                let mut v = Vec::new();
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(121));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }


            "storage-decrement" => {
                // (storage-decrement "key" delta) -> i64
                if a.len() < 2 { return Err("storage-decrement requires (key delta)".into()); }
                if !self.wasi_mode { return Err("storage-decrement is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let delta_expr = self.expr(&a[1])?;
                let delta_expr2 = self.expr(&a[1])?;
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(delta_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(delta_expr2);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                let res_lo = self.heap_ptr; let res_hi = self.heap_ptr + 8; self.heap_ptr += 16;
                v.push(Instruction::I32Const(res_lo as i32));
                v.push(Instruction::I32Const(res_hi as i32));
                v.push(Instruction::Call(130));
                v.push(Instruction::Drop);
                v.push(Instruction::I32Const(res_lo as i32));
                v.push(Instruction::I64Load(ma8));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-if-absent" => {
                // (storage-set-if-absent "key" "value") -> bool (true = was inserted)
                if a.len() < 2 { return Err("storage-set-if-absent requires (key value)".into()); }
                if !self.wasi_mode { return Err("storage-set-if-absent is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(131));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-if-equals" => {
                // (storage-set-if-equals "key" "expected" "new") -> bool
                if a.len() < 3 { return Err("storage-set-if-equals requires (key expected new)".into()); }
                if !self.wasi_mode { return Err("storage-set-if-equals is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let exp_expr = self.expr(&a[1])?;
                let new_expr = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(exp_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(exp_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(new_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(new_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // old_buf at 98304, old_len_ptr at 163840
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(132));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-list-keys" => {
                // (storage-list-keys "prefix") -> string or nil
                if a.is_empty() { return Err("storage-list-keys requires a prefix".into()); }
                if !self.wasi_mode { return Err("storage-list-keys is only available on OutLayer".into()); }
                let prefix_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let len_l = self.local_idx("__sg_lklen");
                let dst_l = self.local_idx("__sg_lkdst");
                let i_l = self.local_idx("__sg_lki");
                let mut v = Vec::new();
                v.extend(prefix_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(prefix_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(133));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-clear-all" => {
                // (storage-clear-all) -> bool
                if !self.wasi_mode { return Err("storage-clear-all is only available on OutLayer".into()); }
                let mut v = Vec::new();
                v.push(Instruction::Call(134));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-worker" => {
                // (storage-set-worker "key" "value") -> bool
                if a.len() < 2 { return Err("storage-set-worker requires (key value)".into()); }
                if !self.wasi_mode { return Err("storage-set-worker is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(135));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get-worker" => {
                // (storage-get-worker "key") -> string or nil
                if a.is_empty() { return Err("storage-get-worker requires a key".into()); }
                if !self.wasi_mode { return Err("storage-get-worker is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(136));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let len_l = self.local_idx("__sg_wlen");
                let dst_l = self.local_idx("__sg_wdst");
                let i_l = self.local_idx("__sg_wi");
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-set-worker-public" => {
                // (storage-set-worker-public "key" "value") -> bool
                if a.len() < 2 { return Err("storage-set-worker-public requires (key value)".into()); }
                if !self.wasi_mode { return Err("storage-set-worker-public is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(137));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get-worker-from-project" => {
                // (storage-get-worker-from-project "key" "project_uuid") -> string or nil
                if a.len() < 2 { return Err("storage-get-worker-from-project requires (key project_uuid)".into()); }
                if !self.wasi_mode { return Err("storage-get-worker-from-project is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let proj_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(proj_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(proj_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(138));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let len_l = self.local_idx("__sg_cplen");
                let dst_l = self.local_idx("__sg_cpdst");
                let i_l = self.local_idx("__sg_cpi");
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            // ── OutLayer RPC (string-based I/O via outlayer module imports) ──
            "outlayer/view" => {
                // (outlayer/view contract method args) -> string or nil
                // Strategy: all locals are i64. Widen i32→i64 and narrow i64→i32 at boundaries.
                if a.len() < 3 { return Err("outlayer/view requires (contract method args)".into()); }
                let contract = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args_val = self.expr(&a[2])?;
                let errno_l = self.local_idx("__ol_err");
                let len_l = self.local_idx("__ol_len");
                let dst_l = self.local_idx("__ol_dst");
                let i_l = self.local_idx("__ol_i");
                let mut v = Vec::new();
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };

                // Push 8 x i32 params for outlayer.view
                // contract ptr/len
                v.extend(contract.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // contract_ptr
                v.extend(contract);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // contract_len
                // method ptr/len
                v.extend(method.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(method);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // args ptr/len
                v.extend(args_val.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(args_val);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // result_buf, result_len_ptr
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(163840));
                // call outlayer.view (returns i32 errno)
                v.push(Instruction::Call(100));
                v.push(Instruction::I64ExtendI32U); // errno i32 → i64
                v.push(Instruction::LocalSet(errno_l));
                // if errno != 0 → nil
                v.push(Instruction::LocalGet(errno_l));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Load result length (i32 from memory → widen to i64)
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(len_l));
                // dst = heap_ptr
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::LocalSet(dst_l));
                // i = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_l));
                // Copy loop — no result type needed
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // dst[i] = src[98304 + i] — narrow to i32 for addresses
                v.push(Instruction::LocalGet(dst_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                // i++
                v.push(Instruction::LocalGet(i_l)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // advance heap
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                // Create tagged string: ((dst | (len << 32)) << 3) | TAG_STR
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End); // if
                Ok(v)
            }

            "outlayer/raw" => {
                // (outlayer/raw method params) -> string result
                // Same as outlayer/view but uses outlayer.call (sentinel 101)
                if a.len() < 2 { return Err("outlayer/raw requires (method params)".into()); }
                let method = self.expr(&a[0])?;
                let params = self.expr(&a[1])?;
                let errno_local = self.local_idx("__ol_errno");
                let len_local = self.local_idx("__ol_len");
                let dst_local = self.local_idx("__ol_dst");
                let i_local = self.local_idx("__ol_i");
                let mut v = Vec::new();
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };

                // outlayer.call takes 14 i32 params:
                // contract_ptr, contract_len, method_ptr, method_len, args_ptr, args_len,
                // gas, deposit_lo, deposit_hi, result_ptr, result_len_ptr, callback_ptr, callback_len
                // For raw RPC: contract="" (empty), method=method, args=params
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0)); // empty contract
                // method
                v.extend(method.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(method);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // args/params
                v.extend(params.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(params);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // gas, deposit_lo, deposit_hi
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                // result_buf, result_len_ptr
                v.push(Instruction::I32Const(98304)); v.push(Instruction::I32Const(163840));
                // callback (empty)
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                // call outlayer.call (sentinel 101)
                v.push(Instruction::Call(101));
                v.push(Instruction::LocalSet(errno_local));
                // Check error
                v.push(Instruction::LocalGet(errno_local));
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Load result len, copy to heap, create tagged string (same as view)
                v.push(Instruction::I32Const(163840));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::LocalSet(len_local));
                v.push(Instruction::I64Const(self.heap_ptr as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(dst_local));
                v.push(Instruction::I32Const(0)); v.push(Instruction::LocalSet(i_local));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(i_local)); v.push(Instruction::LocalGet(len_local));
                v.push(Instruction::I32GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_local)); v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304)); v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add); v.push(Instruction::LocalSet(i_local));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_local)); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(len_local)); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }

            "outlayer/status" => {
                // (outlayer/status) -> string
                // Calls outlayer.view with empty contract, method="status", args=""
                let errno_local = self.local_idx("__ol_errno_st");
                let len_local = self.local_idx("__ol_len_st");
                let dst_local = self.local_idx("__ol_dst_st");
                let i_local = self.local_idx("__ol_i_st");
                let mut v = Vec::new();
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                // outlayer.view("", "", "", "") — we pass the "status" string as a constant
                // Store "status" at a known offset
                let status_str = b"status";
                let status_offset = self.heap_ptr;
                for (j, &byte) in status_str.iter().enumerate() {
                    self.data_segments.push((status_offset + j as u32, vec![byte]));
                }
                self.heap_ptr = status_offset + 64; // align
                // outlayer.view(contract_ptr, contract_len, method_ptr, method_len, args_ptr, args_len, result_buf, result_len_ptr)
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0)); // empty contract
                v.push(Instruction::I32Const(status_offset as i32)); v.push(Instruction::I32Const(6)); // "status"
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0)); // empty args
                v.push(Instruction::I32Const(98304)); v.push(Instruction::I32Const(163840)); // result
                v.push(Instruction::Call(100)); // outlayer.view
                v.push(Instruction::LocalSet(errno_local));
                v.push(Instruction::LocalGet(errno_local));
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(163840)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::LocalSet(len_local));
                v.push(Instruction::I64Const(self.heap_ptr as i64)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(dst_local));
                v.push(Instruction::I32Const(0)); v.push(Instruction::LocalSet(i_local));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(i_local)); v.push(Instruction::LocalGet(len_local));
                v.push(Instruction::I32GeU); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_local)); v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304)); v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1)); v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_local)); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add); v.push(Instruction::LocalSet(i_local));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); v.push(Instruction::End);
                let new_heap = self.heap_ptr as i64 + 65536; self.heap_ptr = new_heap as u32;
                v.push(Instruction::LocalGet(dst_local)); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(len_local)); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }

            "outlayer/storage-set" => {
                // (outlayer/storage-set key value) -> nil
                // Delegates to outlayer.call (sentinel 101)
                if a.len() < 2 { return Err("outlayer/storage-set requires (key value)".into()); }
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                let method_str = b"__storage_set";
                let method_off = self.heap_ptr;
                for (j, &byte) in method_str.iter().enumerate() { self.data_segments.push((method_off + j as u32, vec![byte])); }
                self.heap_ptr = method_off + 64;
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(method_off as i32)); v.push(Instruction::I32Const(method_str.len() as i32));
                v.extend(key.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(98304)); v.push(Instruction::I32Const(163840));
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                v.push(Instruction::Call(101));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            "outlayer/storage-get" => {
                // (outlayer/storage-get key) -> string or nil
                if a.is_empty() { return Err("outlayer/storage-get requires (key)".into()); }
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                let method_str = b"__storage_get";
                let method_off = self.heap_ptr;
                for (j, &byte) in method_str.iter().enumerate() { self.data_segments.push((method_off + j as u32, vec![byte])); }
                self.heap_ptr = method_off + 64;
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(method_off as i32)); v.push(Instruction::I32Const(method_str.len() as i32));
                v.extend(key.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304)); v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(100));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            "outlayer/storage-has" | "outlayer/storage-delete" => {
                Ok(vec![Instruction::I64Const(TAG_NIL)])
            }

            "outlayer/context" => {
                // (outlayer/context "signer_id") -> string
                if a.is_empty() { return Err("outlayer/context requires a key string".into()); }
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                let method_str = b"__context";
                let method_off = self.heap_ptr;
                for (j, &byte) in method_str.iter().enumerate() { self.data_segments.push((method_off + j as u32, vec![byte])); }
                self.heap_ptr = method_off + 64;
                v.push(Instruction::I32Const(0)); v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(method_off as i32)); v.push(Instruction::I32Const(method_str.len() as i32));
                v.extend(key.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304)); v.push(Instruction::I32Const(163840));
                v.push(Instruction::Call(100));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            "print" | "println" => {
                // Evaluate arg, write to stdout (WASI) or log (NEAR), return nil
                if a.is_empty() {
                    return Ok(vec![Instruction::I64Const(TAG_NIL)]);
                }
                let val = self.expr(&a[0])?;
                let mut v = Vec::new();
                if self.wasi_mode {
                    // WASI: fd_write to stdout
                    // Check tag: if string (TAG_STR=5), extract ptr/len and fd_write
                    // If number, convert to decimal at STDOUT_BUF and fd_write
                    let tagged = self.local_idx("__print_val");
                    let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    // Store tagged value
                    v.extend(val);
                    v.push(Instruction::LocalSet(tagged));
                    // Check if string: (tagged & 7) == TAG_STR (5)
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(7));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(5)); // TAG_STR
                    v.push(Instruction::I64Eq);
                    // i64.eq produces i32 directly, no wrap needed
                    v.push(Instruction::If(BlockType::Empty));
                    // ── String path ──
                    // Build iov at offset 64: [ptr, len]
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU); // payload
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone())); // iov[0].buf
                    v.push(Instruction::I32Const(68));
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU); // len
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone())); // iov[0].len
                    // fd_write(1, 64, 1, nwritten=98308) — use 98308 NOT 98304 (STDIN_LEN)
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(98308));
                    v.push(Instruction::Call(WASI_FD_WRITE));
                    v.push(Instruction::Drop);
                    // If println, write newline
                    if op == "println" {
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(0x0A)); // '\n'
                        v.push(Instruction::I32Store8(ma8.clone()));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(98308));
                        v.push(Instruction::Call(WASI_FD_WRITE));
                        v.push(Instruction::Drop);
                    }
                    v.push(Instruction::Else);
                    // ── Non-string path: convert i64 to decimal ──
                    let untagged = self.local_idx("__print_un");
                    let digit_count = self.local_idx("__print_dc");
                    let is_neg = self.local_idx("__print_neg");
                    let wptr = self.local_idx("__print_wp");
                    let sb: i64 = 65536; // STDOUT_BUF
                    // Untag: >> 3 (arithmetic shift to preserve sign)
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrS);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(is_neg));
                    // Check negative
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64LtS);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(is_neg));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::End);
                    // Check zero
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I32Const(sb as i32));
                    v.push(Instruction::I32Const(0x30));
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Else);
                    // Digits backward at sb+31
                    v.push(Instruction::I64Const(sb + 31));
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64RemU);
                    v.push(Instruction::I64Const(0x30));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // ptr+1 = start
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(wptr));
                    // If negative: write '-'
                    v.push(Instruction::LocalGet(is_neg));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Ne);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Const(0x2D)); // '-'
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);
                    v.push(Instruction::End); // else (zero)
                    // fd_write: iov at TEMP+64
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone()));
                    v.push(Instruction::I32Const(68));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone()));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(98308));
                    v.push(Instruction::Call(WASI_FD_WRITE));
                    v.push(Instruction::Drop);
                    // If println, newline
                    if op == "println" {
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(0x0A));
                        v.push(Instruction::I32Store8(ma8.clone()));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(98308));
                        v.push(Instruction::Call(WASI_FD_WRITE));
                        v.push(Instruction::Drop);
                    }
                    v.push(Instruction::End); // if string/else
                } else {
                    // NEAR: use near/log (host func 28) for strings
                    self.need_host(28);
                    // For now: if arg is string literal, log it
                    v.extend(val.clone());
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU); // len
                    v.extend(val);
                    v.push(Instruction::I32WrapI64); // ptr
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Self::host_call(28));
                }
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // ── Standard library aliases ──
            // (list ...) → same as (array ...)
            "list" => {
                let count = a.len() as u32;
                let slots_needed = 1 + count;
                let ptr = self.heap_ptr;
                self.heap_ptr += slots_needed * 8;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.push(Instruction::I64Const(ptr as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(count as i64));
                v.push(Instruction::I64Store(ma));
                for (i, elem) in a.iter().enumerate() {
                    v.push(Instruction::I64Const((ptr + ((i as u32 + 1) * 8)) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.expr(elem)?);
                    v.push(Instruction::I64Store(ma));
                }
                v.push(Instruction::I64Const(((ptr as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // (car lst) → first element
            "car" | "first" => {
                if a.len() != 1 { return Err("car: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__car_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // ptr + 8 (skip count word) → first element
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                Ok(v)
            }

            // (map fn-or-name lst) → new array with fn applied to each element
            // Supports inline (fn [x] body) or named function symbol
            "map" => {
                if a.len() != 2 { return Err("map: need (map fn lst)".into()); }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "map")?;
                let arr_tmp = self.local_idx("__map_arr");
                let n_tmp = self.local_idx("__map_n");
                let i_tmp = self.local_idx("__map_i");
                let new_ptr = self.local_idx("__map_new");
                let res_tmp = self.local_idx("__map_res");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate lst, untag, save
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count from arr[0]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new array at heap
                let new_heap = self.heap_ptr;
                let slots = 1 + 64; // count + max 64 elements
                self.heap_ptr += slots * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store count at new[0]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // i = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n, break
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element: arr[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Bind to param
                v.push(Instruction::LocalSet(p_idx));
                // Evaluate body
                v.extend(self.expr(&body)?);
                v.push(Instruction::LocalSet(res_tmp));
                // Store result at new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(res_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return tagged new array
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // (filter fn-or-name lst) → new array with elements where fn is truthy
            "filter" => {
                if a.len() != 2 { return Err("filter: need (filter fn lst)".into()); }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "filter")?;
                let arr_tmp = self.local_idx("__fil_arr");
                let n_tmp = self.local_idx("__fil_n");
                let i_tmp = self.local_idx("__fil_i");
                let write_i = self.local_idx("__fil_w");
                let elem_tmp = self.local_idx("__fil_e");
                let _pred_tmp = self.local_idx("__fil_p");
                let new_ptr = self.local_idx("__fil_new");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate lst
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new array
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store initial count 0
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(ma));
                // i=0, write_i=0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(write_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(elem_tmp));
                // Bind param, eval predicate
                v.push(Instruction::LocalGet(elem_tmp));
                v.push(Instruction::LocalSet(p_idx));
                v.extend(self.expr(&body)?);
                // Check truthy: untag, then compare raw value != 0
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                // Store element at new[(write_i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(elem_tmp));
                v.push(Instruction::I64Store(ma));
                // Increment count at new[0]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // write_i++
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::End); // if
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return tagged new array
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // (cdr lst) / (rest lst) → new array without first element
            "cdr" | "rest" => {
                if a.len() != 1 { return Err("cdr: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__cdr_arr");
                let n_tmp = self.local_idx("__cdr_n");
                let new_ptr = self.local_idx("__cdr_new");
                let i_tmp = self.local_idx("__cdr_i");
                let val_tmp = self.local_idx("__cdr_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // new_count = count - 1
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store new_count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy elements 1..old_n to new[1..new_n]
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(i+2)*8] (skip count word + skip elem 0)
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // If new_count == 0, return nil instead of empty array
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                v.push(Instruction::End);
                Ok(v)
            }

            // (cons item lst) → new array with item prepended
            "cons" => {
                if a.len() != 2 { return Err("cons: expected 2 args".into()); }
                let item_tmp = self.local_idx("__cons_item");
                let arr_tmp = self.local_idx("__cons_arr");
                let n_tmp = self.local_idx("__cons_n");
                let new_ptr = self.local_idx("__cons_new");
                let i_tmp = self.local_idx("__cons_i");
                let val_tmp = self.local_idx("__cons_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval lst first (so item is evaluated after, but order doesn't matter for pure)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Eval item
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(item_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new: count + 1 elements
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store new_count = old_count + 1
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Store item at new[1]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(item_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy old elements to new[2..]
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+2)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // ── Type predicates ──
            // Each: eval arg, check tag, return tagged bool

            // (number? x) → bool: check (val & 7) == TAG_NUM (0)
            "number?" => {
                if a.len() != 1 { return Err("number?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_NUM));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (zero? x) → bool: untag num, check == 0
            "zero?" => {
                if a.len() != 1 { return Err("zero?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I64Eqz);          // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (nil? x) → bool: check val == TAGGED_NIL (4)
            "nil?" => {
                if a.len() != 1 { return Err("nil?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(TAGGED_NIL));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (list? x) → bool: check (val & 7) == TAG_ARRAY (6)
            "list?" => {
                if a.len() != 1 { return Err("list?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (bool? x) → bool: check (val & 7) == TAG_BOOL (1)
            "bool?" => {
                if a.len() != 1 { return Err("bool?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_BOOL));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (string? x) → bool: check (val & 7) == TAG_STR (5)
            "string?" => {
                if a.len() != 1 { return Err("string?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }

            // (len lst) → tagged number: load count from heap array
            "len" => {
                if a.len() != 1 { return Err("len: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (length lst) → tagged number
            "length" => {
                if a.len() != 1 { return Err("length: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (str-len s) → byte length of string
            "str-len" => {
                if a.len() != 1 { return Err("str-len: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // (nth lst idx) → element at index
            "nth" => {
                if a.len() != 2 { return Err("nth: expected 2 args".into()); }
                let arr_tmp = self.local_idx("__nth_arr");
                let idx_tmp = self.local_idx("__nth_i");
                let len_tmp = self.local_idx("__nth_len");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(idx_tmp));
                // Load list length (ptr[0])
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(len_tmp));
                // Bounds check: idx < len, otherwise trap
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(len_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable); // out of bounds
                v.push(Instruction::End);
                // Load ptr[(idx+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                Ok(v)
            }

            // (range start end) → array of integers [start, end)
            "range" => {
                if a.len() != 2 { return Err("range: need (range start end)".into()); }
                let start_tmp = self.local_idx("__rng_s");
                let end_tmp = self.local_idx("__rng_e");
                let i_tmp = self.local_idx("__rng_i");
                let write_i = self.local_idx("__rng_w");
                let new_ptr = self.local_idx("__rng_new");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(start_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(end_tmp));
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // count = 0
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(start_tmp));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(end_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Store i at new[(write_i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_tmp));
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Store(ma));
                // count++
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // write_i++, i++
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // (reverse lst) → new reversed array
            "reverse" => {
                if a.len() != 1 { return Err("reverse: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__rev_arr");
                let n_tmp = self.local_idx("__rev_n");
                let i_tmp = self.local_idx("__rev_i");
                let new_ptr = self.local_idx("__rev_new");
                let val_tmp = self.local_idx("__rev_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy in reverse
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(n - i)*8] (1-indexed from count word)
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // (reduce fn-or-name init lst) → single value
            "reduce" => {
                if a.len() != 3 { return Err("reduce: need (reduce fn init lst)".into()); }
                let (acc_name, elem_name, body) = self.resolve_lambda_2(&a[0], "reduce")?;
                let arr_tmp = self.local_idx("__red_arr");
                let n_tmp = self.local_idx("__red_n");
                let i_tmp = self.local_idx("__red_i");
                let acc_local = self.local_idx(&acc_name);
                let elem_local = self.local_idx(&elem_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval init → acc
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(acc_local));
                // Eval lst
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // i = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element arr[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(elem_local));
                // Eval body with acc and elem bound
                v.extend(self.expr(&body)?);
                v.push(Instruction::LocalSet(acc_local));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Result is acc
                v.push(Instruction::LocalGet(acc_local));
                Ok(v)
            }

            // (append a b) → new array with b's elements after a's
            "append" => {
                if a.len() != 2 { return Err("append: expected 2 args".into()); }
                let a1_tmp = self.local_idx("__ap_a");
                let a2_tmp = self.local_idx("__ap_b");
                let n1_tmp = self.local_idx("__ap_n1");
                let n2_tmp = self.local_idx("__ap_n2");
                let i_tmp = self.local_idx("__ap_i");
                let val_tmp = self.local_idx("__ap_v");
                let new_ptr = self.local_idx("__ap_new");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a1_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a2_tmp));
                // Load counts
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n1_tmp));
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n2_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 128) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store total count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::LocalGet(n2_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Copy a1
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Copy a2 starting at offset n1
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n2_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // ── Dict (string-keyed association list as flat tagged array) ──
            // Layout: TAG_ARRAY → [n_pairs, key0, val0, key1, val1, ...]
            // n_pairs at ptr[0], keys at ptr[1+2i], vals at ptr[2+2i]

            "dict" => {
                if a.len() % 2 != 0 { return Err("dict: expected even number of args (key val pairs)".into()); }
                let n_pairs = a.len() / 2;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let heap = self.heap_ptr;
                // count + 2*n_pairs elements
                let total_slots = 1 + 2 * n_pairs;
                self.heap_ptr = heap + (total_slots * 8) as u32;
                // But we need extra for alloc_data or strings — no, values are already tagged
                // We need enough space. Pad to 64 slots minimum for safety.
                if total_slots < 64 { self.heap_ptr = heap + 64 * 8; }
                let mut v = Vec::new();
                // Store n_pairs at ptr[0]: addr, value, store
                v.push(Instruction::I64Const(heap as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(n_pairs as i64));
                v.push(Instruction::I64Store(ma));
                // Store key/val pairs
                for i in 0..n_pairs {
                    let off = (1 + 2 * i) as u64;
                    let tmp = self.local_idx("__dict_kv");
                    // key: emit value → save to local → push addr → push value → store
                    v.extend(self.expr(&a[2 * i])?);
                    v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::I64Const(heap as i64 + off as i64 * 8));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(tmp));
                    v.push(Instruction::I64Store(ma));
                    // val: same pattern
                    v.extend(self.expr(&a[2 * i + 1])?);
                    let off2 = (2 + 2 * i) as u64;
                    v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::I64Const(heap as i64 + off2 as i64 * 8));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(tmp));
                    v.push(Instruction::I64Store(ma));
                }
                // Return tagged array
                v.push(Instruction::I64Const(((heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            "dict/get" => {
                if a.len() != 2 { return Err("dict/get: expected 2 args (dict key)".into()); }
                let d_ptr = self.local_idx("__dget_ptr");
                let n = self.local_idx("__dget_n");
                let key = self.local_idx("__dget_key");
                let idx = self.local_idx("__dget_idx");
                let k_raw = self.local_idx("__dget_kraw");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval key first
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(key));
                // Eval dict, untag → ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(d_ptr));
                // Load n_pairs
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Loop
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64))); // $break
                v.push(Instruction::Loop(BlockType::Empty)); // $loop
                    // if idx >= n_pairs → not found, break with nil
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::If(BlockType::Empty));
                        v.push(Instruction::I64Const(TAG_NIL));
                        v.push(Instruction::Br(2)); // break out of Block with nil
                    v.push(Instruction::End);
                    // Load key at ptr[1 + 2*idx]
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                    // Compare with search key
                    v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                    v.extend(self.emit_str_eq());
                    v.push(Instruction::I64Const(8)); // tagged true
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Found! Load val at ptr[2 + 2*idx]
                        v.push(Instruction::LocalGet(d_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma));
                        v.push(Instruction::Br(2)); // break out of Block with val
                    v.push(Instruction::End);
                    // idx++, continue loop
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                    v.push(Instruction::Br(0)); // continue
                v.push(Instruction::End); // loop
                v.push(Instruction::Unreachable); // should never reach here
                v.push(Instruction::End); // block
                Ok(v)
            }

            "dict/set" => {
                if a.len() != 3 { return Err("dict/set: expected 3 args (dict key val)".into()); }
                let d_ptr = self.local_idx("__dset_ptr");
                let n = self.local_idx("__dset_n");
                let key = self.local_idx("__dset_key");
                let val = self.local_idx("__dset_val");
                let idx = self.local_idx("__dset_idx");
                let k_raw = self.local_idx("__dset_kraw");
                let found = self.local_idx("__dset_found");
                let new_ptr = self.local_idx("__dset_new");
                let i2 = self.local_idx("__dset_i2");
                let v_tmp = self.local_idx("__dset_vtmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval key and val first
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(key));
                v.extend(self.expr(&a[2])?); v.push(Instruction::LocalSet(val));
                // Eval dict
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(d_ptr));
                // Load n_pairs
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Scan for existing key
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(found)); // 0=not found
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load key at ptr[1 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                // Compare
                v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                v.extend(self.emit_str_eq());
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalSet(found));
                    v.push(Instruction::Br(2)); // break
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block

                // Now: found != 0 means key exists at index (found-1)... actually found = idx
                // Determine new count: if found, same count; else count+1
                // Alloc new dict
                let new_heap = self.heap_ptr;
                // Max slots needed: 1 + 2*(n+1) — enough for either case
                let alloc_slots = 1 + 2 * 64; // generous allocation (max 64 pairs)
                self.heap_ptr = new_heap + alloc_slots * 8;
                v.push(Instruction::I64Const(new_heap as i64)); v.push(Instruction::LocalSet(new_ptr));

                // Branch: key found or not
                v.push(Instruction::LocalGet(found));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                    // --- Key exists: same count, copy all, update val at found index ---
                    v.push(Instruction::LocalGet(new_ptr)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64Store(ma));
                    // Copy all pairs
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                    // Copy key at old[1+2*i]
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    // Copy val at old[2+2*i] — if i == found, use new val instead
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(found));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Use new val
                        v.push(Instruction::LocalGet(new_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(val));
                        v.push(Instruction::I64Store(ma));
                    v.push(Instruction::Else);
                        // Copy old val
                        v.push(Instruction::LocalGet(d_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                        v.push(Instruction::LocalGet(new_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(v_tmp));
                        v.push(Instruction::I64Store(ma));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                v.push(Instruction::Else);
                    // --- Key not found: count+1, copy all old pairs, append new pair ---
                    v.push(Instruction::LocalGet(new_ptr)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Store(ma));
                    // Copy all old pairs
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                    // Copy key
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    // Copy val
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // Append new pair: key at [1 + 2*n], val at [2 + 2*n]
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(key));
                    v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(val));
                    v.push(Instruction::I64Store(ma));
                v.push(Instruction::End); // if found / not found
                // Return tagged new dict
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            "dict/has?" => {
                if a.len() != 2 { return Err("dict/has?: expected 2 args (dict key)".into()); }
                let d_ptr = self.local_idx("__dhas_ptr");
                let n = self.local_idx("__dhas_n");
                let key = self.local_idx("__dhas_key");
                let idx = self.local_idx("__dhas_idx");
                let k_raw = self.local_idx("__dhas_kraw");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(key));
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64))); // $break
                v.push(Instruction::Loop(BlockType::Empty)); // $loop
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::If(BlockType::Empty));
                        // Not found → tagged false
                        v.push(Instruction::I64Const(1));
                        v.push(Instruction::Br(2)); // break out of Block
                    v.push(Instruction::End);
                    // Load key
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                    v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                    v.extend(self.emit_str_eq());
                    v.push(Instruction::I64Const(8)); v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Found → tagged true
                        v.push(Instruction::I64Const(8));
                        v.push(Instruction::Br(2)); // break out of Block
                    v.push(Instruction::End);
                    // i++, continue
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                    v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::Unreachable);
                v.push(Instruction::End); // block
                Ok(v)
            }

            "dict/keys" => {
                if a.len() != 1 { return Err("dict/keys: expected 1 arg (dict)".into()); }
                let d_ptr = self.local_idx("__dk_ptr");
                let n = self.local_idx("__dk_n");
                let idx = self.local_idx("__dk_idx");
                let k_tmp = self.local_idx("__dk_tmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Alloc result list: [n, key0, key1, ...]
                let res_heap = self.heap_ptr;
                let alloc = std::cmp::max(1 + n as usize, 64);
                self.heap_ptr = res_heap + (alloc * 8) as u32;
                // Store count
                v.push(Instruction::I64Const(res_heap as i64)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64Store(ma));
                // Copy keys
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load key from dict at [1 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_tmp));
                // Store to result at [1 + i]
                v.push(Instruction::I64Const(res_heap as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(k_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((res_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            "dict/vals" => {
                if a.len() != 1 { return Err("dict/vals: expected 1 arg (dict)".into()); }
                let d_ptr = self.local_idx("__dv_ptr");
                let n = self.local_idx("__dv_n");
                let idx = self.local_idx("__dv_idx");
                let v_tmp2 = self.local_idx("__dv_tmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                let res_heap = self.heap_ptr;
                let alloc = std::cmp::max(1 + n as usize, 64);
                self.heap_ptr = res_heap + (alloc * 8) as u32;
                v.push(Instruction::I64Const(res_heap as i64)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load val from dict at [2 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp2));
                // Store to result at [1 + i]
                v.push(Instruction::I64Const(res_heap as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(v_tmp2));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((res_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }

            // User function call
            _ => {
                let pos = self.funcs.iter().position(|f| f.name == op).ok_or_else(|| format!("in {}: unknown function '{}'", self.current_func.as_deref().unwrap_or("top"), op))?;
                let func = &self.funcs[pos];
                // If function takes 0 params but args are provided, it's a value define
                // that returns a closure. Call it to get the closure, then dynamic-dispatch.
                if func.param_count == 0 && !a.is_empty() {
                    let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                    let temp_callee = self.next_local; self.next_local += 1;
                    let temp_closure_ptr = self.next_local; self.next_local += 1;
                    let lambda_id_local = self.next_local; self.next_local += 1;
                    let arg_locals: Vec<u32> = a.iter().map(|_| { let l = self.next_local; self.next_local += 1; l }).collect();
                    // 1. Call the 0-arg function to get the closure
                    let mut v = Vec::new();
                    v.push(Instruction::Call(USER_BASE | pos as u32));
                    v.push(Instruction::LocalSet(temp_callee));
                    // 2. Evaluate args
                    for (i, arg) in a.iter().enumerate() {
                        v.extend(self.expr(arg)?);
                        v.push(Instruction::LocalSet(arg_locals[i]));
                    }
                    // 3. Dispatch based on lambda_info
                    let n_lambdas = self.lambda_info.len();
                    if n_lambdas == 0 {
                        return Err(format!("compile error: dynamic call to '{}' but no functions defined yet — define the function before calling it", op));
                    }
                    v.push(Instruction::LocalGet(temp_callee));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(2));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(temp_callee));
                    v.push(Instruction::I64Const(TAG_BITS as i64));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(lambda_id_local));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(temp_closure_ptr));
                    v.push(Instruction::Else);
                    v.push(Instruction::LocalGet(temp_callee));
                    v.push(Instruction::I64Const(TAG_BITS as i64));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(temp_closure_ptr));
                    v.push(Instruction::LocalGet(temp_closure_ptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma));
                    v.push(Instruction::LocalSet(lambda_id_local));
                    v.push(Instruction::End);
                    for (lid, &(func_idx, _cap_count)) in self.lambda_info.iter().enumerate() {
                        v.push(Instruction::LocalGet(lambda_id_local));
                        v.push(Instruction::I64Const(lid as i64));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::LocalGet(temp_closure_ptr));
                        for &al in &arg_locals { v.push(Instruction::LocalGet(al)); }
                        v.push(Instruction::Call(USER_BASE | func_idx as u32));
                        v.push(Instruction::Return);
                        v.push(Instruction::Else);
                    }
                    v.push(Instruction::I64Const(-1));
                    for _ in 0..n_lambdas { v.push(Instruction::End); }
                    Ok(v)
                } else {
                    let mut v = Vec::new();
                    for x in a { v.extend(self.expr(x)?); }
                    v.push(Instruction::Call(USER_BASE | pos as u32));
                    Ok(v)
                }
            }
        }
    }

}
