use super::*;

impl WasmEmitter {
    pub(crate) fn call_json(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
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
                        let mut v = if a.len() > 1 {
                            // (json-get-str "key" buffer) — scan directly from buffer, zero-copy
                            // The tagged string payload IS the packed (len<<32|ptr) that __json_get expects
                            let mut setup = Vec::new();
                            setup.extend(self.expr(&a[1])?);
                            setup.push(Instruction::I64Const(3)); setup.push(Instruction::I64ShrU);
                            // Stack: payload = (len << 32) | ptr — pass directly to __json_get
                            let pat = {
                                let mut p = vec![b'"'];
                                p.extend(key.as_bytes());
                                p.extend_from_slice(b"\":");
                                p
                            };
                            let pat_off = self.alloc_data(&pat) as i64;
                            let pat_len = pat.len() as i64;
                            let pat_packed = (pat_off as u64) | ((pat_len as u64) << 32);
                            setup.push(Instruction::I64Const(pat_packed as i64));
                            let jg_idx = self.ensure_json_get_func();
                            setup.push(Instruction::Call(crate::wasm_emit::USER_BASE | jg_idx));
                            setup
                        } else if self.wasi_mode {
                            self.json_get_wasi(key, "str")?
                        } else {
                            self.json_get_with_scanner(key, "str")?
                        };
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
            "json-extract" => {
                // (json-extract json_str "key0" "key1" ...) → tagged array of values
                if a.len() < 3 { return Err("json-extract requires a JSON string and at least 2 keys".into()); }
                let n_keys = a.len() - 1;
                if n_keys > 8 { return Err("json-extract supports at most 8 keys".into()); }
                // Build pattern data for each key: "key":
                let mut v = Vec::new();
                // Evaluate the JSON string argument
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); // untag string
                // Push each key pattern as packed (len << 32) | ptr
                for key_arg in &a[1..] {
                    match key_arg {
                        LispVal::Str(key) => {
                            let pat = {
                                let mut p = vec![b'"'];
                                p.extend(key.as_bytes());
                                p.extend_from_slice(b"\":");
                                p
                            };
                            let pat_off = self.alloc_data(&pat) as i64;
                            let pat_len = pat.len() as i64;
                            let pat_packed = (pat_off as u64) | ((pat_len as u64) << 32);
                            v.push(Instruction::I64Const(pat_packed as i64));
                        }
                        _ => return Err("json-extract keys must be string literals".into()),
                    }
                }
                // Call __json_extract_N
                let idx = self.ensure_json_extract_func(n_keys);
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                // Result is already tagged as array (TAG_ARRAY)
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
