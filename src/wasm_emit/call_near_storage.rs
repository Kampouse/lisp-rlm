use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_storage(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
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
            "near/storage_usage" => { let mut v = vec![Self::host_call(11)]; v.extend(self.emit_tag_num()); Ok(v) },
            _ => Err("__not_handled__".into()),
        }
    }
}
