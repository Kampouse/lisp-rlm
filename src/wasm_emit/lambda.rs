
use super::*;

impl WasmEmitter {
    pub(crate) fn extract_lambda(form: &LispVal) -> Result<(String, LispVal), String> {
        match form {
            LispVal::List(items) if items.len() >= 3 => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "lambda" || s == "fn" {
                        if let LispVal::List(params) = &items[1] {
                            if let Some(LispVal::Sym(p)) = params.first() {
                                let body = if items.len() > 3 {
                                    LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(items[2..].iter().cloned()).collect())
                                } else { items[2].clone() };
                                return Ok((p.clone(), body));
                            }
                        }
                        if let LispVal::Sym(p) = &items[1] {
                            let body = if items.len() > 3 {
                                LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                    .chain(items[2..].iter().cloned()).collect())
                            } else { items[2].clone() };
                            return Ok((p.clone(), body));
                        }
                    }
                }
                Err(format!("hof: expected (lambda (param) body), got {:?}", form))
            }
            _ => Err(format!("hof: expected lambda form, got {:?}", form)),
        }
    }

    pub(crate) fn extract_lambda_2param(form: &LispVal) -> Result<(Vec<String>, LispVal), String> {
        match form {
            LispVal::List(items) if items.len() >= 3 => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "lambda" || s == "fn" {
                        if let LispVal::List(params) = &items[1] {
                            let names: Vec<String> = params.iter()
                                .filter_map(|p| if let LispVal::Sym(s) = p { Some(s.clone()) } else { None })
                                .collect();
                            if names.len() == 2 {
                                let body = if items.len() > 3 {
                                    LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(items[2..].iter().cloned()).collect())
                                } else { items[2].clone() };
                                return Ok((names, body));
                            }
                        }
                    }
                }
                Err(format!("hof/reduce: expected (lambda (acc x) body), got {:?}", form))
            }
            _ => Err(format!("hof/reduce: expected lambda form, got {:?}", form)),
        }
    }

    // ── Tail-call detection ──


    pub(crate) fn parse_u128(s: &str) -> Result<(i64, i64), String> {
        let mut lo: u64 = 0;
        let mut hi: u64 = 0;
        for ch in s.chars() {
            if ch == '_' { continue; }
            if ch < '0' || ch > '9' { return Err(format!("invalid digit in u128 literal: '{}'", ch)); }
            let digit = ch as u64 - '0' as u64;
            let old_hi = hi as u128;
            let old_lo = lo as u128;
            let new_val = old_hi * (1u128 << 64) + old_lo;
            let new_val = new_val * 10 + digit as u128;
            lo = new_val as u64;
            hi = (new_val >> 64) as u64;
        }
        Ok((lo as i64, hi as i64))
    }


    pub(crate) fn has_tc(&self, e: &LispVal) -> bool {
        let LispVal::List(items) = e else { return false };
        if items.is_empty() { return false }
        let LispVal::Sym(op) = &items[0] else { return false };
        let a = &items[1..];
        if Some(op.as_str()) == self.current_func.as_deref() && a.len() == self.current_param_count { return true }
        if op == "if" { return self.has_tc(&a[1]) || (a.len() > 2 && self.has_tc(&a[2])) }
        if op == "begin" && !a.is_empty() { return self.has_tc(items.last().unwrap()) }
        if op == "let" && a.len() > 1 { return a[1..].iter().any(|x| self.has_tc(x)) }
        false
    }

    pub(crate) fn is_self(&self, e: &LispVal) -> bool {
        let LispVal::List(items) = e else { return false };
        if items.len() < 2 { return false }
        let LispVal::Sym(op) = &items[0] else { return false };
        Some(op.as_str()) == self.current_func.as_deref() && items.len() - 1 == self.current_param_count
    }

    pub(crate) fn free_vars(&self, e: &LispVal, bound: &HashSet<String>) -> HashSet<String> {
        let mut free = HashSet::new();
        self.collect_free(e, bound, &mut free);
        free
    }

    pub(crate) fn collect_free(&self, e: &LispVal, bound: &HashSet<String>, free: &mut HashSet<String>) {
        match e {
            LispVal::Sym(s) => {
                if !bound.contains(s) && self.locals.contains_key(s) && !self.funcs.iter().any(|f| f.name == *s) {
                    free.insert(s.clone());
                }
            }
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(op) = &items[0] {
                    match op.as_str() {
                        "lambda" => {
                            if items.len() >= 3 {
                                if let LispVal::List(params) = &items[1] {
                                    let mut inner_bound = bound.clone();
                                    for p in params {
                                        if let LispVal::Sym(s) = p { inner_bound.insert(s.clone()); }
                                    }
                                    // Collect free vars from all body expressions
                                    for body_expr in &items[2..] {
                                        self.collect_free(body_expr, &inner_bound, free);
                                    }
                                }
                            }
                            return;
                        }
                        "let" | "let*" => {
                            if items.len() >= 3 {
                                if let LispVal::List(bindings) = &items[1] {
                                    let mut inner_bound = bound.clone();
                                    for b in bindings {
                                        if let LispVal::List(pair) = b {
                                            if let LispVal::Sym(s) = &pair[0] {
                                                inner_bound.insert(s.clone());
                                                if pair.len() > 1 { self.collect_free(&pair[1], bound, free); }
                                            }
                                        }
                                    }
                                    // Collect free vars from all body expressions
                                    for body_expr in &items[2..] {
                                        self.collect_free(body_expr, &inner_bound, free);
                                    }
                                }
                                return;
                            }
                        }
                        "define" => return, // don't look inside nested defines
                        _ => {}
                    }
                }
                for item in items { self.collect_free(item, bound, free); }
            }
            _ => {}
        }
    }

    pub(crate) fn emit_lambda(&mut self, params: &[String], body: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let lambda_id = self.lambda_counter as usize;
        self.lambda_counter += 1;
        
        // Find free variables
        let param_set: HashSet<String> = params.iter().cloned().collect();
        let free = self.free_vars(body, &param_set);
        let captured: Vec<String> = free.into_iter().collect();
        let captured_count = captured.len();
        
        // Generate hidden function name
        let name = format!("__lambda_{}", lambda_id);
        
        // Save state
        let saved_func = self.current_func.take();
        let saved_param_count = self.current_param_count;
        let saved_locals = self.locals.clone();
        let saved_next_local = self.next_local;
        let saved_captured_map = self.captured_map.clone();
        
        // Set up lambda function
        self.locals.clear();
        self.next_local = 0;
        self.captured_map.clear();
        let _env_idx = self.local_idx("__closure_ptr"); // first param: closure pointer
        for p in params { self.local_idx(p); }
        self.current_func = Some(name.clone());
        self.current_param_count = params.len() + 1; // +1 for closure ptr
        // Set up captured var map: var_name -> offset in closure (1-indexed, [0] is lambda_id)
        for (i, cap) in captured.iter().enumerate() {
            self.captured_map.insert(cap.clone(), i + 1); // offset 1, 2, 3...
        }
        self.scan_host(body);
        
        // Pre-insert placeholder
        let total_params = params.len() + 1;
        let placeholder_idx = self.funcs.len();
        self.funcs.push(FuncDef { name: name.clone(), param_count: total_params, local_count: 0, instrs: Vec::new(), local_entries: None });
        
        let instrs = self.expr(body)?;
        let total_locals = self.next_local as usize;
        self.funcs[placeholder_idx] = FuncDef { name: name.clone(), param_count: total_params, local_count: total_locals, instrs, local_entries: None };
        
        // Record lambda info
        self.lambda_info.push((placeholder_idx, captured_count));
        
        // Restore state
        self.current_func = saved_func;
        self.current_param_count = saved_param_count;
        self.locals = saved_locals;
        self.next_local = saved_next_local;
        self.captured_map = saved_captured_map;
        
        // Build closure value: allocate heap memory [fn_idx, cap1, cap2, ...]
        let mut v = Vec::new();
        if captured.is_empty() {
            // No captures → direct fn ref
            // Value: (lambda_id << TAG_BITS) | TAG_FNREF
            v.push(Instruction::I64Const(((lambda_id as i64) << TAG_BITS) | TAG_FNREF));
        } else {
            // Allocate closure on heap: [lambda_id, captured_val_1, captured_val_2, ...]
            let closure_size = (1 + captured_count) as u32; // i64 slots
            let ptr = self.heap_ptr;
            self.heap_ptr += closure_size * 8;
            
            // Store lambda_id at closure[0]
            v.push(Instruction::I64Const(ptr as i64));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Const(lambda_id as i64));
            let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
            v.push(Instruction::I64Store(ma));
            
            // Store each captured value (self.locals is restored to enclosing scope at this point)
            for (i, cap) in captured.iter().enumerate() {
                let &local_idx = self.locals.get(cap).ok_or_else(|| format!("lambda capture: undef local {}", cap))?;
                v.push(Instruction::I64Const((ptr + ((i as u32 + 1) * 8)) as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(local_idx));
                v.push(Instruction::I64Store(ma));
            }
            
            // Return closure ptr tagged
            v.push(Instruction::I64Const(((ptr as i64) << TAG_BITS) | TAG_CLOSURE));
        }
        Ok(v)
    }

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
            "near/attached_deposit" => self.need_host(14),
            "near/prepaid_gas" => self.need_host(15),
            "near/used_gas" => self.need_host(16),
            "near/sha256" => { self.need_host(21); self.need_host(0); self.need_host(1); }
            "near/keccak256" => { self.need_host(22); self.need_host(0); self.need_host(1); }
            "near/ed25519_verify" => self.need_host(24),
            "near/p256_verify" => self.need_host(55),
            "near/signer_account_pk" => { self.need_host(5); self.need_host(0); self.need_host(1); }
            "near/storage_usage" => self.need_host(11),
            "near/account_balance" => self.need_host(12),
            "near/account_balance_high" => self.need_host(12),
            "near/account_locked_balance" => self.need_host(13),
            "near/account_locked_balance_high" => self.need_host(13),
            "near/attached_deposit_high" => self.need_host(14),
            "near/log_utf16" => self.need_host(29),
            "near/random_seed" => { self.need_host(23); self.need_host(0); self.need_host(1); }
            "near/promise_create" => self.need_host(30),
            "near/promise_then" => { self.need_host(31); }
            "near/promise_and" => self.need_host(32),
            "near/promise_results_count" => self.need_host(33),
            "near/promise_return" => self.need_host(35),
            "near/call" => { self.need_host(30); self.need_host(35); }
            "near/promise_result" => { self.need_host(34); self.need_host(0); self.need_host(1); }
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
            "print" | "println" => { if !self.wasi_mode { self.need_host(28); } }
            "near/json_get_int" | "near/json_get_str" | "near/json_get_u128" | "json-get" | "json-get-str" | "json-get-float" | "json/get" => { if !self.wasi_mode { self.need_host(7); self.need_host(0); self.need_host(1); } }
            "u128/store_storage" => { self.need_host(17); }
            "u128/load_storage" => { self.need_host(18); self.need_host(0); }
            "near/json_return_int" | "near/json_return_str" | "json-return" => { if !self.wasi_mode { self.need_host(25); } },
            "borsh-serialize" | "borsh-deserialize" | "array" => { /* pure WASM, no host fns needed */ },
            "near/iter_prefix" => { self.need_host(36); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_range" => { self.need_host(37); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_next" => { self.need_host(38); self.need_host(0); self.need_host(1); }
            // Global contracts
            "near/deploy_contract" => self.need_host(50),
            "near/current_code_hash" => { self.need_host(51); self.need_host(0); }
            // Extra crypto
            "near/keccak512" => { self.need_host(52); self.need_host(0); self.need_host(1); }
            "near/ripemd160" => { self.need_host(53); self.need_host(0); self.need_host(1); }
            // Deposit check: writes attached_deposit to TEMP_MEM, compares u128 against compile-time (lo, hi)
            "near/deposit-gte" => { if !self.wasi_mode { self.need_host(14); } }
            "near/ecrecover" => self.need_host(54),
            "near/p256_verify" => self.need_host(55),
            // Alt BN128
            "near/alt_bn128_g1_multiexp" => { self.need_host(56); self.need_host(0); }
            "near/alt_bn128_g1_sum" => { self.need_host(57); self.need_host(0); }
            "near/alt_bn128_pairing_check" => self.need_host(58),
            // BLS12-381
            "near/bls12381_p1_sum" => { self.need_host(59); self.need_host(0); }
            "near/bls12381_p2_sum" => { self.need_host(60); self.need_host(0); }
            "near/bls12381_g1_multiexp" => { self.need_host(61); self.need_host(0); }
            "near/bls12381_g2_multiexp" => { self.need_host(62); self.need_host(0); }
            "near/bls12381_map_fp_to_g1" => { self.need_host(63); self.need_host(0); }
            "near/bls12381_map_fp2_to_g2" => { self.need_host(64); self.need_host(0); }
            "near/bls12381_pairing_check" => self.need_host(65),
            "near/bls12381_p1_decompress" => { self.need_host(66); self.need_host(0); }
            "near/bls12381_p2_decompress" => { self.need_host(67); self.need_host(0); }
            // Extra promises
            "near/promise_set_refund_to" => self.need_host(68),
            "near/promise_batch_action_state_init" => self.need_host(69),
            "near/promise_batch_action_state_init_by_account_id" => self.need_host(70),
            "near/set_state_init_data_entry" => self.need_host(71),
            "near/current_contract_code" => { self.need_host(72); self.need_host(0); }
            "near/refund_to_account_id" => { self.need_host(73); self.need_host(0); }
            "near/promise_batch_action_function_call_weight" => self.need_host(74),
            "near/promise_batch_action_deploy_global_contract" => self.need_host(75),
            "near/promise_batch_action_deploy_global_contract_by_account_id" => self.need_host(76),
            "near/promise_batch_action_use_global_contract" => self.need_host(77),
            "near/promise_batch_action_use_global_contract_by_account_id" => self.need_host(78),
            "near/promise_batch_action_transfer_to_gas_key" => self.need_host(79),
            "near/promise_batch_action_add_gas_key_with_full_access" => self.need_host(80),
            "near/promise_batch_action_add_gas_key_with_function_call" => self.need_host(81),
            "near/promise_yield_create" => self.need_host(82),
            "near/promise_yield_resume" => self.need_host(83),
            // Validator
            "near/validator_stake" => self.need_host(84),
            "near/validator_total_stake" => self.need_host(85),
            // OutLayer RPC — uses "outlayer" module imports
            "outlayer/view" | "outlayer/raw" | "outlayer/status" |
            "outlayer/storage-set" | "outlayer/storage-get" | "outlayer/storage-has" | "outlayer/storage-delete" |
            "outlayer/context" |
            "storage-set" | "storage-get" | "storage-has" | "storage-delete" | "storage-increment" |
            "env/signer" | "env/predecessor" |
            "storage-decrement" | "storage-set-if-absent" | "storage-set-if-equals" |
            "storage-list-keys" | "storage-clear-all" |
            "storage-set-worker" | "storage-get-worker" | "storage-set-worker-public" | "storage-get-worker-from-project" => {
                self.need_outlayer = true;
            }
            "outlayer/http-post" => {
                self.need_outlayer = true;
            }
            "http-get" | "http-post" => {
                if self.p2_mode {
                    // Direct wasi:http — emit canonical ABI calls, no adapter needed
                    self.need_wasi_http = true;
                } else {
                    self.need_outlayer = true;
                }
            }
            // FP-allocating builtins (need frame save/restore in NEAR mode)
            "str-cat" | "str-concat" | "string-append" | "str-slice" | "near/load-bytes" | "u32-to-bytes" | "near/store-bytes" => {
                if !self.wasi_mode && !self.p2_mode { self.needs_frame = true; }
            }
            _ => {}
        }
    }

    pub(crate) fn resolve_lambda_1(&self, arg: &LispVal, ctx: &str) -> Result<(String, LispVal), String> {
        match arg {
            // Inline lambda: (fn [x] body) or (fn x body)
            LispVal::List(items) if items.len() >= 3 && matches!(&items[0], LispVal::Sym(s) if s == "fn" || s == "lambda") => {
                let pname = match &items[1] {
                    LispVal::Sym(s) => s.clone(),
                    LispVal::List(ps) if !ps.is_empty() => match &ps[0] { LispVal::Sym(s) => s.clone(), _ => "x".into() },
                    _ => "x".into(),
                };
                Ok((pname, items[2].clone()))
            },
            // Named function symbol — look up in func_defs
            LispVal::Sym(name) => {
                let (params, body) = self.func_defs.get(name)
                    .ok_or_else(|| format!("{}: unknown function '{}'", ctx, name))?;
                if params.len() != 1 {
                    return Err(format!("{}: '{}' takes {} params, need exactly 1", ctx, name, params.len()));
                }
                Ok((params[0].clone(), body.clone()))
            },
            _ => Err(format!("{}: first arg must be (fn [x] body) or named function", ctx)),
        }
    }

    /// Resolve a 2-param lambda arg: inline (fn [a b] body) or named function symbol.
    /// Returns (param1_name, param2_name, body_ast).

    pub(crate) fn resolve_lambda_2(&self, arg: &LispVal, ctx: &str) -> Result<(String, String, LispVal), String> {
        match arg {
            LispVal::List(items) if items.len() >= 3 && matches!(&items[0], LispVal::Sym(s) if s == "fn" || s == "lambda") => {
                let (an, en) = match &items[1] {
                    LispVal::List(ps) if ps.len() >= 2 => {
                        let an = match &ps[0] { LispVal::Sym(s) => s.clone(), _ => "a".into() };
                        let en = match &ps[1] { LispVal::Sym(s) => s.clone(), _ => "b".into() };
                        (an, en)
                    },
                    _ => ("a".into(), "b".into()),
                };
                Ok((an, en, items[2].clone()))
            },
            LispVal::Sym(name) => {
                let (params, body) = self.func_defs.get(name)
                    .ok_or_else(|| format!("{}: unknown function '{}'", ctx, name))?;
                if params.len() != 2 {
                    return Err(format!("{}: '{}' takes {} params, need exactly 2", ctx, name, params.len()));
                }
                Ok((params[0].clone(), params[1].clone(), body.clone()))
            },
            _ => Err(format!("{}: first arg must be (fn [a b] body) or named function", ctx)),
        }
    }

}
