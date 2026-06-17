use super::*;

impl WasmEmitter {
    pub(crate) fn call(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        // ── Domain dispatch (each returns Err("__not_handled__") if op doesn't match) ──
        macro_rules! try_domain {
            ($method:expr) => {
                match $method {
                    Ok(v) => return Ok(v),
                    Err(e) if e == "__not_handled__" => {}
                    Err(e) => return Err(e),
                }
            };
        }
        try_domain!(self.call_core(op, a));
        try_domain!(self.call_near_storage(op, a));
        try_domain!(self.call_near_io(op, a));
        try_domain!(self.call_hof(op, a));
        try_domain!(self.call_json(op, a));
        try_domain!(self.call_borsh(op, a));
        try_domain!(self.call_list(op, a));
        try_domain!(self.call_near_context(op, a));
        try_domain!(self.call_near_crypto(op, a));
        try_domain!(self.call_near_promise(op, a));
        try_domain!(self.call_near_iter(op, a));
        try_domain!(self.call_u128(op, a));
        try_domain!(self.call_fp(op, a));
        try_domain!(self.call_defi(op, a));
        try_domain!(self.call_bitwise(op, a));
        try_domain!(self.call_string(op, a));
        try_domain!(self.call_outlayer(op, a));
        try_domain!(self.call_predicate(op, a));
        try_domain!(self.call_dict(op, a));

        // ── Self-passing call: Y-combinator pattern ──
        if self.locals.contains_key(op)
            && !a.is_empty()
            && matches!(&a[0], LispVal::Sym(s) if s == op)
        {
            let pos = self
                .funcs
                .iter()
                .position(|f| Some(f.name.as_str()) == self.current_func.as_deref())
                .ok_or_else(|| "self-passing call outside of function".to_string())?;
            let mut v = Vec::new();
            for x in a {
                v.extend(self.expr(x)?);
            }
            v.push(Instruction::Call(USER_BASE | pos as u32));
            return Ok(v);
        }

        // ── Local variable holding a closure? ──
        // If op is a local variable that might hold a closure/fnref,
        // generate dynamic dispatch code (same pattern as named dispatch above).
        if self.locals.contains_key(op) && !self.funcs.iter().any(|f| f.name == op) {
            let n_lambdas = self.lambda_info.len();
            if n_lambdas > 0 && !a.is_empty() {
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let local_idx = self.locals[op];
                let temp_closure_ptr = self.next_local;
                self.next_local += 1;
                let lambda_id_local = self.next_local;
                self.next_local += 1;
                let arg_locals: Vec<u32> = a
                    .iter()
                    .map(|_| {
                        let l = self.next_local;
                        self.next_local += 1;
                        l
                    })
                    .collect();
                let mut v = Vec::new();
                // Evaluate args and save to locals
                for (i, arg) in a.iter().enumerate() {
                    v.extend(self.expr(arg)?);
                    v.push(Instruction::LocalSet(arg_locals[i]));
                }
                // Load the closure/fnref from local
                v.push(Instruction::LocalGet(local_idx));
                // Check tag: TAG_FNREF(2) or TAG_CLOSURE(3)?
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // TAG_FNREF: lambda_id = untagged value, no closure ptr
                v.push(Instruction::LocalGet(local_idx));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(lambda_id_local));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(temp_closure_ptr));
                v.push(Instruction::Else);
                // TAG_CLOSURE: closure_ptr = untagged value, load fn_idx from heap
                v.push(Instruction::LocalGet(local_idx));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(temp_closure_ptr));
                v.push(Instruction::LocalGet(temp_closure_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(lambda_id_local));
                v.push(Instruction::End);
                // Dispatch: if/else chain matching lambda_id against known lambdas
                for (lid, &(func_idx, _cap_count)) in self.lambda_info.iter().enumerate() {
                    v.push(Instruction::LocalGet(lambda_id_local));
                    v.push(Instruction::I64Const(lid as i64));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::LocalGet(temp_closure_ptr));
                    for &al in &arg_locals {
                        v.push(Instruction::LocalGet(al));
                    }
                    v.push(Instruction::Call(USER_BASE | func_idx as u32));
                    v.push(Instruction::Return);
                    v.push(Instruction::Else);
                }
                v.push(Instruction::I64Const(-1)); // fallback: return -1 (error sentinel)
                for _ in 0..n_lambdas {
                    v.push(Instruction::End);
                }
                return Ok(v);
            }
        }

        // ── Named function call / dynamic dispatch ──
        let pos = self
            .funcs
            .iter()
            .position(|f| f.name == op)
            .ok_or_else(|| {
                format!(
                    "in {}: unknown function '{}'",
                    self.current_func.as_deref().unwrap_or("top"),
                    op
                )
            })?;
        let func = &self.funcs[pos];
        if func.param_count == 0 && !a.is_empty() {
            let ma = wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            };
            let temp_callee = self.next_local;
            self.next_local += 1;
            let temp_closure_ptr = self.next_local;
            self.next_local += 1;
            let lambda_id_local = self.next_local;
            self.next_local += 1;
            let arg_locals: Vec<u32> = a
                .iter()
                .map(|_| {
                    let l = self.next_local;
                    self.next_local += 1;
                    l
                })
                .collect();
            let mut v = Vec::new();
            v.push(Instruction::Call(USER_BASE | pos as u32));
            v.push(Instruction::LocalSet(temp_callee));
            for (i, arg) in a.iter().enumerate() {
                v.extend(self.expr(arg)?);
                v.push(Instruction::LocalSet(arg_locals[i]));
            }
            let n_lambdas = self.lambda_info.len();
            if n_lambdas == 0 {
                return Err(format!(
                    "compile error: dynamic call to '{}' but no functions defined yet",
                    op
                ));
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
                for &al in &arg_locals {
                    v.push(Instruction::LocalGet(al));
                }
                v.push(Instruction::Call(USER_BASE | func_idx as u32));
                v.push(Instruction::Return);
                v.push(Instruction::Else);
            }
            v.push(Instruction::I64Const(-1));
            for _ in 0..n_lambdas {
                v.push(Instruction::End);
            }
            Ok(v)
        } else {
            let mut v = Vec::new();
            for x in a {
                v.extend(self.expr(x)?);
            }
            v.push(Instruction::Call(USER_BASE | pos as u32));
            Ok(v)
        }
    }
}
