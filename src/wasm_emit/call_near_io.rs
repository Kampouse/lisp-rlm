use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_io(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/return" => {
                let val = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32)); v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // value_return(len=8, ptr=TEMP_MEM) — idx 25
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                // Set return flag so export wrapper skips its value_return
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/log" => {
                // (near/log "string") — log string
                // (near/log "prefix" num) — log string then number (two separate log calls)
                if a.len() == 1 {
                    let msg = self.expr(&a[0])?;
                    let mut v = Vec::new();
                    // Untag string to get encoded (ptr | (len << 32))
                    v.extend(msg.clone());
                    v.extend(self.emit_untag());
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                    v.extend(msg);
                    v.extend(self.emit_untag());
                    v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
                } else {
                    // Two separate log calls: first the string, then the number
                    let msg = self.expr(&a[0])?;
                    let num_expr = self.expr(&a[1])?;
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    let abs_val = self.local_idx("__logn_abs");
                    let digit_count = self.local_idx("__logn_digits");
                    let is_neg = self.local_idx("__logn_neg");
                    let tmp_digit = self.local_idx("__logn_d");
                    let ptr = self.local_idx("__logn_ptr");
                    let mut v = Vec::new();
                    // First: log the string
                    v.extend(msg.clone());
                    v.extend(self.emit_untag());
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                    v.extend(msg);
                    v.extend(self.emit_untag());
                    v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                    v.push(Self::host_call(28));
                    // Second: log the number (same technique as near/log_num)
                    v.extend(num_expr);
                    v.push(Instruction::LocalSet(abs_val));
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64LtS);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(is_neg));
                    v.push(Instruction::LocalGet(is_neg));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::Else);
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalSet(abs_val));
                    v.push(Instruction::I64Const(4184));
                    v.push(Instruction::LocalSet(ptr));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64RemS);
                    v.push(Instruction::LocalSet(tmp_digit));
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64DivS);
                    v.push(Instruction::LocalSet(abs_val));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(ptr));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Instruction::LocalGet(tmp_digit));
                    v.push(Instruction::I64Const(48));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.emit_safe_store8());
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);
                    // Zero special case
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(4183));
                    v.push(Instruction::LocalSet(ptr));
                    v.push(Instruction::I64Const(4183));
                    v.push(Instruction::I64Const(48));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.emit_safe_store8());
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);
                    // Negative prefix
                    v.push(Instruction::LocalGet(is_neg));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(ptr));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Instruction::I64Const(45));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.emit_safe_store8());
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::LocalGet(ptr));
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
                }
            }
            "near/panic" => {
                let msg = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(27)); // panic_utf8(len, ptr)
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "near/abort" => {
                // panic() — idx 26, traps unconditionally
                Ok(vec![Self::host_call(26), Instruction::I64Const(0)])
            }
            "abort" => {
                // WASM unreachable — always traps, no env import needed
                Ok(vec![Instruction::Unreachable])
            }
            "print" | "println" => {
                // Evaluate arg, write to stdout (WASI) or log (NEAR), return nil
                if a.is_empty() {
                    return Ok(vec![Instruction::I64Const(TAG_NIL)]);
                }
                let val = self.expr(&a[0])?;
                let mut v = Vec::new();
                if self.wasi_mode {
                    // WASI: fd_write to stdout
                    // Check tag: if string (TAG_STR=5), extract ptr/len and fd_write
                    // If number, convert to decimal at STDOUT_BUF and fd_write
                    let tagged = self.local_idx("__print_val");
                    let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 };
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    // Store tagged value
                    v.extend(val);
                    v.push(Instruction::LocalSet(tagged));
                    // Check if string: (tagged & 7) == TAG_STR (5)
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(7));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(5)); // TAG_STR
                    v.push(Instruction::I64Eq);
                    // i64.eq produces i32 directly, no wrap needed
                    v.push(Instruction::If(BlockType::Empty));
                    // ── String path ──
                    // Build iov at offset 64: [ptr, len]
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU); // payload
                    v.push(Instruction::I64Const(0xFFFFFFFF));
                    v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone())); // iov[0].buf
                    v.push(Instruction::I32Const(68));
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU); // len
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone())); // iov[0].len
                    // fd_write(1, 64, 1, nwritten=98308) — use 98308 NOT 98304 (STDIN_LEN)
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(98308));
                    v.push(Instruction::Call(WASI_FD_WRITE));
                    v.push(Instruction::Drop);
                    // If println, write newline
                    if op == "println" {
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(0x0A)); // '\n'
                        v.push(Instruction::I32Store8(ma8.clone()));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(98308));
                        v.push(Instruction::Call(WASI_FD_WRITE));
                        v.push(Instruction::Drop);
                    }
                    v.push(Instruction::Else);
                    // ── Non-string path: convert i64 to decimal ──
                    let untagged = self.local_idx("__print_un");
                    let digit_count = self.local_idx("__print_dc");
                    let is_neg = self.local_idx("__print_neg");
                    let wptr = self.local_idx("__print_wp");
                    let sb: i64 = 65536; // STDOUT_BUF
                    // Untag: >> 3 (logical shift for correct unsigned values)
                    v.push(Instruction::LocalGet(tagged));
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(is_neg));
                    // Check negative
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64LtS);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(is_neg));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::End);
                    // Check zero
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I32Const(sb as i32));
                    v.push(Instruction::I32Const(0x30));
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Else);
                    // Digits backward at sb+31
                    v.push(Instruction::I64Const(sb + 31));
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64RemU);
                    v.push(Instruction::I64Const(0x30));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::LocalGet(untagged));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(untagged));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // ptr+1 = start
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(wptr));
                    // If negative: write '-'
                    v.push(Instruction::LocalGet(is_neg));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Ne);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(wptr));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Const(0x2D)); // '-'
                    v.push(Instruction::I32Store8(ma8.clone()));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);
                    v.push(Instruction::End); // else (zero)
                    // fd_write: iov at TEMP+64
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::LocalGet(wptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone()));
                    v.push(Instruction::I32Const(68));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store(ma4.clone()));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(64));
                    v.push(Instruction::I32Const(1));
                    v.push(Instruction::I32Const(98308));
                    v.push(Instruction::Call(WASI_FD_WRITE));
                    v.push(Instruction::Drop);
                    // If println, newline
                    if op == "println" {
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(0x0A));
                        v.push(Instruction::I32Store8(ma8.clone()));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(64));
                        v.push(Instruction::I32Const(1));
                        v.push(Instruction::I32Const(98308));
                        v.push(Instruction::Call(WASI_FD_WRITE));
                        v.push(Instruction::Drop);
                    }
                    v.push(Instruction::End); // if string/else
                } else {
                    // NEAR: use near/log (host func 28) for strings
                    self.need_host(28);
                    // For now: if arg is string literal, log it
                    v.extend(val.clone());
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU); // len
                    v.extend(val);
                    v.push(Instruction::I32WrapI64); // ptr
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Self::host_call(28));
                }
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
