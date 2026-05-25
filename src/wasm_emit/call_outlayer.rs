use super::*;

impl WasmEmitter {
    pub(crate) fn call_outlayer(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "http-get" => {
                // (http-get "https://api.example.com/data") -> string or nil
                if a.is_empty() { return Err("http-get requires a URL string argument".into()); }
                if !self.wasi_mode { return Err("http-get is only available on OutLayer (WASI) target".into()); }

                // OutLayer host function path (works on both P1 and P2)
                self.need_outlayer = true;

                let url_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840; // RET_AREA offset
                let mut v = Vec::new();

                // Push url_ptr, url_len
                v.extend(url_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // url_ptr
                v.extend(url_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // url_len
                // Push ret_area
                v.push(Instruction::I32Const(ret_area));
                // Call http-get (sentinel 103) — canonical ABI
                v.push(Instruction::Call(103));
                // Read discriminant: 0 = ok, 1 = err
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL)); // error → nil
                v.push(Instruction::Else);
                // ok: read ptr from ret_area+4, len from ret_area+8
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 8)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                // Tag: ((ptr | (len << 32)) << 3) | TAG_STR
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End); // if
                Ok(v)
            }
            "http-post" => {
                // (http-post "https://api.example.com/data" "body") -> string or nil
                if a.len() < 2 { return Err("http-post requires (url body)".into()); }
                if !self.wasi_mode { return Err("http-post is only available on OutLayer (WASI) target".into()); }

                // OutLayer host function path (works on both P1 and P2)
                // WIT: http-post(url: string, body: list<u8>, content_type: string) -> result<list<u8>, string>
                // Canonical ABI: url(2) + body(2) + content_type(2) + retptr(1) = 7 i32 params
                self.need_outlayer = true;
                let url_expr = self.expr(&a[0])?;
                let body_expr = self.expr(&a[1])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840;
                let content_type_area: i32 = 163900;
                // Store default content-type "application/json" as data segment
                self.data_segments.push((content_type_area as u32, b"application/json".to_vec()));
                let content_type_len: i32 = 16; // "application/json".len()
                let mut v = Vec::new();
                // url ptr/len
                v.extend(url_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(url_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // body ptr/len
                v.extend(body_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(body_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // content_type ptr/len
                v.push(Instruction::I32Const(content_type_area));
                v.push(Instruction::I32Const(content_type_len));
                // ret_area
                v.push(Instruction::I32Const(ret_area));
                // Call http-post (sentinel 104) — canonical ABI with 7 params
                v.push(Instruction::Call(104));
                // Read result (same pattern as http-get)
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 8)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End);
                Ok(v)
            }
            "storage-set" => {
                // (storage-set "key" "value") -> bool
                // WIT: storage-set(key: string, value: list<u8>) -> result<(), string>
                // Canonical ABI: (key_ptr, key_len, val_ptr, val_len, ret_ptr) -> ()
                if a.len() < 2 { return Err("storage-set requires (key value)".into()); }
                if !self.wasi_mode { return Err("storage-set is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 64; // offset past http ret areas
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
                // ret_ptr
                v.push(Instruction::I32Const(ret_area));
                // Call storage-set (sentinel 110) — canonical ABI: 5 params, void
                v.push(Instruction::Call(110));
                // Read discriminant from ret_area: 0 = ok, 1 = err
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                // Return (discriminant == 0) as tagged bool
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-get" => {
                // (storage-get "key") -> string or nil
                // WIT: result<option<list<u8>>, string>
                // Canonical ABI layout (NOT flattened, nested result+option):
                //   +0: result disc (0=ok, 1=err)
                //   +4: option disc (0=none, 1=some) — only valid when result=ok
                //   +8: ptr (i32) — only valid when option=some
                //   +12: len (i32) — only valid when option=some
                if a.is_empty() { return Err("storage-get requires a key".into()); }
                if !self.wasi_mode { return Err("storage-get is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 128;
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
                // ret_ptr
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(111));
                // Read result discriminant: 0=ok, 1=err
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // disc != 0 → error → TAG_NIL
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // disc=0 (ok): read option discriminant at +4
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // option=0 → none → TAG_NIL
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // option=1 → some: read ptr@+8, len@+12
                v.push(Instruction::I32Const(ret_area + 8)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 12)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End); // inner if (option)
                v.push(Instruction::End); // outer if (result)
                Ok(v)
            }
            "storage-has" => {
                // (storage-has "key") -> bool
                // WIT: storage-has(key: string) -> result<bool, string>
                // Canonical ABI: (key_ptr, key_len, ret_ptr) -> ()
                // Result: disc(0=ok,1=err), if ok: i32(0=false,1=true) at +4
                if a.is_empty() { return Err("storage-has requires a key".into()); }
                if !self.wasi_mode { return Err("storage-has is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 192;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(112));
                // Read discriminant then bool
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-delete" => {
                // (storage-delete "key") -> bool
                // WIT: storage-delete(key: string) -> result<_, string>
                // Canonical ABI: (key_ptr, key_len, ret_ptr) -> ()
                // Result: disc(0=ok,1=err) at ret_ptr
                if a.is_empty() { return Err("storage-delete requires a key".into()); }
                if !self.wasi_mode { return Err("storage-delete is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 256;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(113));
                // Read discriminant: 0=ok, nonzero=err → return as bool
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "storage-increment" => {
                // (storage-increment "key" delta) -> i64 (new value)
                // Canonical ABI: (i32 key_ptr, i32 key_len, i64 delta, i32 ret_area) -> ()
                // result<s64, string> ret_area layout: [i32 disc] [4 pad] [i64 s64 @ +8]
                if a.len() < 2 { return Err("storage-increment requires (key delta)".into()); }
                if !self.wasi_mode { return Err("storage-increment is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let delta_expr = self.expr(&a[1])?;
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // key ptr/len (2 × i32) — extract from tagged pair
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // key_ptr (i32)
                v.extend(key_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // key_len (i32)
                // delta (i64) — untag, keep as i64 for canonical ABI
                v.extend(delta_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); // untag → i64
                // ret_area pointer (i32)
                v.push(Instruction::I32Const(163840)); // OL_RET_AREA
                v.push(Instruction::Call(114));
                // Read s64 result from ret_area + 8 (canonical ABI: disc@0, payload@8)
                v.push(Instruction::I32Const(163840 + 8));
                v.push(Instruction::I64Load(ma8));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "env/signer" => {
                // WIT: env-signer() -> string
                // Canonical ABI: (ret_ptr) -> ()
                // Result: (ptr, len) written by host to ret_area
                if !self.wasi_mode { return Err("env/signer is only available on OutLayer".into()); }
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 320;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(120));
                // Read ptr/len from ret_area
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                Ok(v)
            }
            "env/predecessor" => {
                // WIT: env-predecessor() -> string
                // Canonical ABI: (ret_ptr) -> ()
                // Result: (ptr, len) written by host to ret_area
                if !self.wasi_mode { return Err("env/predecessor is only available on OutLayer".into()); }
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 384;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(ret_area));
                v.push(Instruction::Call(121));
                // Read ptr/len from ret_area
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
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
            // ── P2 inline operations (no host calls, pure WASM) ──
            "outlayer/http-post" => {
                // (outlayer/http-post "url" "body" ["content-type"]) -> string or nil
                // Canonical ABI: http-post(url_ptr, url_len, body_ptr, body_len, ct_ptr, ct_len, ret_area) -> ()
                // ret_area: disc(4) + ptr(4) + len(4) = 12 bytes
                if a.len() < 2 { return Err("outlayer/http-post requires (url body) or (url body content-type)".into()); }
                if !self.wasi_mode { return Err("outlayer/http-post is only available on OutLayer (WASI) target".into()); }
                self.need_outlayer = true;

                let url_expr = self.expr(&a[0])?;
                let body_expr = self.expr(&a[1])?;
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = 163840 + 16; // POST ret_area after GET's
                let mut v = Vec::new();

                // url ptr/len
                v.extend(url_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // url_ptr
                v.extend(url_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // url_len
                // body ptr/len
                v.extend(body_expr.clone());
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64); // body_ptr
                v.extend(body_expr);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // body_len
                // content-type ptr/len
                if a.len() > 2 {
                    let ct_expr = self.expr(&a[2])?;
                    v.extend(ct_expr.clone());
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64); // ct_ptr
                    v.extend(ct_expr);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I32WrapI64); // ct_len
                } else {
                    let ct_str = b"application/json";
                    let ct_off = self.alloc_data(ct_str);
                    v.push(Instruction::I32Const(ct_off as i32)); // ct_ptr
                    v.push(Instruction::I32Const(ct_str.len() as i32)); // ct_len
                }
                // ret_area pointer
                v.push(Instruction::I32Const(ret_area));
                // Call http-post (sentinel 104) — canonical ABI
                v.push(Instruction::Call(104));
                // Read discriminant: 0 = ok, 1 = err
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL)); // error → nil
                v.push(Instruction::Else);
                // ok: read ptr from ret_area+4, len from ret_area+8
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(ret_area + 8)); v.push(Instruction::I32Load(ma4));
                v.push(Instruction::I64ExtendI32U);
                // Tag: ((ptr | (len << 32)) << 3) | TAG_STR
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_STR)); v.push(Instruction::I64Or);
                v.push(Instruction::End); // if
                Ok(v)
            }
            "outlayer/json-get" => {
                // (outlayer/json-get tagged_string "key") -> string or nil
                // Scans JSON from a tagged string (e.g., HTTP response) for the given key
                if a.len() < 2 { return Err("outlayer/json-get requires (string key)".into()); }
                if !self.wasi_mode { return Err("outlayer/json-get is only available on OutLayer (WASI) target".into()); }
                match &a[1] {
                    crate::types::LispVal::Str(key) => {
                        let buf_expr = self.expr(&a[0])?;
                        let tmp_l = self.local_idx("__ojg_tmp");
                        let src_ptr_l = self.local_idx("__ojg_sp");
                        let copy_i = self.local_idx("__ojg_ci");
                        let target_buf = 98304i64; // response buffer
                        let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };

                        let mut setup = Vec::new();
                        // Untag the source string: >>3 gives payload = (len << 32) | ptr
                        setup.extend(buf_expr);
                        setup.push(Instruction::I64Const(3)); setup.push(Instruction::I64ShrU);
                        setup.push(Instruction::LocalSet(tmp_l));
                        // Extract src_ptr = tmp & 0xFFFFFFFF
                        setup.push(Instruction::LocalGet(tmp_l));
                        setup.push(Instruction::I64Const(0xFFFFFFFF)); setup.push(Instruction::I64And);
                        setup.push(Instruction::LocalSet(src_ptr_l));
                        // Extract len = tmp >> 32
                        setup.push(Instruction::LocalGet(tmp_l));
                        setup.push(Instruction::I64Const(32)); setup.push(Instruction::I64ShrU);
                        // Copy string to target_buf so json_get_from_buf can scan it
                        // Save len to another local for the copy
                        let src_len_l = self.local_idx("__ojg_slen");
                        setup.push(Instruction::LocalSet(src_len_l));
                        // Copy loop: target_buf[i] = src_ptr[i] for i in 0..len
                        setup.push(Instruction::I64Const(0)); setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::LocalGet(src_len_l));
                        setup.push(Instruction::I64GeU); setup.push(Instruction::BrIf(1));
                        // target_buf[i] = src[i]
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(copy_i)); setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(src_ptr_l)); setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(copy_i)); setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Add);
                        setup.push(Instruction::I32Load8U(ma1));
                        setup.push(Instruction::I32Store8(ma1));
                        setup.push(Instruction::LocalGet(copy_i)); setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add); setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End); setup.push(Instruction::End);
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
                if a.len() < 2 { return Err("outlayer/str-concat requires at least 2 args".into()); }
                self.call_string("str-cat", a)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}

/// Parse a URL string into (authority, path_with_query).
/// Supports `https://host/path?query` and `http://host/path?query`.
fn parse_url(url: &str) -> Result<(String, String), String> {
    let url = url.trim();
    let rest = url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("http-get: URL must start with https:// or http://, got: {}", url))?;
    let slash_pos = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..slash_pos];
    let path = if slash_pos < rest.len() { &rest[slash_pos..] } else { "/" };
    Ok((authority.to_string(), path.to_string()))
}
