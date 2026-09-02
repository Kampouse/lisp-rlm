use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_iter(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            // (near/iter_prefix prefix-str) → iter id (tagged Num).
            // TS surface: near.iterPrefix("amm:") — one packed string arg.
            // Lowers: write prefix to register 0, storage_iter_prefix(len, 0)
            // → iterator id on the stack.
            "near/iter_prefix" => {
                if a.len() != 1 {
                    return Err(format!(
                        "near/iter_prefix: need exactly 1 arg (prefix string), got {}",
                        a.len()
                    ));
                }
                let prefix = self.expr(&a[0])?;
                self.need_host(36); // storage_iter_prefix
                self.need_host(0);  // read_register
                self.need_host(1);  // register_len
                let p = self.local_idx("__itp_p");
                let mut v = Vec::new();
                v.extend(prefix);
                v.push(Instruction::LocalSet(p));
                Self::emit_assert_tag_str(&mut v, p);
                // write_register(0, len, ptr) — idx 2
                v.push(Instruction::I64Const(0)); // register_id = 0
                v.push(Instruction::LocalGet(p));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // len
                v.push(Instruction::LocalGet(p));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(2));
                // storage_iter_prefix(prefix_len, register_id=0) — idx 36
                v.push(Instruction::LocalGet(p));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(36));
                // → iter id (i64), tag as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            // (near/iter_range start-str end-str) → iter id (tagged Num).
            "near/iter_range" => {
                if a.len() != 2 {
                    return Err(format!(
                        "near/iter_range: need exactly 2 args (start string, end string), got {}",
                        a.len()
                    ));
                }
                let start = self.expr(&a[0])?;
                let end = self.expr(&a[1])?;
                self.need_host(37);
                self.need_host(0);
                self.need_host(1);
                let s = self.local_idx("__itr_s");
                let e = self.local_idx("__itr_e");
                let mut v = Vec::new();
                v.extend(start);
                v.push(Instruction::LocalSet(s));
                v.extend(end);
                v.push(Instruction::LocalSet(e));
                // write start to register 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2));
                // write end to register 1
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(e));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(e));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2));
                // storage_iter_range(start_len, reg0, end_len, reg1) — idx 37
                v.push(Instruction::LocalGet(s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(e));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(37));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            // (near/iter_next iter-id) → next key as tagged Str, NIL when done.
            // TS surface: near.iterNext(id). Host: storage_iter_next(id,
            // key_reg=1, val_reg=2); we read register 1 back as the key.
            "near/iter_next" => {
                if a.len() != 1 {
                    return Err(format!(
                        "near/iter_next: need exactly 1 arg (iter id), got {}",
                        a.len()
                    ));
                }
                self.need_host(38);
                self.need_host(0);
                self.need_host(1);
                let id = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(id);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(1)); // key register
                v.push(Instruction::I64Const(2)); // value register (unused)
                v.push(Self::host_call(38));
                // status on stack: 1 = key written, 0 = exhausted
                let st = self.local_idx("__itn_st");
                v.push(Instruction::LocalSet(st));
                v.push(Instruction::LocalGet(st));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // success: read key register (1) into a fresh heap buffer
                let buf = self.local_idx("__itn_buf");
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(1)); // register_len(1)
                let len_i = self.local_idx("__itn_len");
                v.push(Instruction::LocalSet(len_i));
                v.extend(self.emit_rtheap_alloc(buf, len_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(buf));
                v.push(Self::host_call(0)); // read_register(1, buf)
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                // fresh HEAP copy — register buffers alias on re-use
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_str());
                v.extend(self.emit_str_concat());
                v.push(Instruction::Else);
                // exhausted: NIL
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}