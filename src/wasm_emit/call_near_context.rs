use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_context(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/current_account_id" => self.read_to_register(3, a),
            "near/predecessor_account_id" => self.read_to_register(6, a),
            "near/input" => self.read_to_register(7, a),
            "near/block_index" => {
                let mut v = vec![Self::host_call(8)];
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/block_timestamp" => {
                // Option A ruling (2026-08-26): NEAR ns timestamps (~2^60.4)
                // can't fit the 61-bit tagged payload — return DECIMAL STRING
                // (u128-string representation). Compare with u128/gt etc.
                let mut v = vec![Self::host_call(9)];
                v.extend(self.emit_itoa_raw());
                Ok(v)
            }
            "near/epoch_height" => {
                let mut v = vec![Self::host_call(10)];
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/prepaid_gas" => {
                let mut v = vec![Self::host_call(15)];
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/used_gas" => {
                let mut v = vec![Self::host_call(16)];
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/attached_deposit" => self.read_u128_low(14),
            "near/attached_deposit_high" => self.read_u128_high(14),
            "near/deposit-gte" => {
                let lo_val = match &a[0] {
                    LispVal::Num(n) => *n as u64,
                    _ => return Err("near/deposit-gte: lo must be a number literal".into()),
                };
                let hi_val = if a.len() > 1 {
                    match &a[1] {
                        LispVal::Num(n) => *n as u64,
                        _ => return Err("near/deposit-gte: hi must be a number literal".into()),
                    }
                } else {
                    0u64
                };
                let mut v = Vec::new();
                // attached_deposit(balance_ptr) writes 16 bytes directly to memory
                v.push(Instruction::I64Const(TEMP_MEM as i64)); // balance_ptr
                v.push(Self::host_call(14)); // attached_deposit -> writes to memory at TEMP_MEM
                                             // Compare: deposit >= threshold (u128 comparison)
                                             // deposit at TEMP_MEM[0..16], threshold = (lo_val, hi_val)
                                             // if dep_hi < threshold_hi → false (0)
                                             // if dep_hi > threshold_hi → true (1)
                                             // else dep_lo >= threshold_lo → result
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 8,
                    align: 3,
                    memory_index: 0,
                })); // dep_hi
                v.push(Instruction::I64Const(hi_val as i64)); // threshold_hi
                v.push(Instruction::I64LtU);
                // Stack: [i32 condition] - If consumes i32
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0)); // dep_hi < threshold_hi → false
                v.push(Instruction::Else);
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 8,
                    align: 3,
                    memory_index: 0,
                })); // dep_hi
                v.push(Instruction::I64Const(hi_val as i64));
                v.push(Instruction::I64GtU);
                // Stack: [i32 condition] - If consumes i32
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1)); // dep_hi > threshold_hi → true
                v.push(Instruction::Else);
                // Highs equal, compare low parts
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })); // dep_lo
                v.push(Instruction::I64Const(lo_val as i64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::I64ExtendI32U); // i32 → i64
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Result is 0 (false) or 1 (true) on stack
                // Tag as boolean: (payload << 3) | TAG_BOOL
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "near/attached_deposit_u128" => {
                // attached_deposit(balance_ptr) writes 16 bytes directly to memory.
                // Render as a decimal u128 STRING via the shared helper — the old
                // emit_tag_num read the pointer as an i64 and tagged garbage
                // ("64" in the differential; lisp twin str-cat rendered it as "").
                let mut v = Vec::new();
                v.push(Instruction::I64Const(TEMP_MEM as i64)); // balance_ptr
                v.push(Self::host_call(14)); // attached_deposit -> writes u128 LE at TEMP_MEM
                let h = self.ensure_u128_str_helpers();
                v.push(Instruction::I64Const(TEMP_MEM as i64)); // lo@0, hi@8 — helper's layout
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "near/account_balance" => self.read_u128_low(12),
            "near/account_balance_high" => self.read_u128_high(12),
            "near/account_locked_balance" => self.read_u128_low(13),
            "near/account_locked_balance_high" => self.read_u128_high(13),
            "near/current_code_hash" => self.read_to_register(51, a),
            "near/current_contract_code" => {
                let mut v = Vec::new();
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(72));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/refund_to_account_id" => self.read_to_register(73, a),
            "near/validator_stake" => {
                let acct = self.expr(&a[0])?;
                let stake = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(acct.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(acct);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.extend(stake);
                v.push(Self::host_call(84));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/validator_total_stake" => self.read_u128_low(85),
            "near/signer_to_buf" => {
                self.need_host(4); self.need_host(0); self.need_host(1);
                // Writes signer_account_id to SIGNER_BUF (4096), returns length as tagged NUM
                const SIGNER_BUF: i64 = 4096;
                let mut v = Vec::new();
                // host_call 4: signer_account_id() writes result to register 0
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(4));
                // read_register(0, SIGNER_BUF): register_id first, then ptr
                v.push(Instruction::I64Const(0));          // register_id
                v.push(Instruction::I64Const(SIGNER_BUF)); // ptr
                v.push(Self::host_call(0));
                // register_len(0): returns length of register 0
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/write_amount" => {
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                let __wval = self.local_idx("__wval");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                // Save value to local, then store as u128 (16 bytes) at AMOUNT_MEM (256)
                v.push(Instruction::LocalSet(__wval));
                // Low 64 bits at addr 256
                v.push(Instruction::I32Const(256));
                v.push(Instruction::LocalGet(__wval));
                v.push(Instruction::I64Store(ma));
                // High 64 bits = 0 at addr 264
                v.push(Instruction::I32Const(264));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
