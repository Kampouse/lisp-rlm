use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_iter(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
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
            _ => Err("__not_handled__".into()),
        }
    }
}
