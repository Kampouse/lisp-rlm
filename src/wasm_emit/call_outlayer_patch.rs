            "env/get" => {
                // WIT: env-var(name: string) -> string
                // Returns empty string if not found
                // Canonical ABI: (name_ptr, name_len, ret_ptr) -> ()
                // Return area: (ptr: i32, len: i32) for the result string
                if a.is_empty() { return Err("env/get requires a variable name string".into()); }
                if !self.wasi_mode { return Err("env/get is only available on OutLayer".into()); }
                let key_expr = self.expr(&a[0])?;
                let key_expr2 = key_expr.clone();
                let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                let ret_area: i32 = crate::wasi_http::OL_RET_AREA_BASE + 448;
                let mut v = Vec::new();
                // key ptr/len (2 × i32) — extract from tagged string
                v.extend(key_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64); // len
                v.extend(key_expr2);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // ptr
                v.push(Instruction::I32Const(ret_area)); // ret_area
                v.push(Instruction::Call(122));
                // Read result: (ptr, len) at ret_area
                v.push(Instruction::I32Const(ret_area)); v.push(Instruction::I32Load(ma4)); // ptr
                v.push(Instruction::I32Const(ret_area + 4)); v.push(Instruction::I32Load(ma4)); // len
                // Pack into tagged string: (len << 32) | ptr | TAG_STR
                v.push(Instruction::I64ExtendI32U); // len
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Const(TAG_STR as i64)); v.push(Instruction::I64Or);
                Ok(v)
            }