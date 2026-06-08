use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_storage(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/store" => {
                let key_expr = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let key_local = self.local_idx("__store_key");
                let mut v = Vec::new();
                // Evaluate key once and save to local (key expression may have side effects)
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // Store tagged val at mem[STORAGE_BUF] — preserves type through storage round-trip
                v.push(Instruction::I32Const(STORAGE_BUF as i32)); v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=STORAGE_BUF, register_id=0) — idx 17
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // raw >> 32 = key_len
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // raw & 0xFFFF_FFFF = key_ptr
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/load" => {
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__load_key");
                let mut v = Vec::new();
                // Evaluate key once and save to local (key expression may have side effects)
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // storage_read(key_len, key_ptr, register_id=1) — idx 18
                // Use return value (0=not found, 1=found) instead of register_len,
                // because register_len doesn't clear stale data from previous reads.
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1)); // register 1
                v.push(Self::host_call(18));
                // Check return value: 0 = not found, nonzero = found
                v.push(Instruction::I64Const(0));
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
                v.push(Instruction::I64Const(1)); // register 1 (not 0, to avoid stale data)
                v.push(Self::host_call(18));
                // Check return value: 0 = not found, 1 = found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                    v.push(Instruction::I64Const(1));
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
            // near/store_num: (near/store_num key_i64 val_i64) — key is raw i64, 8 LE bytes
            // Stores TAGGED val under 8-byte LE key. Gas-efficient numeric keys.
            "near/store_num" => {
                self.need_host(17); self.need_host(0);
                let key_expr = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let key_local = self.local_idx("__sn_key");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // Store tagged val at STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32)); v.extend(val);
                v.push(Instruction::I64Store(ma));
                // Write un-tagged key as 8 LE bytes at TEMP_MEM
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(ma));
                // storage_write(key_len=8, key_ptr=TEMP_MEM, val_len=8, val_ptr=STORAGE_BUF, register_id=0)
                v.push(Instruction::I64Const(8)); // key_len
                v.push(Instruction::I64Const(TEMP_MEM)); // key_ptr
                v.push(Instruction::I64Const(8)); // val_len
                v.push(Instruction::I64Const(STORAGE_BUF)); // val_ptr
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            // near/load_num: (near/load_num key_i64) — key is raw i64, returns tagged value or 0
            "near/load_num" => {
                self.need_host(18); self.need_host(0);
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__ln_key");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // Write un-tagged key as 8 LE bytes at TEMP_MEM
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(ma));
                // storage_read(key_len=8, key_ptr=TEMP_MEM, register_id=1) → returns 0 if not found, 1 if found
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                // Check return value: 0 = not found, 1 = found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return tagged 0
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // Key found: read_register(1, STORAGE_BUF)
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(ma));
                // Tag validation: trap if loaded value has invalid tag bits (7)
                v.extend(self.emit_tag_validate());
                v.push(Instruction::End);
                Ok(v)
            }
            "near/storage_usage" => { let mut v = vec![Self::host_call(11)]; v.extend(self.emit_tag_num()); Ok(v) },
            // near/store_u128: (near/store_u128 "key" tagged_ptr) -> nil
            // High-level u128 storage - takes tagged pointer, stores 16 bytes to NEAR storage
            "near/store_u128" => {
                self.need_host(17);
                if a.len() != 2 { return Err("near/store_u128: need 2 args (key, tagged_ptr)".into()); }
                let key_expr = self.expr(&a[0])?;
                let ptr_expr = self.expr(&a[1])?;
                let ptr_local = self.local_idx("__u128_ptr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Save ptr to local
                v.extend(ptr_expr);
                v.push(Instruction::LocalSet(ptr_local));
                // Copy u128 from ptr to STORAGE_U128_BUF
                // STORAGE_U128_BUF[0..8] = ptr[0..8] (low)
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::LocalGet(ptr_local)); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma.clone()));
                v.push(Instruction::I64Store(ma.clone()));
                // STORAGE_U128_BUF[8..16] = ptr[8..16] (high)
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::LocalGet(ptr_local)); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(ma));
                // storage_write(key_len, key_ptr, 16, STORAGE_U128_BUF, register=0)
                v.extend(key_expr.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key_expr); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Const(STORAGE_U128_BUF as i64));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            // near/load_u128: (near/load_u128 "key") -> tagged_ptr
            // High-level u128 load - reads 16 bytes from storage, returns tagged pointer to TEMP_MEM
            "near/load_u128" => {
                self.need_host(18);
                self.need_host(0);
                if a.len() != 1 { return Err("near/load_u128: need 1 arg (key)".into()); }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                let found = self.local_idx("__found");
                // storage_read(key_len, key_ptr, register=1)
                v.extend(key_expr.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key_expr); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                v.push(Instruction::LocalSet(found));
                // Check if found
                v.push(Instruction::LocalGet(found));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return tagged 0
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // Found: read_register(1, STORAGE_U128_BUF)
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(STORAGE_U128_BUF as i64));
                v.push(Self::host_call(0)); // read_register returns nothing
                // Copy from STORAGE_U128_BUF to TEMP_MEM
                // I64Store type: [i32 addr, i64 value] -> [] (value on TOP)
                // TEMP_MEM[0..8] = STORAGE_U128_BUF[0..8]
                v.push(Instruction::I32Const(TEMP_MEM as i32));  // dest addr (BOTTOM)
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));  // src addr
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Stack: [dest, value] - dest at bottom, value on top
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // TEMP_MEM[8..16] = STORAGE_U128_BUF[8..16]
                v.push(Instruction::I32Const((TEMP_MEM + 8) as i32));  // dest addr
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));  // src addr
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Return TEMP_MEM as tagged pointer
                v.push(Instruction::I64Const(TEMP_MEM));
                v.extend(self.emit_tag_num());
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into())
        }
    }
}
