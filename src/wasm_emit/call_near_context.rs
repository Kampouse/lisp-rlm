use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_context(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/current_account_id" => self.read_to_register(3, a),
            "near/predecessor_account_id" => self.read_to_register(6, a),
            "near/input" => self.read_to_register(7, a),
            "near/block_index" => { let mut v = vec![Self::host_call(8)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/block_timestamp" => { let mut v = vec![Self::host_call(9)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/epoch_height" => { let mut v = vec![Self::host_call(10)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/prepaid_gas" => { let mut v = vec![Self::host_call(15)]; v.extend(self.emit_tag_num()); Ok(v) },
            "near/used_gas" => { let mut v = vec![Self::host_call(16)]; v.extend(self.emit_tag_num()); Ok(v) },
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
                } else { 0u64 };
                let mut v = Vec::new();
                // Call attached_deposit host (writes 16 bytes to TEMP_MEM)
                v.push(Instruction::I64Const(TEMP_MEM as i64));
                v.push(Self::host_call(14));
                // Compare: deposit >= threshold
                // deposit_lo = I64Load(TEMP_MEM+0), deposit_hi = I64Load(TEMP_MEM+8)
                // threshold_lo = lo_val, threshold_hi = hi_val
                // if dep_hi < threshold_hi → false
                // if dep_hi > threshold_hi → true
                // else dep_lo >= threshold_lo
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); // dep_hi
                v.push(Instruction::I64Const(hi_val as i64)); // threshold_hi
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                    v.push(Instruction::I64Const(0)); // dep < threshold → false
                v.push(Instruction::Else);
                    v.push(Instruction::I32Const(TEMP_MEM as i32));
                    v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); // dep_hi
                    v.push(Instruction::I64Const(hi_val as i64));
                    v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::I64Const(1)); // dep > threshold → true
                    v.push(Instruction::Else);
                        // Highs equal, compare low
                        v.push(Instruction::I32Const(TEMP_MEM as i32));
                        v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); // dep_lo
                        v.push(Instruction::I64Const(lo_val as i64));
                        v.push(Instruction::I64GeU);
                        v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::End);
                v.push(Instruction::End);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/attached_deposit_u128" => {
                // Write attached deposit (16 bytes) to TEMP_MEM via host call 14
                // Then represent as 2-element array [lo, hi] on heap
                let mut v = Vec::new();
                v.push(Instruction::I64Const(TEMP_MEM as i64));
                v.push(Self::host_call(14)); // writes 16 bytes to TEMP_MEM
                // We return TEMP_MEM as a tagged Num - caller can pass to u128/store_storage
                // TEMP_MEM now holds: [u128_lo @ offset 0, u128_hi @ offset 8]
                // Alternatively, we could allocate on heap, but TEMP_MEM works for immediate use
                v.push(Instruction::I64Const(TEMP_MEM));
                v.extend(self.emit_tag_num());
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
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/refund_to_account_id" => self.read_to_register(73, a),
            "near/validator_stake" => {
                let acct = self.expr(&a[0])?;
                let stake = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(stake);
                v.push(Self::host_call(84));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/validator_total_stake" => self.read_u128_low(85),
            _ => Err("__not_handled__".into()),
        }
    }
}
