
use super::*;

impl WasmEmitter {
    pub(crate) fn emit_dynamic_call(&mut self, callee: &LispVal, args: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        
        // Create temp locals for this call site
        let temp_callee = self.next_local; self.next_local += 1;
        let temp_closure_ptr = self.next_local; self.next_local += 1;
        let lambda_id_local = self.next_local; self.next_local += 1;
        let arg_locals: Vec<u32> = args.iter().map(|_| { let l = self.next_local; self.next_local += 1; l }).collect();
        
        // 1. Evaluate callee (this triggers emit_lambda which populates lambda_info)
        let mut v = self.expr(callee)?;
        v.push(Instruction::LocalSet(temp_callee));
        
        // 2. Evaluate args
        for (i, arg) in args.iter().enumerate() {
            v.extend(self.expr(arg)?);
            v.push(Instruction::LocalSet(arg_locals[i]));
        }
        
        // 3. Now lambda_info is populated — generate dispatch
        let n_lambdas = self.lambda_info.len();
        if n_lambdas == 0 {
            return Err("dynamic call but no lambdas defined".into());
        }
        
        // Compute lambda_id from callee tag
        // First compute (callee & 3) to determine tag, then dispatch
        v.push(Instruction::LocalGet(temp_callee));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Const(2));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        // fn-ref path
        v.push(Instruction::LocalGet(temp_callee));
        v.push(Instruction::I64Const(TAG_BITS as i64));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(lambda_id_local));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(temp_closure_ptr));
        v.push(Instruction::Else);
        // closure path
        v.push(Instruction::LocalGet(temp_callee));
        v.push(Instruction::I64Const(TAG_BITS as i64));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(temp_closure_ptr));
        v.push(Instruction::LocalGet(temp_closure_ptr));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma));
        v.push(Instruction::LocalSet(lambda_id_local));
        v.push(Instruction::End);
        
        // Sequential if/else dispatch — use Block+Br instead of Return
        // so dynamic calls can be used as sub-expressions
        v.push(Instruction::Block(BlockType::Result(ValType::I64)));
        for (lid, &(func_idx, _cap_count)) in self.lambda_info.iter().enumerate() {
            v.push(Instruction::LocalGet(lambda_id_local));
            v.push(Instruction::I64Const(lid as i64));
            v.push(Instruction::I64Eq);
            v.push(Instruction::If(BlockType::Empty));
            v.push(Instruction::LocalGet(temp_closure_ptr));
            for &al in &arg_locals { v.push(Instruction::LocalGet(al)); }
            v.push(Instruction::Call(USER_BASE | func_idx as u32));
            v.push(Instruction::Br(1)); // break out of Block with result
            v.push(Instruction::Else);
        }
        v.push(Instruction::I64Const(-1)); // unreachable fallback
        for _ in 0..n_lambdas { v.push(Instruction::End); }
        v.push(Instruction::End); // end Block
        
        Ok(v)
    }

}
