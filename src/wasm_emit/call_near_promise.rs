use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_promise(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/call" => {
                if a.len() != 5 { return Err("near/call: need 5 args (target, method, args, gas, deposit)".into()); }
                let acct = self.expr(&a[0])?;
                let meth = self.expr(&a[1])?;
                let args_val = self.expr(&a[2])?;
                let gas = self.expr(&a[3])?;
                let dep = self.expr(&a[4])?;
                let mut v = Vec::new();
                // Write deposit u128 to TEMP_MEM (zero high 64, write low 64)
                // Stack: [addr_i32, val_i64] for i64.store
                // First zero out high 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // Zero out low 64 bits
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Write deposit (low 64 bits) to TEMP_MEM (addr first, then val)
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                v.extend(dep); v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // account_id (len, ptr)
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method (len, ptr)
                v.extend(meth.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(meth); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // args (len, ptr)
                v.extend(args_val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args_val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount_ptr (TEMP_MEM)
                v.push(Instruction::I64Const(TEMP_MEM));
                // gas (untagged Num)
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(30)); // promise_create → promise_idx on stack
                v.push(Self::host_call(35)); // promise_return(promise_idx) — forward result to caller
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/promise_create" => {
                // promise_create(account_id_len, account_id_ptr, method_name_len, method_name_ptr,
                //                arguments_len, arguments_ptr, amount_ptr, gas) → i64  (idx 30)
                let account = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let mut v = Vec::new();
                // account_id: untag → len >> 32, ptr & 0xFFFF_FFFF
                v.extend(account.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method_name: untag → len >> 32, ptr
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // arguments: untag → len >> 32, ptr
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount: untag, store at mem[0], pass ptr=0
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); // amount_ptr
                // gas: untag for host
                v.extend(gas);
                v.extend(self.emit_untag());
                v.push(Self::host_call(30)); // returns promise_index
                v.extend(self.emit_tag_num()); // tag return
                Ok(v)
            }
            "near/promise_then" => {
                let pidx = self.expr(&a[0])?;
                let account = self.expr(&a[1])?;
                let method = self.expr(&a[2])?;
                let args = self.expr(&a[3])?;
                let amount = self.expr(&a[4])?;
                let gas = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(pidx); v.extend(self.emit_untag()); // untag promise idx
                v.extend(account.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0));
                v.extend(gas);
                v.push(Self::host_call(31));
                Ok(v)
            }
            "near/promise_and" => {
                // promise_and(promise_idx_ptr, promise_idx_count) → i64  (idx 32)
                // Store all promise indices at mem offset 64, then pass ptr+count
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() {
                    v.push(Instruction::I32Const((64 + i * 8) as i32));
                    v.extend(self.expr(x)?);
                    v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                }
                v.push(Instruction::I64Const(64)); // ptr
                v.push(Instruction::I64Const(a.len() as i64)); // count
                v.push(Self::host_call(32));
                Ok(v)
            }
            "near/promise_results_count" => {
                Ok(vec![Self::host_call(33), Instruction::I64Const(TAG_BITS), Instruction::I64Shl])
            }
            "near/promise_result" => {
                self.need_host(34); self.need_host(0); self.need_host(1);
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(34));
                // Read register to TEMP_MEM, get length, return packed
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM as i64)); v.push(Instruction::I64Or);
                Ok(v)
            }
            "near/promise_return" => {
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(35));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_create" => {
                if a.len() != 2 { return Err("near/promise_batch_create: need 2 args (ptr, len)".into()); }
                let ptr = self.expr(&a[0])?;
                let len = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(len); v.extend(ptr);
                v.push(Self::host_call(39));
                Ok(v)
            }
            "near/promise_batch_then" => {
                if a.len() != 3 { return Err("near/promise_batch_then: need 3 args (idx, ptr, len)".into()); }
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(len); v.extend(ptr);
                v.push(Self::host_call(40));
                Ok(v)
            }
            "near/promise_batch_action_create_account" => {
                if a.len() != 1 { return Err("near/promise_batch_action_create_account: need 1 args (idx)".into()); }
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(41));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_deploy_contract" => {
                if a.len() != 3 { return Err("near/promise_batch_action_deploy_contract: need 3 args (idx, code_ptr, code_len)".into()); }
                let idx = self.expr(&a[0])?;
                let code_ptr = self.expr(&a[1])?;
                let code_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(code_len); v.extend(code_ptr);
                v.push(Self::host_call(42));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_function_call" => {
                if a.len() != 7 { return Err("near/promise_batch_action_function_call: need 7 args (idx, method_ptr, method_len, args_ptr, args_len, amount_ptr, gas)".into()); }
                let idx = self.expr(&a[0])?;
                let method_ptr = self.expr(&a[1])?;
                let method_len = self.expr(&a[2])?;
                let args_ptr = self.expr(&a[3])?;
                let args_len = self.expr(&a[4])?;
                let amount_ptr = self.expr(&a[5])?;
                let gas = self.expr(&a[6])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(method_len); v.extend(method_ptr);
                v.extend(args_len); v.extend(args_ptr);
                v.extend(amount_ptr); v.extend(gas);
                v.push(Self::host_call(43));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_transfer" => {
                if a.len() != 3 { return Err("near/promise_batch_action_transfer: need 3 args (idx, amount_ptr, amount_len)".into()); }
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(amount_ptr); v.extend(amount_len);
                v.push(Self::host_call(44));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_stake" => {
                if a.len() != 5 { return Err("near/promise_batch_action_stake: need 5 args (idx, amount_ptr, amount_len, pk_ptr, pk_len)".into()); }
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let pk_ptr = self.expr(&a[3])?;
                let pk_len = self.expr(&a[4])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(amount_ptr); v.extend(amount_len);
                v.extend(pk_ptr); v.extend(pk_len);
                v.push(Self::host_call(45));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_add_key_with_full_access" => {
                if a.len() != 4 { return Err("near/promise_batch_action_add_key_with_full_access: need 4 args (idx, pk_ptr, pk_len, nonce)".into()); }
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let nonce = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len); v.extend(nonce);
                v.push(Self::host_call(46));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_add_key_with_function_call" => {
                if a.len() != 7 { return Err("near/promise_batch_action_add_key_with_function_call: need 7 args (idx, pk_ptr, pk_len, nonce, method_ptr, method_len, allowance)".into()); }
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let nonce = self.expr(&a[3])?;
                let method_ptr = self.expr(&a[4])?;
                let method_len = self.expr(&a[5])?;
                let allowance = self.expr(&a[6])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len); v.extend(nonce);
                v.extend(method_ptr); v.extend(method_len); v.extend(allowance);
                v.push(Self::host_call(47));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_delete_key" => {
                if a.len() != 3 { return Err("near/promise_batch_action_delete_key: need 3 args (idx, pk_ptr, pk_len)".into()); }
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len);
                v.push(Self::host_call(48));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_batch_action_delete_account" => {
                if a.len() != 3 { return Err("near/promise_batch_action_delete_account: need 3 args (idx, ptr, len)".into()); }
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(ptr); v.extend(len);
                v.push(Self::host_call(49));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/batch" => {
                if a.len() != 1 { return Err("near/batch: expected 1 arg".into()); }
                self.need_host(39);
                let account = self.expr(&a[0])?;
                let mut v = Vec::new();
                // account untagged: (len << 32) | ptr
                v.extend(account.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(account);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(39));
                Ok(v)
            }
            "near/batch-create-account" => {
                if a.len() != 1 { return Err("near/batch-create-account: expected 1 arg".into()); }
                self.need_host(41);
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(self.emit_untag());
                v.push(Self::host_call(41));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/batch-deploy" => {
                if a.len() != 2 { return Err("near/batch-deploy: expected 2 args".into()); }
                self.need_host(42);
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // code untagged: (len << 32) | ptr
                v.extend(code.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // code_len
                v.extend(code);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // code_ptr
                v.push(Self::host_call(42));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/batch-transfer" => {
                if a.len() != 3 { return Err("near/batch-transfer: expected 3 args".into()); }
                self.need_host(44);
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(self.emit_untag());
                v.extend(amount_ptr); v.extend(self.emit_untag());
                v.extend(amount_len); v.extend(self.emit_untag());
                v.push(Self::host_call(44));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/batch-call" => {
                if a.len() != 5 { return Err("near/batch-call: expected 5 args".into()); }
                self.need_host(43);
                let idx = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount_ptr = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let mut v = Vec::new();
                // promise_index (untag)
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // method_name_len
                v.extend(method.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // method_len
                // method_name_ptr
                v.extend(method);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // method_ptr
                // arguments_len
                v.extend(args.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // args_len
                // arguments_ptr
                v.extend(args);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // args_ptr
                // amount_ptr (raw, untag)
                v.extend(amount_ptr); v.extend(self.emit_untag());
                // gas (untag)
                v.extend(gas); v.extend(self.emit_untag());
                v.push(Self::host_call(43));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/batch-add-key" => {
                if a.len() != 3 { return Err("near/batch-add-key: expected 3 args".into()); }
                self.need_host(46);
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonce = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx.clone()); v.extend(self.emit_untag());
                // pk ptr/len
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                // nonce
                v.extend(nonce); v.extend(self.emit_untag());
                v.push(Self::host_call(46));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/deploy_contract" => {
                if a.len() != 2 { return Err("near/deploy_contract: need 2 args (code_ptr, code_len)".into()); }
                let code_ptr = self.expr(&a[0])?;
                let code_len = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Untag: extract ptr and len from tagged string
                v.extend(code_len.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // code_len
                v.extend(code_ptr);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // code_ptr
                v.push(Self::host_call(50)); // deploy_contract(len, ptr)
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/promise_set_refund_to" => {
                if a.len() != 2 { return Err("near/promise_set_refund_to: need 2 args (idx, acct)".into()); }
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(68));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_state_init" => {
                if a.len() != 3 { return Err("near/promise_batch_action_state_init: need 3 args (idx, code, amt)".into()); }
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(69));
                v.extend(self.emit_tag_num()); Ok(v)
            }
            "near/promise_batch_action_state_init_by_account_id" => {
                if a.len() != 3 { return Err("near/promise_batch_action_state_init_by_account_id: need 3 args (idx, acct, amt)".into()); }
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(70));
                v.extend(self.emit_tag_num()); Ok(v)
            }
            "near/set_state_init_data_entry" => {
                if a.len() != 4 { return Err("near/set_state_init_data_entry: need 4 args (pidx, aidx, key, val)".into()); }
                let pidx = self.expr(&a[0])?;
                let aidx = self.expr(&a[1])?;
                let key = self.expr(&a[2])?;
                let val = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(pidx); v.extend(aidx);
                v.extend(key.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(val.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(val); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(71));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_function_call_weight" => {
                if a.len() != 6 { return Err("near/promise_batch_action_function_call_weight: need 6 args (idx, method, args, amount, gas, weight)".into()); }
                let idx = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let weight = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amount); v.extend(gas); v.extend(weight);
                v.push(Self::host_call(74));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_deploy_global_contract" => {
                if a.len() != 2 { return Err("near/promise_batch_action_deploy_global_contract: need 2 args (idx, code)".into()); }
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(75));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_deploy_global_contract_by_account_id" => {
                if a.len() != 2 { return Err("near/promise_batch_action_deploy_global_contract_by_account_id: need 2 args (idx, code)".into()); }
                let idx = self.expr(&a[0])?;
                let code = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(code.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(code); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(76));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_use_global_contract" => {
                if a.len() != 2 { return Err("near/promise_batch_action_use_global_contract: need 2 args (idx, hash)".into()); }
                let idx = self.expr(&a[0])?;
                let hash = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(hash.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(hash); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(77));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_use_global_contract_by_account_id" => {
                if a.len() != 2 { return Err("near/promise_batch_action_use_global_contract_by_account_id: need 2 args (idx, acct)".into()); }
                let idx = self.expr(&a[0])?;
                let acct = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(78));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_transfer_to_gas_key" => {
                if a.len() != 3 { return Err("near/promise_batch_action_transfer_to_gas_key: need 3 args (idx, pk, amt)".into()); }
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let amt = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(amt);
                v.push(Self::host_call(79));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_add_gas_key_with_full_access" => {
                if a.len() != 3 { return Err("near/promise_batch_action_add_gas_key_with_full_access: need 3 args (idx, pk, nonces)".into()); }
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonces = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(nonces);
                v.push(Self::host_call(80));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_batch_action_add_gas_key_with_function_call" => {
                if a.len() != 6 { return Err("near/promise_batch_action_add_gas_key_with_function_call: need 6 args (idx, pk, nonces, allow, recv, methods)".into()); }
                let idx = self.expr(&a[0])?;
                let pk = self.expr(&a[1])?;
                let nonces = self.expr(&a[2])?;
                let allow = self.expr(&a[3])?;
                let recv = self.expr(&a[4])?;
                let methods = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(nonces); v.extend(allow);
                v.extend(recv.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(recv); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(methods.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(methods); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(81));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/promise_yield_create" => {
                if a.len() != 4 { return Err("near/promise_yield_create: need 4 args (method, args, gas, weight)".into()); }
                let method = self.expr(&a[0])?;
                let args = self.expr(&a[1])?;
                let gas = self.expr(&a[2])?;
                let weight = self.expr(&a[3])?;
                let mut v = Vec::new();
                v.extend(method.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(gas); v.extend(weight);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(82));
                v.extend(self.emit_tag_num()); Ok(v)
            }
            "near/promise_yield_resume" => {
                if a.len() != 2 { return Err("near/promise_yield_resume: need 2 args (data_id, payload)".into()); }
                let data_id = self.expr(&a[0])?;
                let payload = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(data_id.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data_id); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(payload.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(payload); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(83));
                v.extend(self.emit_tag_num()); Ok(v)
            }
            // near/transfer: (near/transfer account_id amount_yocto) -> nil
            // High-level transfer: creates promise batch, adds transfer action
            "near/transfer" => {
                if a.len() != 2 { return Err("near/transfer: need 2 args (account_id, amount_yocto)".into()); }
                let acct = self.expr(&a[0])?;
                let amt = self.expr(&a[1])?;
                let amt_local = self.local_idx("__xfr_amt");
                let mut v = Vec::new();
                // Save amount pointer to local
                v.extend(amt);
                v.push(Instruction::LocalSet(amt_local));
                // Copy u128 from heap to AMOUNT_MEM
                // I64Store: [dest:i32, value:i64] -> [] (value on TOP)
                // Low 64 bits: load from ptr[0], store to AMOUNT_MEM[0]
                v.push(Instruction::I32Const(AMOUNT_MEM as i32));  // dest addr (BOTTOM)
                v.push(Instruction::LocalGet(amt_local)); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);  // src addr
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Stack: [dest, value]
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // High 64 bits: load from ptr[8], store to AMOUNT_MEM[8]
                v.push(Instruction::I32Const((AMOUNT_MEM + 8) as i32));  // dest addr
                v.push(Instruction::LocalGet(amt_local)); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // promise_batch_create(account_len, account_ptr) -> promise_idx
                v.extend(acct.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(acct); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(39)); v.push(Instruction::Drop);
                // promise_batch_action_transfer(promise_idx=0, amount_ptr=AMOUNT_MEM, amount_len=16)
                v.push(Instruction::I64Const(0)); // promise_idx (just created)
                v.push(Instruction::I64Const(AMOUNT_MEM)); // amount_ptr as i64
                v.push(Instruction::I64Const(16)); // amount_len
                v.push(Self::host_call(44)); // returns void
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
