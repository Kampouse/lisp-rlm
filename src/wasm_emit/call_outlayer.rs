use super::*;

impl WasmEmitter {
    pub(crate) fn call_outlayer(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "http-get" => {
                // (http-get "https://api.example.com/data") -> string or nil
                // Always uses the combined P2 core __wasi_http_get (sentinel 103)
                // with 5-param convention: (url_ptr, url_len, buf_ptr, buf_len, len_ptr)
                if a.is_empty() {
                    return Err("http-get requires a URL string argument".into());
                }
                if !self.wasi_mode {
                    return Err("http-get is only available on OutLayer (WASI) target".into());
                }

                let url_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE;
                let mut v = Vec::new();

                // Register URL for data segment generation if it's a string literal
                // and determine per-call sentinel index
                let mut url_idx: u32 = 0;
                if let crate::types::LispVal::Str(url) = &a[0] {
                    if let Some((auth, path)) = Self::split_url(url) {
                        let existing = self
                            .http_urls
                            .iter()
                            .position(|(a, p)| a == &auth && p == &path);
                        match existing {
                            Some(idx) => url_idx = idx as u32,
                            None => {
                                url_idx = self.http_urls.len() as u32;
                                self.http_urls.push((auth, path));
                            }
                        }
                    }
                } else {
                    // Non-literal URL — still register a sentinel entry
                    url_idx = self.http_urls.len() as u32;
                    self.http_urls.push(("dynamic".into(), "/".into()));
                }

                // 5-param convention for combined P2 core __wasi_http_get
                let buf_ptr: i32 = crate::wasi_http::SENTINEL_BUF;
                let buf_len: i32 = crate::wasi_http::SENTINEL_BUF_SIZE;

                // Push url_ptr, url_len (ignored by data-segment path, but needed for param count)
                v.extend(url_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // url_ptr
                v.extend(url_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // url_len
                                                 // buf_ptr, buf_len
                v.push(Instruction::I32Const(buf_ptr));
                v.push(Instruction::I32Const(buf_len));
                // len_ptr = ret_area+8
                v.push(Instruction::I32Const(ret_area + 8));
                // Call http-get (sentinel 103+url_idx) — returns i32 (status), drop it
                v.push(Instruction::Call(103 + url_idx));
                v.push(Instruction::Drop);
                // Write disc=0 (ok) at ret_area+0
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Store(ma4));
                // Write buf_ptr at ret_area+4
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Const(buf_ptr));
                v.push(Instruction::I32Store(ma4));
                // len is already at ret_area+8 (written by http_get)
                // Read result: build tagged string from ptr + len
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 8));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                // Tag: ((ptr | (len << 32)) << 3) | TAG_STR
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "http-post" => {
                // (http-post "https://api.example.com/data" "body") -> string or nil
                if a.len() < 2 {
                    return Err("http-post requires (url body)".into());
                }
                if !self.wasi_mode {
                    return Err("http-post is only available on OutLayer (WASI) target".into());
                }

                let url_expr = self.expr(&a[0])?;
                let body_expr = self.expr(&a[1])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE;
                let content_type_area: i32 = 163900;
                // Store default content-type "application/json" as data segment
                self.data_segments
                    .push((content_type_area as u32, b"application/json".to_vec()));
                let content_type_len: i32 = 16; // "application/json".len()
                let mut v = Vec::new();
                let mut sentinel_idx: u32 = 0; // index into http_post_urls for sentinel mapping

                // Register URL for data segment generation if it's a string literal
                if self.need_wasi_http {
                    if let crate::types::LispVal::Str(url) = &a[0] {
                        if let Some((auth, path)) = Self::split_url(url) {
                            if let Some(existing) = self
                                .http_post_urls
                                .iter()
                                .position(|(a, p)| a == &auth && p == &path)
                            {
                                // URL already registered — reuse its index
                                sentinel_idx = existing as u32;
                            } else {
                                sentinel_idx = self.http_post_urls.len() as u32;
                                self.http_post_urls.push((auth, path));
                            }
                        }
                    } else {
                        // Non-literal URL (variable) — still need a sentinel entry
                        // so build_combined_p2_core generates the shim function
                        sentinel_idx = self.http_post_urls.len() as u32;
                        self.http_post_urls
                            .push(("_dynamic".to_string(), "/".to_string()));
                    }
                }

                if self.need_wasi_http {
                    // Direct wasi:http POST path — 7-param convention:
                    // (url_ptr, url_len, body_ptr, body_len, buf_ptr, buf_len, len_ptr) -> i32
                    // http_post ignores url_ptr/url_len (uses data segments),
                    // reads body from body_ptr/body_len,
                    // writes response to buf_ptr, length to *len_ptr.
                    let buf_ptr: i32 = crate::wasi_http::SENTINEL_BUF;
                    let buf_len: i32 = crate::wasi_http::SENTINEL_BUF_SIZE;
                    // Sentinel based on URL index, not call count
                    let post_sentinel: u32 = 200 + sentinel_idx;
                    // url ptr/len (ignored by data-segment path)
                    v.extend(url_expr.clone());
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.extend(url_expr);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64);
                    // body ptr/len
                    v.extend(body_expr.clone());
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.extend(body_expr);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64);
                    // response buffer: buf_ptr, buf_len
                    v.push(Instruction::I32Const(buf_ptr));
                    v.push(Instruction::I32Const(buf_len));
                    // len_ptr = ret_area+8 (length will be written here directly)
                    v.push(Instruction::I32Const(ret_area + 8));
                    // Call http-post (sentinel 200+call_idx for wasi:http path) — returns i32 (status), drop it
                    v.push(Instruction::Call(post_sentinel));
                    v.push(Instruction::Drop);
                    // Write disc=0 (ok) at ret_area+0
                    v.push(Instruction::I32Const(ret_area));
                    v.push(Instruction::I32Const(0));
                    v.push(Instruction::I32Store(ma4));
                    // Write buf_ptr at ret_area+4
                    v.push(Instruction::I32Const(ret_area + 4));
                    v.push(Instruction::I32Const(buf_ptr));
                    v.push(Instruction::I32Store(ma4));
                    // len is already at ret_area+8 (written by http_post)
                    // Read result: ptr from ret_area+4, len from ret_area+8
                    v.push(Instruction::I32Const(ret_area + 4));
                    v.push(Instruction::I32Load(ma4));
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::I32Const(ret_area + 8));
                    v.push(Instruction::I32Load(ma4));
                    v.push(Instruction::I64ExtendI32U);
                    // Tag: ((ptr | (len << 32)) << 3) | TAG_STR
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(TAG_STR));
                    v.push(Instruction::I64Or);
                } else {
                    // OutLayer host function path
                    self.need_outlayer = true;
                    // url ptr/len
                    v.extend(url_expr.clone());
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.extend(url_expr);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64);
                    // body ptr/len
                    v.extend(body_expr.clone());
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.extend(body_expr);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64);
                    // content_type ptr/len
                    v.push(Instruction::I32Const(content_type_area));
                    v.push(Instruction::I32Const(content_type_len));
                    // ret_area
                    v.push(Instruction::I32Const(ret_area));
                    // Call http-post (sentinel 104) — canonical ABI with 7 params
                    v.push(Instruction::Call(104));
                    // Read result (same pattern as http-get)
                    v.push(Instruction::I32Const(ret_area));
                    v.push(Instruction::I32Load(ma4));
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Ne);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(TAG_NIL));
                    v.push(Instruction::Else);
                    v.push(Instruction::I32Const(ret_area + 4));
                    v.push(Instruction::I32Load(ma4));
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::I32Const(ret_area + 8));
                    v.push(Instruction::I32Load(ma4));
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(TAG_STR));
                    v.push(Instruction::I64Or);
                    v.push(Instruction::End);
                }
                Ok(v)
            }
            "storage-set" => {
                // (storage-set "key" "value") -> bool
                // Production WIT: set(key: string, value: list<u8>) -> string
                // Canonical ABI: (kp, kl, vp, vl, ret_ptr) -> () — 5 params
                // ret_ptr layout: +0: str_ptr, +4: str_len (success response string)
                if a.len() < 2 {
                    return Err("storage-set requires (key value)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-set is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 64;
                // Use locals to evaluate key/val ONCE — avoids double-execution of str-cat
                let key_local = self.local_idx("__ss_key");
                let val_local = self.local_idx("__ss_val");
                let mut v = Vec::new();
                // Evaluate key → local
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // Evaluate val → local
                v.extend(val_expr);
                v.push(Instruction::LocalSet(val_local));
                // key ptr/len from local
                v.push(Instruction::LocalGet(key_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(key_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // val ptr/len from local
                v.push(Instruction::LocalGet(val_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // ret_ptr for string return
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(110));
                // Production returns string via ret_ptr — just return true (success)
                v.push(Instruction::I64Const(1));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get" => {
                // (storage-get "key") -> string or nil
                // Production WIT: get(key: string) -> tuple<list<u8>, string>
                // Canonical ABI: (kp, kl, ret_ptr) -> () — 3 params
                // ret_ptr layout: +0: list_ptr, +4: list_len, +8: str_ptr, +12: str_len
                // Each call gets a unique ret_area to avoid overwriting previous results
                if a.is_empty() {
                    return Err("storage-get requires a key".into());
                }
                if !self.wasi_mode {
                    return Err("storage-get is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let call_idx = self.storage_get_count;
                self.storage_get_count += 1;
                // Each storage-get gets its own 16-byte ret_area: base=163968, stride=16
                let ret_area: i32 =
                    crate::wasi_http::OL_RET_AREA_BASE + 128 + (call_idx as i32) * 16;
                // Use local to evaluate key ONCE — avoids double-execution of str-cat
                let key_local = self.local_idx("__sg_key");
                let mut v = Vec::new();
                // Evaluate key → local
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // key ptr/len from local
                v.push(Instruction::LocalGet(key_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(key_local));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // ret_ptr
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(111));
                // IMMEDIATELY copy list data to safe buffer to prevent corruption
                // Safe buffer starts at 163968 + 4096 = 168064, each get gets 256 bytes
                let safe_buf: i32 = 168064 + (call_idx as i32) * 256;
                // Read list_ptr and list_len from ret_area
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // list_len == 0 → not found → TAG_NIL
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // list_len > 0 → copy data to safe buffer, then construct tagged string
                // memory.copy(dst=safe_buf, src=list_ptr, len=list_len)
                v.push(Instruction::I32Const(safe_buf)); // dst
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4)); // src = list_ptr
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4)); // len = list_len
                v.push(Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });
                // Construct tagged string pointing to safe buffer
                v.push(Instruction::I32Const(safe_buf));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-has" => {
                // (storage-has "key") -> bool
                // Production WIT: has(key: string) -> bool
                // Canonical ABI: (kp, kl) -> i32 — 2 params, direct i32 return (NO ret_ptr)
                if a.is_empty() {
                    return Err("storage-has requires a key".into());
                }
                if !self.wasi_mode {
                    return Err("storage-has is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // bool returned directly as i32 — no ret_ptr
                v.push(Instruction::Call(112));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-delete" => {
                // (storage-delete "key") -> bool
                // Production WIT: delete(key: string) -> bool
                // Canonical ABI: (kp, kl) -> i32 — 2 params, direct i32 return (NO ret_ptr)
                if a.is_empty() {
                    return Err("storage-delete requires a key".into());
                }
                if !self.wasi_mode {
                    return Err("storage-delete is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // bool returned directly as i32 — no ret_ptr
                v.push(Instruction::Call(113));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-increment" => {
                // (storage-increment "key" delta) -> i64 (new value)
                // Canonical ABI: (i32 key_ptr, i32 key_len, i64 delta, i32 ret_area) -> ()
                // result<s64, string> ret_area layout: [i32 disc] [4 pad] [i64 s64 @ +8]
                if a.len() < 2 {
                    return Err("storage-increment requires (key delta)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-increment is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let delta_expr = self.expr(&a[1])?;
                let ma8 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                // key ptr/len (2 × i32) — extract from tagged pair
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // key_ptr (i32)
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // key_len (i32)
                                                 // delta (i64) — untag, keep as i64 for canonical ABI
                v.extend(delta_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag → i64
                                              // ret_area pointer (i32)
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE)); // OL_RET_AREA
                v.push(Instruction::Call(114));
                // Read s64 result from ret_area + 0 (tuple<s64, string>: s64 @ 0, string @ +8)
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::I64Load(ma8));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "env/signer" => {
                // WIT: env-signer() -> string
                // Canonical ABI: (ret_ptr) -> ()
                // Result: (ptr, len) written by host to ret_area
                if !self.wasi_mode {
                    return Err("env/signer is only available on OutLayer".into());
                }
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 320;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(120));
                // Read ptr/len from ret_area
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "env/predecessor" => {
                // WIT: env-predecessor() -> string
                // Canonical ABI: (ret_ptr) -> ()
                // Result: (ptr, len) written by host to ret_area
                if !self.wasi_mode {
                    return Err("env/predecessor is only available on OutLayer".into());
                }
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 384;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(121));
                // Read ptr/len from ret_area
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "storage-decrement" => {
                // (storage-decrement "key" delta) -> i64 (new value)
                // Canonical ABI: (i32 key_ptr, i32 key_len, i64 delta, i32 ret_area) -> ()
                // result<s64, string> ret_area layout: [i32 disc] [4 pad] [i64 s64 @ +8]
                if a.len() < 2 {
                    return Err("storage-decrement requires (key delta)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-decrement is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let delta_expr = self.expr(&a[1])?;
                let ma8 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                // key ptr/len (2 × i32) — extract from tagged pair
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // key_ptr (i32)
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // key_len (i32)
                                                 // delta (i64) — untag, keep as i64 for canonical ABI
                v.extend(delta_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag → i64
                                              // ret_area pointer (i32)
                v.push(Instruction::I32Const(
                    crate::wasi_http::OL_RET_AREA_BASE + 384,
                )); // separate from increment's crate::wasi_http::OL_RET_AREA_BASE
                v.push(Instruction::Call(130));
                // Read s64 result from ret_area + 0 (tuple<s64, string>: s64 @ 0)
                v.push(Instruction::I32Const(
                    crate::wasi_http::OL_RET_AREA_BASE + 384,
                ));
                v.push(Instruction::I64Load(ma8));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-if-absent" => {
                // (storage-set-if-absent "key" "value") -> bool (true = was inserted)
                if a.len() < 2 {
                    return Err("storage-set-if-absent requires (key value)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-set-if-absent is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::Call(131));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-if-equals" => {
                // (storage-set-if-equals "key" "expected" "new") -> bool
                if a.len() < 3 {
                    return Err("storage-set-if-equals requires (key expected new)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-set-if-equals is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let exp_expr = self.expr(&a[1])?;
                let new_expr = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(exp_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(exp_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(new_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(new_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // old_buf at 98304, old_len_ptr at crate::wasi_http::OL_RET_AREA_BASE
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::Call(132));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-list-keys" => {
                // (storage-list-keys "prefix") -> string or nil
                if a.is_empty() {
                    return Err("storage-list-keys requires a prefix".into());
                }
                if !self.wasi_mode {
                    return Err("storage-list-keys is only available on OutLayer".into());
                }
                let prefix_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ma1 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                };
                let len_l = self.local_idx("__sg_lklen");
                let dst_l = self.local_idx("__sg_lkdst");
                let i_l = self.local_idx("__sg_lki");
                let mut v = Vec::new();
                v.extend(prefix_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(prefix_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::Call(133));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr_i32() as i64));
                v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                self.heap_bump(65536);
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-clear-all" => {
                // (storage-clear-all) -> bool
                if !self.wasi_mode {
                    return Err("storage-clear-all is only available on OutLayer".into());
                }
                let mut v = Vec::new();
                v.push(Instruction::Call(134));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-set-worker" => {
                // (storage-set-worker "key" "value") -> bool
                // near:storage/api set-worker(key, value, is_encrypted: option<bool>) -> string
                // canonical ABI: key_ptr, key_len, val_ptr, val_len, opt_disc, opt_val, ret_ptr = 7 i32
                if a.len() < 2 {
                    return Err("storage-set-worker requires (key value)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-set-worker is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // option<bool>: disc=0 (none), val=0
                v.push(Instruction::I32Const(0)); // none discriminant
                v.push(Instruction::I32Const(0)); // bool value (unused when none)
                                                  // ret_ptr for return string
                v.push(Instruction::I32Const(131296)); // SCRATCH_READ_RESULT
                v.push(Instruction::Call(135));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get-worker" => {
                // (storage-get-worker "key") -> string or nil
                if a.is_empty() {
                    return Err("storage-get-worker requires a key".into());
                }
                if !self.wasi_mode {
                    return Err("storage-get-worker is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::Call(136));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ma1 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                };
                let len_l = self.local_idx("__sg_wlen");
                let dst_l = self.local_idx("__sg_wdst");
                let i_l = self.local_idx("__sg_wi");
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr_i32() as i64));
                v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                self.heap_bump(65536);
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-set-worker-public" => {
                // (storage-set-worker-public "key" "value") -> bool
                // near:storage/api set-worker-public(key, value, is_encrypted: option<bool>) -> string
                // canonical ABI: key_ptr, key_len, val_ptr, val_len, opt_disc, opt_val, ret_ptr = 7 i32
                if a.len() < 2 {
                    return Err("storage-set-worker-public requires (key value)".into());
                }
                if !self.wasi_mode {
                    return Err("storage-set-worker-public is only available on OutLayer".into());
                }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // option<bool>: disc=0 (none), val=0
                v.push(Instruction::I32Const(0)); // none discriminant
                v.push(Instruction::I32Const(0)); // bool value (unused when none)
                                                  // ret_ptr for return string
                v.push(Instruction::I32Const(131296)); // SCRATCH_READ_RESULT
                v.push(Instruction::Call(137));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get-worker-from-project" => {
                // (storage-get-worker-from-project "key" "project_uuid") -> string or nil
                if a.len() < 2 {
                    return Err(
                        "storage-get-worker-from-project requires (key project_uuid)".into(),
                    );
                }
                if !self.wasi_mode {
                    return Err(
                        "storage-get-worker-from-project is only available on OutLayer".into(),
                    );
                }
                let key_expr = self.expr(&a[0])?;
                let proj_expr = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.extend(proj_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(proj_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::I32Const(65536));
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::Call(138));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ma1 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                };
                let len_l = self.local_idx("__sg_cplen");
                let dst_l = self.local_idx("__sg_cpdst");
                let i_l = self.local_idx("__sg_cpi");
                v.push(Instruction::I32Const(crate::wasi_http::OL_RET_AREA_BASE));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(len_l));
                v.push(Instruction::I64Const(self.heap_ptr_i32() as i64));
                v.push(Instruction::LocalSet(dst_l));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(98304));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I32Store8(ma1));
                v.push(Instruction::LocalGet(i_l));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_l));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                self.heap_bump(65536);
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "outlayer/view" => {
                // (outlayer/view contract method args) -> string or nil
                // near:rpc/api view(contract-id, method-name, args-json, finality-or-block) -> tuple<string, string>
                // Canonical ABI: 8 i32 (4 strings) + ret_area = 9 params, void return
                // ret_area layout: +0: result_ptr, +4: result_len, +8: error_ptr, +12: error_len
                if a.len() < 3 {
                    return Err("outlayer/view requires (contract method args)".into());
                }
                let contract = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args_val = self.expr(&a[2])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 448; // separate from storage ret areas
                let mut v = Vec::new();

                // Push 9 i32 params for near:rpc/api view
                // contract ptr/len
                v.extend(contract.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(contract);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // method ptr/len
                v.extend(method.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(method);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // args ptr/len
                v.extend(args_val.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(args_val);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // finality-or-block: empty string (default = "final")
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0));
                // ret_area
                v.push(Instruction::I32Const(ret_area));
                // call view (sentinel 100) — void return
                v.push(Instruction::Call(100));
                // Read tuple<string, string> from ret_area:
                // +0: result_ptr, +4: result_len, +8: error_ptr, +12: error_len
                v.push(Instruction::I32Const(ret_area + 12));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL)); // error → nil
                v.push(Instruction::Else);
                // Read result: ptr from +0, len from +4
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "outlayer/raw" => {
                // (outlayer/raw method params) -> string result
                // near:rpc/api raw(method: string, params-json: string) -> tuple<string, string>
                // Uses sentinel 140 mapped to near:rpc/api "raw" import
                // Canonical ABI: 4 i32 (2 strings) + ret_area = 5 params
                // ret_area layout: +0: result_ptr, +4: result_len, +8: error_ptr, +12: error_len
                if a.len() < 2 {
                    return Err("outlayer/raw requires (method params)".into());
                }
                let method = self.expr(&a[0])?;
                let params = self.expr(&a[1])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 512;
                let mut v = Vec::new();

                // method ptr/len
                v.extend(method.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(method);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // params ptr/len
                v.extend(params.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(params);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // ret_area
                v.push(Instruction::I32Const(ret_area));
                // call raw (sentinel 140)
                v.push(Instruction::Call(140));
                // Read tuple<string, string>: check error_len @ +12
                v.push(Instruction::I32Const(ret_area + 12));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Read result: ptr from +0, len from +4
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "outlayer/status" => {
                // (outlayer/status) -> string
                // near:rpc/api view with empty contract, method="status", args=""
                // Uses sentinel 100 (view) with split interface canonical ABI
                let mut v = Vec::new();
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 448;
                // Store "status" at a known offset
                let status_str = b"status";
                let status_offset = self.heap_bump(64);
                for (j, &byte) in status_str.iter().enumerate() {
                    self.data_segments
                        .push((status_offset + j as u32, vec![byte]));
                }
                // view(contract_ptr, contract_len, method_ptr, method_len, args_ptr, args_len, finality_ptr, finality_len, ret_area)
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0)); // empty contract
                v.push(Instruction::I32Const(status_offset as i32));
                v.push(Instruction::I32Const(6)); // "status"
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0)); // empty args
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0)); // empty finality
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(100)); // near:rpc/api view
                                                // Read tuple<string, string>: check error_len @ +12
                v.push(Instruction::I32Const(ret_area + 12));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Read result: ptr from +0, len from +4
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "outlayer/storage-set" => {
                // (outlayer/storage-set key value) -> nil
                // Delegates to storage-set (sentinel 110) via split near:storage/api
                if a.len() < 2 {
                    return Err("outlayer/storage-set requires (key value)".into());
                }
                let key = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 64;
                let mut v = Vec::new();
                // key ptr/len
                v.extend(key.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // val ptr/len
                v.extend(val_expr.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(val_expr);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // ret_ptr
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(110)); // near:storage/api set
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "outlayer/storage-get" => {
                // (outlayer/storage-get key) -> string or nil
                // Delegates to storage-get (sentinel 111) via split near:storage/api
                if a.is_empty() {
                    return Err("outlayer/storage-get requires (key)".into());
                }
                let key = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 128;
                let mut v = Vec::new();
                // key ptr/len
                v.extend(key.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // ret_ptr
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(111)); // near:storage/api get
                                                // Read tuple<list<u8>, string>: check list_len @ +4
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "outlayer/storage-has" | "outlayer/storage-delete" => {
                Ok(vec![Instruction::I64Const(TAG_NIL)])
            }
            "outlayer/context" => {
                // (outlayer/context "signer_id") -> string
                // Uses env-signer (sentinel 119) from outlayer:api/host
                if a.is_empty() {
                    return Err("outlayer/context requires a key string".into());
                }
                let _key = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 576;
                let mut v = Vec::new();
                // env-signer() -> string: (ret_area) -> ()
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(120)); // env-signer (sentinel 120)
                                                // Read string from ret_area: ptr @ +0, len @ +4
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            // ── P2 inline operations (no host calls, pure WASM) ──
            "outlayer/http-post" => {
                // (outlayer/http-post "url" "body" ["content-type"]) -> string or nil
                // Uses wasi:http POST path (same as http-post kebab form)
                if a.len() < 2 {
                    return Err(
                        "outlayer/http-post requires (url body) or (url body content-type)".into(),
                    );
                }
                if !self.wasi_mode {
                    return Err(
                        "outlayer/http-post is only available on OutLayer (WASI) target".into(),
                    );
                }

                // Register URL for wasi:http POST helper generation
                // (Note: delegation to http-post below will also register, but dedup handles it)
                if self.need_wasi_http {
                    if let crate::types::LispVal::Str(url) = &a[0] {
                        if let Some((auth, path)) = Self::split_url(url) {
                            if !self
                                .http_post_urls
                                .iter()
                                .any(|(a, p)| a == &auth && p == &path)
                            {
                                self.http_post_urls.push((auth, path));
                            }
                        }
                    }
                }

                // Delegate to http-post kebab form (same logic)
                return self.call_outlayer("http-post", a);
            }
            "outlayer/json-get" => {
                // (outlayer/json-get tagged_string "key") -> string or nil
                // Scans JSON from a tagged string (e.g., HTTP response) for the given key
                if a.len() < 2 {
                    return Err("outlayer/json-get requires (string key)".into());
                }
                if !self.wasi_mode {
                    return Err(
                        "outlayer/json-get is only available on OutLayer (WASI) target".into(),
                    );
                }
                match &a[1] {
                    crate::types::LispVal::Str(key) => {
                        let buf_expr = self.expr(&a[0])?;
                        let tmp_l = self.local_idx("__ojg_tmp");
                        let src_ptr_l = self.local_idx("__ojg_sp");
                        let copy_i = self.local_idx("__ojg_ci");
                        let target_buf = 98304i64; // response buffer
                        let ma1 = wasm_encoder::MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        };

                        let mut setup = Vec::new();
                        // Untag the source string: >>3 gives payload = (len << 32) | ptr
                        setup.extend(buf_expr);
                        setup.push(Instruction::I64Const(3));
                        setup.push(Instruction::I64ShrU);
                        setup.push(Instruction::LocalSet(tmp_l));
                        // Extract src_ptr = tmp & 0xFFFFFFFF
                        setup.push(Instruction::LocalGet(tmp_l));
                        setup.push(Instruction::I64Const(0xFFFFFFFF));
                        setup.push(Instruction::I64And);
                        setup.push(Instruction::LocalSet(src_ptr_l));
                        // Extract len = tmp >> 32
                        setup.push(Instruction::LocalGet(tmp_l));
                        setup.push(Instruction::I64Const(32));
                        setup.push(Instruction::I64ShrU);
                        // Copy string to target_buf so json_get_from_buf can scan it
                        // Save len to another local for the copy
                        let src_len_l = self.local_idx("__ojg_slen");
                        setup.push(Instruction::LocalSet(src_len_l));
                        // Copy loop: target_buf[i] = src_ptr[i] for i in 0..len
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::LocalGet(src_len_l));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::BrIf(1));
                        // target_buf[i] = src[i]
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(src_ptr_l));
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Add);
                        setup.push(Instruction::I32Load8U(ma1));
                        setup.push(Instruction::I32Store8(ma1));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        // Push len for buf_len_setup
                        setup.push(Instruction::LocalGet(src_len_l));

                        let mut v = self.json_get_from_buf(key, "str", target_buf, &mut setup)?;
                        v.extend(self.emit_tag_str());
                        Ok(v)
                    }
                    _ => Err("outlayer/json-get key must be a string literal".into()),
                }
            }
            "outlayer/str-concat" | "outlayer/str-cat" => {
                // Delegate to str-concat (handles P2/WASI tagged strings)
                if a.len() < 2 {
                    return Err("outlayer/str-concat requires at least 2 args".into());
                }
                self.call_string("str-cat", a)
            }
            "outlayer/call" => {
                // (outlayer/call receiver method args deposit gas) -> string or nil
                // Uses near:rpc/api call() — host signs+broadcasts transaction.
                // signer_id/signer_key: empty — host fills from env vars / --signer flag.
                if a.len() != 5 {
                    return Err(
                        "outlayer/call requires 5 args: receiver method args deposit gas".into(),
                    );
                }
                let ma4 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                };
                let ret_area: i32 = 163840 + 512; // separate from view ret area
                let mut v = Vec::new();
                // Push 17 i32 params for near:rpc/api call:
                // 8 strings × (ptr, len) + ret_area = 17
                // signer_id: empty (host fills)
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0));
                // signer_key: empty (host fills)
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0));
                // receiver, method, args — evaluate as tagged strings
                for i in 0..3 {
                    let val = self.expr(&a[i])?;
                    v.extend(val.clone());
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.extend(val);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64);
                }
                // deposit — evaluate as tagged string
                let dep = self.expr(&a[3])?;
                v.extend(dep.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(dep);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // gas — evaluate as tagged string
                let gas = self.expr(&a[4])?;
                v.extend(gas.clone());
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(gas);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // wait_until: empty (host default = FINAL)
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Const(0));
                // ret_area ptr
                v.push(Instruction::I32Const(ret_area));
                // call (sentinel 101) — void return
                v.push(Instruction::Call(101));
                // Read tuple<string, string> from ret_area:
                // +0: tx_hash_ptr, +4: tx_hash_len, +8: error_ptr, +12: error_len
                v.push(Instruction::I32Const(ret_area + 12));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL)); // error → nil
                v.push(Instruction::Else);
                // Read result: ptr from +0, len from +4
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4));
                v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}

/// Parse a URL string into (authority, path_with_query).
/// Supports `https://host/path?query` and `http://host/path?query`.
fn parse_url(url: &str) -> Result<(String, String), String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| {
            format!(
                "http-get: URL must start with https:// or http://, got: {}",
                url
            )
        })?;
    let slash_pos = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..slash_pos];
    let path = if slash_pos < rest.len() {
        &rest[slash_pos..]
    } else {
        "/"
    };
    Ok((authority.to_string(), path.to_string()))
}
