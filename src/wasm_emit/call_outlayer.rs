use super::*;

impl WasmEmitter {
    pub(crate) fn call_outlayer(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
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
            _ => Err("__not_handled__".into()),
        }
    }
}
