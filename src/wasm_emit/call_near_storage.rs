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
            // near/kstore: (near/kstore prefix account val) — FP_GLOBAL-safe storage via KEY_BUF
            // Concatenates prefix + account as key, stores value (tagged i64).
            "near/kstore" => {
                if a.len() != 3 { return Err("near/kstore requires 3 args: prefix account value".into()); }
                self.need_host(17); self.need_host(0); self.need_host(1);
                let prefix_expr = self.expr(&a[0])?;
                let acct_expr = self.expr(&a[1])?;
                let val_expr = self.expr(&a[2])?;
                let prefix_local = self.local_idx("__kstore_prefix");
                let acct_local = self.local_idx("__kstore_acct");
                let key_len_local = self.local_idx_i32("__kstore_keylen");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate prefix, account, value and save to locals
                v.extend(prefix_expr);
                v.push(Instruction::LocalSet(prefix_local));
                v.extend(acct_expr);
                v.push(Instruction::LocalSet(acct_local));
                // Store tagged val at STORAGE_BUF
                // I64Store: [addr (i32), value (i64)] - push addr FIRST, then value
                v.push(Instruction::I32Const(STORAGE_BUF as i32)); // addr (i32) - pushed FIRST
                v.extend(val_expr); // value (i64) - pushed SECOND
                v.push(Instruction::I64Store(ma));
                
                // Copy prefix to KEY_BUF
                // MemoryCopy: dst (i32), src (i32), len (i32)
                v.push(Instruction::I32Const(KEY_BUF as i32)); // dst
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = prefix ptr
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // prefix_len (i64)
                v.push(Instruction::I32WrapI64); // len = prefix_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // key_len = prefix_len (save for later)
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(key_len_local));
                
                // Copy account to KEY_BUF + prefix_len
                v.push(Instruction::I32Const(KEY_BUF as i32));
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::I32Add); // dst = KEY_BUF + prefix_len
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = account ptr
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64); // len = account_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // key_len = prefix_len + account_len
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalSet(key_len_local));
                
                // storage_write(key_len, KEY_BUF, val_len=8, STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(key_len_local));
                v.extend(self.emit_i32_to_i64());
                v.push(Instruction::I64Const(KEY_BUF as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            // near/kload: (near/kload prefix account) — FP_GLOBAL-safe storage load
            // Returns tagged value or 0 (tagged as Num) if not found.
            "near/kload" => {
                if a.len() != 2 { return Err("near/kload requires 2 args: prefix account".into()); }
                let prefix_expr = self.expr(&a[0])?;
                let acct_expr = self.expr(&a[1])?;
                let prefix_local = self.local_idx("__kload_prefix");
                let acct_local = self.local_idx("__kload_acct");
                let key_len_local = self.local_idx_i32("__kload_keylen");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate prefix and account, save to locals
                v.extend(prefix_expr);
                v.push(Instruction::LocalSet(prefix_local));
                v.extend(acct_expr);
                v.push(Instruction::LocalSet(acct_local));
                
                // Copy prefix to KEY_BUF
                // MemoryCopy: dst (i32), src (i32), len (i32)
                v.push(Instruction::I32Const(KEY_BUF as i32)); // dst
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = prefix ptr
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // prefix_len (i64)
                v.push(Instruction::I32WrapI64); // len = prefix_len (i32)
                v.push(Instruction::LocalTee(key_len_local)); // save for later AND keep on stack
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // Copy account to KEY_BUF + prefix_len
                v.push(Instruction::I32Const(KEY_BUF as i32));
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::I32Add); // dst = KEY_BUF + prefix_len
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = account ptr
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64); // len = account_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // Compute total key_len = prefix_len + account_len
                v.push(Instruction::LocalGet(key_len_local)); // prefix_len (i32)
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalSet(key_len_local)); // key_len = prefix_len + account_len
                
                // storage_read(key_len, KEY_BUF, register=1) → returns 0 if not found, 1 if found
                v.push(Instruction::LocalGet(key_len_local));
                v.extend(self.emit_i32_to_i64()); // host expects i64
                v.push(Instruction::I64Const(KEY_BUF as i64));
                v.push(Instruction::I64Const(1)); // register 1
                v.push(Self::host_call(18)); // storage_read
                
                // Check return value: 0 = not found, 1 = found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return tagged 0
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // Found: read_register(1, STORAGE_BUF)
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_validate());
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
