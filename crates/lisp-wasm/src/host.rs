use crate::emit::WasmEmitter;
use crate::emit::TEMP_MEM;
use lisp_core::types::LispVal;
use wasm_encoder::Instruction;

impl WasmEmitter {
    pub(crate) fn scan_host(&mut self, e: &LispVal) {
        let LispVal::List(items) = e else { return };
        for i in items { self.scan_host(i) }
        if items.is_empty() { return }
        let LispVal::Sym(op) = &items[0] else { return };
        match op.as_str() {
            "near/store" => { self.need_host(17); self.need_host(18); self.need_host(0); self.need_host(1); }
            "near/load" => { self.need_host(18); self.need_host(0); self.need_host(1); }
            "near/remove" => { self.need_host(19); }
            "near/has_key" => { self.need_host(20); }
            "near/return" => self.need_host(25),
            "near/log" => self.need_host(28),
            "near/panic" => self.need_host(27),
            "near/current_account_id" => { self.need_host(3); self.need_host(0); self.need_host(1); }
            "near/signer_account_id" => { self.need_host(4); self.need_host(0); self.need_host(1); }
            "near/predecessor_account_id" => { self.need_host(6); self.need_host(0); self.need_host(1); }
            "near/input" => { self.need_host(7); self.need_host(0); self.need_host(1); }
            "near/block_index" => self.need_host(8),
            "near/block_timestamp" => self.need_host(9),
            "near/epoch_height" => self.need_host(10),
            "near/attached_deposit" => { self.need_host(14); self.need_host(0); }
            "near/attached_deposit_high" => { self.need_host(14); self.need_host(0); }
            "near/prepaid_gas" => self.need_host(15),
            "near/used_gas" => self.need_host(16),
            "near/account_balance" => { self.need_host(12); self.need_host(0); self.need_host(1); }
            "near/sha256" => { self.need_host(21); self.need_host(0); self.need_host(1); }
            "near/random_seed" => { self.need_host(23); self.need_host(0); self.need_host(1); }
            "near/promise_create" => self.need_host(30),
            "near/promise_then" => { self.need_host(31); }
            "near/promise_and" => self.need_host(32),
            "near/promise_results_count" => self.need_host(33),
            "near/promise_return" => self.need_host(35),
            "near/promise_batch_create" => self.need_host(39),
            "near/promise_batch_then" => self.need_host(40),
            "near/promise_batch_action_create_account" => self.need_host(41),
            "near/promise_batch_action_deploy_contract" => self.need_host(42),
            "near/promise_batch_action_function_call" => self.need_host(43),
            "near/promise_batch_action_transfer" => self.need_host(44),
            "near/promise_batch_action_stake" => self.need_host(45),
            "near/promise_batch_action_add_key_with_full_access" => self.need_host(46),
            "near/promise_batch_action_add_key_with_function_call" => self.need_host(47),
            "near/promise_batch_action_delete_key" => self.need_host(48),
            "near/promise_batch_action_delete_account" => self.need_host(49),
            "near/abort" => self.need_host(26),
            "near/storage_set" => { self.need_host(17); }
            "near/storage_get" => { self.need_host(18); self.need_host(0); }
            "near/storage_has" => { self.need_host(20); }
            "near/storage_remove" => { self.need_host(19); }
            "near/log_num" => self.need_host(28),
            "near/json_get_int" | "near/json_get_str" | "near/json_get_u128" => { self.need_host(7); self.need_host(0); self.need_host(1); }
            "u128/store_storage" => { self.need_host(17); }
            "u128/load_storage" => { self.need_host(18); self.need_host(0); }
            "near/json_return_int" | "near/json_return_str" => self.need_host(25),
            "near/iter_prefix" => { self.need_host(36); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_range" => { self.need_host(37); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_next" => { self.need_host(38); self.need_host(0); self.need_host(1); }
            _ => {}
        }
    }

    // ── Public API ──


    pub(crate) fn read_to_register(&mut self, host_idx: usize, _a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        v.push(Instruction::I64Const(0)); // register_id=0
        v.push(Self::host_call(host_idx));
        // read_register(0, TEMP_MEM)
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(TEMP_MEM));
        v.push(Self::host_call(0));
        // register_len(0)
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(1));
        // Pack: (len << 32) | TEMP_MEM
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(TEMP_MEM));
        v.push(Instruction::I64Or);
        Ok(v)
    }

    // Helper: call host(register_id=0) writing u128 to register, read to mem, return low 64 bits
    pub(crate) fn read_u128_low(&mut self, host_idx: usize) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        v.push(Instruction::I64Const(0)); // register_id=0
        v.push(Self::host_call(host_idx));
        // read_register(0, 0) — copy 16 bytes to mem[0..16]
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(0));
        // Load low 8 bytes (bytes 0..7) as i64
        v.push(Instruction::I32Const(0));
        v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
        Ok(v)
    }

    // Helper: same but return high 64 bits of u128
    pub(crate) fn read_u128_high(&mut self, host_idx: usize) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        v.push(Instruction::I64Const(0)); // register_id=0
        v.push(Self::host_call(host_idx));
        // read_register(0, 0) — copy 16 bytes to mem[0..16]
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(0));
        // Load high 8 bytes (bytes 8..15) as i64
        v.push(Instruction::I32Const(8));
        v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
        Ok(v)
    }

}
