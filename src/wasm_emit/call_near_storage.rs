use super::*;
use wasm_encoder::{MemArg, Instruction, BlockType, ValType};

impl WasmEmitter {
    pub(crate) fn call_near_storage(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
match op {
            "near/store" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let __s = self.local_idx("__s");
                let __v = self.local_idx("__sv");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.extend(val);
                v.push(Instruction::LocalSet(__v));
                let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(__v));
                v.push(Instruction::I64Store(ma));
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load" => {
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__load_key");
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/remove" => {
                let key = self.expr(&a[0])?;
                let __s = self.local_idx("__s");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/store_num" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let __s = self.local_idx("__s");
                let __n = self.local_idx("__n");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(__n));
                let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(__n));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load_num" => {
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__ln_key");
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/has_key" => {
                let key = self.expr(&a[0])?;
                let __s = self.local_idx("__s");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // ── near/kv: composite key-value store ──
            // (near/kv "prefix" part1 part2 ... val)
            // Last arg = value (i64), all preceding = key parts (strings).
            // Builds composite key in KEY_BUF via byte-copy loop, then storage_write.
            "near/kv" => {
                self.need_host(17);
                let n_parts = a.len().saturating_sub(1);
                if n_parts < 1 {
                    return Err("near/kv needs >= 2 args: key parts + value".into());
                }
                let val_expr = self.expr(&a[a.len()-1])?;
                let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                let off = self.local_idx("__kv_off");
                let part = self.local_idx("__kv_part");
                let raw = self.local_idx("__kv_raw");
                let plen = self.local_idx("__kv_plen");
                let pptr = self.local_idx("__kv_pptr");
                let ci = self.local_idx("__kv_ci");
                // Evaluate value first, save to local
                v.extend(val_expr);
                let val_local = self.local_idx("__kv_val");
                v.push(Instruction::LocalSet(val_local));
                // offset = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(off));
                // Pre-evaluate all part expressions
                let mut part_exprs = Vec::new();
                for i in 0..n_parts {
                    part_exprs.push(self.expr(&a[i])?);
                }
                // For each part: untag, extract ptr/len, byte-copy to KEY_BUF, advance offset
                for part_e in part_exprs {
                    v.extend(part_e);
                    v.push(Instruction::LocalSet(part));
                    // Untag → raw = (len << 32) | ptr
                    v.push(Instruction::LocalGet(part));
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw));
                    // plen = raw >> 32
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(plen));
                    // pptr = raw & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(pptr));
                    // ci = 0
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(ci));
                    // byte-copy loop: while ci < plen { KEY_BUF[off+ci] = mem[pptr+ci]; ci++ }
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    // if ci >= plen: break
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::BrIf(1));
                    // dst = KEY_BUF + off + ci (i32)
                    v.push(Instruction::I64Const(KEY_BUF));
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // src = pptr + ci (i32)
                    v.push(Instruction::LocalGet(pptr));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // load 1 byte from src (zero-extend to i64), store 8 bytes to dst
                    v.push(Instruction::I64Load8U(ma0.clone()));
                    v.push(Instruction::I64Store(ma0.clone()));
                    // ci++
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(ci));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // off += plen
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off));
                }
                // Write tagged value to STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(val_local));
                v.push(Instruction::I64Store(ma8));
                // storage_write(key_len=off, key_ptr=KEY_BUF, val_len=8, val_ptr=STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(off));
                v.push(Instruction::I64Const(KEY_BUF));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // ── near/kv-get: read from composite key ──
            // (near/kv-get "prefix" part1 part2 ...)
            // All args are key parts (strings). Returns tagged value or 0.
            "near/kv-get" => {
                self.need_host(18);
                self.need_host(0);
                let n_parts = a.len();
                if n_parts < 1 {
                    return Err("near/kv-get needs >= 1 key part".into());
                }
                let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                let off = self.local_idx("__kvg_off");
                let part = self.local_idx("__kvg_part");
                let raw = self.local_idx("__kvg_raw");
                let plen = self.local_idx("__kvg_plen");
                let pptr = self.local_idx("__kvg_pptr");
                let ci = self.local_idx("__kvg_ci");
                // offset = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(off));
                // Pre-evaluate all part expressions
                let mut part_exprs = Vec::new();
                for i in 0..n_parts {
                    part_exprs.push(self.expr(&a[i])?);
                }
                // Build composite key in KEY_BUF
                for part_e in part_exprs {
                    v.extend(part_e);
                    v.push(Instruction::LocalSet(part));
                    // Untag → raw = (len << 32) | ptr
                    v.push(Instruction::LocalGet(part));
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw));
                    // plen = raw >> 32
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(plen));
                    // pptr = raw & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(pptr));
                    // ci = 0
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(ci));
                    // byte-copy loop
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::BrIf(1));
                    v.push(Instruction::I64Const(KEY_BUF));
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(pptr));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load8U(ma0.clone()));
                    v.push(Instruction::I64Store(ma0.clone()));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(ci));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off));
                }
                // storage_read(key_len=off, key_ptr=KEY_BUF, register=1)
                v.push(Instruction::LocalGet(off));
                v.push(Instruction::I64Const(KEY_BUF));
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                // 0 = not found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // register_read(1, STORAGE_BUF) — void
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                // Load tagged value from STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::End);
                Ok(v)
            }

            _ => Err("__not_handled__".to_string()),
        }
    }
}
