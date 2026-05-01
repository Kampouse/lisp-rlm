//! Binary WASM emitter via wasm-encoder. No string-based WAT.
//!
//! NEAR constraints baked in:
//! - All exports: () -> ()
//! - value_return (not return_value)
//! - Exact host function signatures
//! - Only import what's used
//! - Imports before memory

use crate::types::LispVal;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

// ── Lightweight type system for typechecking ──

#[derive(Debug, Clone, PartialEq)]
enum Ty {
    Num,
    Bool,
    Str,
    Void,
    Any,   // unknown / not checkable
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Ty::Num => write!(f, "number"),
            Ty::Bool => write!(f, "bool"),
            Ty::Str => write!(f, "string"),
            Ty::Void => write!(f, "void"),
            Ty::Any => write!(f, "any"),
        }
    }
}
use wasm_encoder::{
    BlockType, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

// ── NEAR host functions (name, params, results) ──

// NEAR host function signatures — from nearcore imports.rs, verified on testnet Apr 30 2026
// Format: (wasm_name, params, results)
// [] = void return, [I64] = returns u64
const HOST_FUNCS: &[(&str, &[ValType], &[ValType])] = &[
    // Registers
    ("read_register",               &[ValType::I64, ValType::I64], &[]),           // 0
    ("register_len",                &[ValType::I64], &[ValType::I64]),             // 1
    ("write_register",              &[ValType::I64, ValType::I64, ValType::I64], &[]), // 2
    // Context
    ("current_account_id",          &[ValType::I64], &[]),                         // 3
    ("signer_account_id",           &[ValType::I64], &[]),                         // 4
    ("signer_account_pk",           &[ValType::I64], &[]),                         // 5
    ("predecessor_account_id",      &[ValType::I64], &[]),                         // 6
    ("input",                       &[ValType::I64], &[]),                         // 7
    ("block_index",                 &[], &[ValType::I64]),                         // 8
    ("block_timestamp",             &[], &[ValType::I64]),                         // 9
    ("epoch_height",                &[], &[ValType::I64]),                         // 10
    ("storage_usage",               &[], &[ValType::I64]),                         // 11
    // Economics
    ("account_balance",             &[ValType::I64], &[]),                         // 12
    ("account_locked_balance",      &[ValType::I64], &[]),                         // 13
    ("attached_deposit",            &[ValType::I64], &[]),                         // 14
    ("prepaid_gas",                 &[], &[ValType::I64]),                         // 15
    ("used_gas",                    &[], &[ValType::I64]),                         // 16
    // Storage
    ("storage_write",               &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 17
    ("storage_read",                &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),     // 18
    ("storage_remove",              &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),     // 19
    ("storage_has_key",             &[ValType::I64, ValType::I64], &[ValType::I64]),                   // 20
    // Math / Crypto
    ("sha256",                      &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 21
    ("keccak256",                   &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 22
    ("random_seed",                 &[ValType::I64], &[]),                                          // 23
    ("ed25519_verify",              &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 24
    // Misc
    ("value_return",                &[ValType::I64, ValType::I64], &[]),            // 25
    ("panic",                       &[], &[]),                                      // 26
    ("panic_utf8",                  &[ValType::I64, ValType::I64], &[]),            // 27
    ("log_utf8",                    &[ValType::I64, ValType::I64], &[]),            // 28
    ("log_utf16",                   &[ValType::I64, ValType::I64], &[]),            // 29
    // Promises (core)
    ("promise_create",              &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 30
    ("promise_then",                &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 31
    ("promise_and",                 &[ValType::I64, ValType::I64], &[ValType::I64]), // 32
    ("promise_results_count",       &[], &[ValType::I64]),                          // 33
    ("promise_result",              &[ValType::I64, ValType::I64], &[]),            // 34
    ("promise_return",              &[ValType::I64], &[]),                          // 35
    // Iterator
    ("storage_iter_prefix",         &[ValType::I64, ValType::I64], &[ValType::I64]), // 36
    ("storage_iter_range",          &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 37
    ("storage_iter_next",           &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 38
    // Promise batch
    ("promise_batch_create",        &[ValType::I64, ValType::I64], &[ValType::I64]),              // 39
    ("promise_batch_then",          &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 40
    ("promise_batch_action_create_account", &[ValType::I64], &[]),                                // 41
    ("promise_batch_action_deploy_contract", &[ValType::I64, ValType::I64, ValType::I64], &[]),    // 42
    ("promise_batch_action_function_call", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 43
    ("promise_batch_action_transfer", &[ValType::I64, ValType::I64, ValType::I64], &[]),            // 44
    ("promise_batch_action_stake",  &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 45
    ("promise_batch_action_add_key_with_full_access", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 46
    ("promise_batch_action_add_key_with_function_call", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 47
    ("promise_batch_action_delete_key", &[ValType::I64, ValType::I64, ValType::I64], &[]),          // 48
    ("promise_batch_action_delete_account", &[ValType::I64, ValType::I64, ValType::I64], &[]),      // 49
];

const HOST_BASE: u32 = 0xFF00_0000;
const USER_BASE: u32 = 0xFF01_0000;
const TEMP_MEM: i64 = 64;
const STORAGE_BUF: i64 = 8192;  // 8 bytes for storage read/write buffer
const STORAGE_U128_BUF: i64 = 8208;  // 16 bytes for u128 storage ops
// ~300 Tgas on NEAR ≈ ~10B simple ops. Cap at 1B to be safe (stops runaway, still uses full NEAR runtime).
const GAS_LIMIT: i64 = 1_000_000_000;
const DEPTH_LIMIT: i64 = 512;
const DEPTH_GLOBAL: u32 = 0; // mutable i64 global for call depth

struct FuncDef {
    name: String,
    param_count: usize,
    local_count: usize,
    instrs: Vec<Instruction<'static>>,
}

pub struct WasmEmitter {
    locals: HashMap<String, u32>,
    next_local: u32,
    current_func: Option<String>,
    current_param_count: usize,
    while_id: Cell<usize>,
    funcs: Vec<FuncDef>,
    memory_pages: u32,
    exports: Vec<(String, String, bool)>,
    data_segments: Vec<(u32, Vec<u8>)>,
    next_data_offset: u32,
    host_needed: HashSet<usize>,
    gas_local: Option<u32>, // index of the gas counter local (i64)
}

impl WasmEmitter {
    pub fn new() -> Self {
        Self {
            locals: HashMap::new(), next_local: 0, current_func: None, current_param_count: 0,
            while_id: Cell::new(0), funcs: Vec::new(), memory_pages: 1, exports: Vec::new(),
            data_segments: Vec::new(), next_data_offset: 256, host_needed: HashSet::new(),
            gas_local: None,
        }
    }

    fn local_idx(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.locals.get(name) { return i; }
        let i = self.next_local;
        self.locals.insert(name.to_string(), i);
        self.next_local += 1;
        i
    }

    /// Format a helpful error for undefined variables
    fn fmt_undef_error(&self, name: &str) -> String {
        // Known internal variable mappings
        let context: Option<String> = match name {
            "__hof_it" | "__it" => Some("this is the loop variable in hof/map. Your lambda body references 'it' which maps to this.".into()),
            "__hof_count" => Some("this is the loop count variable in hof/map.".into()),
            n if n.starts_with("__logn_") => Some("this is an internal variable used by near/log_num, not accessible from user code.".into()),
            n if n.starts_with("__clog_") => Some("this is an internal variable used by near/log (combined logging), not accessible from user code.".into()),
            n if n.starts_with("__") => Some("this is an internal compiler variable, not accessible from user code.".into()),
            _ => None,
        };

        let mut msg = format!("Undefined variable '{}'", name);

        if let Some(ctx) = context {
            msg.push_str(&format!("\n  Note: {}", ctx));
        }

        // Suggest closest matching user-visible local
        let candidates: Vec<&str> = self.locals.keys()
            .filter(|k| !k.starts_with("__"))
            .map(String::as_str)
            .collect();

        if !candidates.is_empty() {
            let mut best: Option<(&str, usize)> = None;
            for c in &candidates {
                let dist = levenshtein(name, c);
                if dist <= 3 {
                    match best {
                        Some((_, best_dist)) if dist >= best_dist => {}
                        _ => best = Some((c, dist)),
                    }
                }
            }
            if let Some((suggestion, _)) = best {
                msg.push_str(&format!("\n  Did you mean '{}'?", suggestion));
            }
        }

        msg
    }

    fn alloc_data(&mut self, bytes: &[u8]) -> u32 {
        let off = self.next_data_offset;
        self.data_segments.push((off, bytes.to_vec()));
        self.next_data_offset += bytes.len() as u32;
        self.next_data_offset = (self.next_data_offset + 7) & !7;
        off
    }

    /// Extract (lambda (param) body) → (param_name, body_expr)
    fn extract_lambda(form: &LispVal) -> Result<(String, LispVal), String> {
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

    /// Extract (lambda (p1 p2) body) → (vec![p1, p2], body)
    fn extract_lambda_2param(form: &LispVal) -> Result<(Vec<String>, LispVal), String> {
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

    fn need_host(&mut self, idx: usize) { self.host_needed.insert(idx); }

    fn host_call(idx: usize) -> Instruction<'static> {
        Instruction::Call(HOST_BASE | idx as u32)
    }

    fn parse_u128(s: &str) -> Result<(i64, i64), String> {
        let mut lo: u64 = 0;
        let mut hi: u64 = 0;
        for ch in s.chars() {
            if ch == '_' { continue; }
            if ch < '0' || ch > '9' { return Err(format!("invalid digit in u128 literal: '{}'", ch)); }
            let digit = ch as u64 - '0' as u64;
            // hi:lo = hi:lo * 10 + digit
            let old_hi = hi as u128;
            let old_lo = lo as u128;
            let new_val = old_hi * (1u128 << 64) + old_lo;
            let new_val = new_val * 10 + digit as u128;
            lo = new_val as u64;
            hi = (new_val >> 64) as u64;
        }
        Ok((lo as i64, hi as i64))
    }

    // ── Tail-call detection ──

    fn has_tc(&self, e: &LispVal) -> bool {
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

    fn is_self(&self, e: &LispVal) -> bool {
        let LispVal::List(items) = e else { return false };
        if items.len() < 2 { return false }
        let LispVal::Sym(op) = &items[0] else { return false };
        Some(op.as_str()) == self.current_func.as_deref() && items.len() - 1 == self.current_param_count
    }

    // ── Lightweight typecheck pre-pass ──

    /// Typecheck a function body. Called in `emit_define` before codegen.
    /// `func_name` is the name of the function being compiled, `params` are its parameter names.
    /// `funcs` provides the list of all defined functions so far (for checking calls).
    pub fn typecheck_expr(funcs: &Vec<FuncDef>, params: &[String], body: &LispVal) -> Result<(), String> {
        let mut tc = TypeChecker { funcs, local_names: params.iter().cloned().collect() };
        tc.check(body)?;
        Ok(())
    }

    fn scan_host(&mut self, e: &LispVal) {
        let LispVal::List(items) = e else { return };
        for i in items { self.scan_host(i) }
        if items.is_empty() { return }
        let LispVal::Sym(op) = &items[0] else { return };
        match op.as_str() {
            "near/store" | "near/storage_set" => { self.need_host(17); }
            "near/load" | "near/storage_get" => { self.need_host(18); self.need_host(0); }
            "near/remove" | "near/storage_remove" => { self.need_host(19); }
            "near/has_key" | "near/storage_has" => { self.need_host(20); }
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
            "u128/store_storage" => { self.need_host(17); }
            "u128/load_storage" => { self.need_host(18); self.need_host(0); }
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
            "near/iter_prefix" => { self.need_host(36); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_range" => { self.need_host(37); self.need_host(2); self.need_host(0); self.need_host(1); }
            "near/iter_next" => { self.need_host(38); self.need_host(0); self.need_host(1); }
            _ => {}
        }
    }

    // ── Public API ──

    pub fn emit_define(&mut self, name: &str, params: &[String], body: &LispVal) -> Result<(), String> {
        self.locals.clear(); self.next_local = 0;
        for p in params { self.local_idx(p); }
        self.current_func = Some(name.to_string());
        self.current_param_count = params.len();
        self.while_id.set(0);
        self.scan_host(body);

        // Allocate gas local and return-value local
        let gas_local = self.local_idx("__gas");
        let ret_local = self.local_idx("__ret");
        self.gas_local = Some(gas_local);

        // Pre-insert placeholder so self-recursion resolves
        let placeholder_idx = self.funcs.len();
        self.funcs.push(FuncDef { name: name.into(), param_count: params.len(), local_count: 0, instrs: Vec::new() });

        // Run typecheck pre-pass (catches obvious errors before codegen)
        if !name.starts_with("__") {
            if let Err(e) = Self::typecheck_expr(&self.funcs, params, body) {
                // Remove placeholder before returning error
                self.funcs.pop();
                return Err(format!("in function '{}': {}", name, e));
            }
        }

        let tc = self.has_tc(body);

        // Build prologue: init gas + depth increment/check
        let mut prologue = Vec::new();
        // gas = GAS_LIMIT
        prologue.push(Instruction::I64Const(GAS_LIMIT));
        prologue.push(Instruction::LocalSet(gas_local));
        // depth++
        prologue.push(Instruction::GlobalGet(DEPTH_GLOBAL));
        prologue.push(Instruction::I64Const(1));
        prologue.push(Instruction::I64Add);
        prologue.push(Instruction::GlobalSet(DEPTH_GLOBAL));
        // if depth > DEPTH_LIMIT: trap
        prologue.push(Instruction::GlobalGet(DEPTH_GLOBAL));
        prologue.push(Instruction::I64Const(DEPTH_LIMIT));
        prologue.push(Instruction::I64GtS);
        // I64GtS produces i32, use directly for If
        prologue.push(Instruction::If(BlockType::Empty));
        prologue.push(Instruction::Unreachable);
        prologue.push(Instruction::End);

        // Build body
        let mut body_instrs = if tc { self.tc_body(body)? } else { self.expr(body)? };

        // Epilogue: save return, depth--, restore return
        let mut epilogue = Vec::new();
        epilogue.push(Instruction::LocalSet(ret_local));
        // depth--
        epilogue.push(Instruction::GlobalGet(DEPTH_GLOBAL));
        epilogue.push(Instruction::I64Const(1));
        epilogue.push(Instruction::I64Sub);
        epilogue.push(Instruction::GlobalSet(DEPTH_GLOBAL));
        epilogue.push(Instruction::LocalGet(ret_local));

        // Combine: prologue + body + epilogue
        let mut instrs = prologue;
        instrs.append(&mut body_instrs);
        instrs.append(&mut epilogue);

        // Inject gas checks before every Br(0) back-edge and host_call
        let instrs = Self::inject_gas_checks(instrs, gas_local);

        let total = self.next_local as usize;
        self.current_func = None;
        self.gas_local = None;
        self.funcs[placeholder_idx] = FuncDef { name: name.into(), param_count: params.len(), local_count: total, instrs };
        Ok(())
    }

    /// Generate gas check instructions: gas -= 1; if gas <= 0: unreachable
    fn gas_check_instrs(gas_local: u32) -> Vec<Instruction<'static>> {
        vec![
            Instruction::LocalGet(gas_local),
            Instruction::I64Const(1),
            Instruction::I64Sub,
            Instruction::LocalTee(gas_local),
            Instruction::I64Const(0),
            Instruction::I64LeS,
            // I64LeS produces i32, use directly for If
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
        ]
    }

    /// Post-process: inject gas check before every Br(0) back-edge and host_call
    fn inject_gas_checks(instrs: Vec<Instruction<'static>>, gas_local: u32) -> Vec<Instruction<'static>> {
        let check = Self::gas_check_instrs(gas_local);
        let mut out = Vec::with_capacity(instrs.len() * 2);
        for i in &instrs {
            match i {
                Instruction::Br(0) => { out.extend(check.iter().cloned()); out.push(i.clone()); }
                Instruction::Call(idx) if *idx >= HOST_BASE && *idx < USER_BASE => {
                    out.extend(check.iter().cloned()); out.push(i.clone());
                }
                _ => out.push(i.clone()),
            }
        }
        out
    }

    pub fn set_memory(&mut self, p: u32) { self.memory_pages = p; }
    pub fn add_export(&mut self, fn_: &str, en: &str, is_view: bool) {
        self.exports.push((fn_.into(), en.into(), is_view));
    }

    // ── Tail-call ──

    fn tc_body(&mut self, body: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let inner = self.tc(body)?;
        let mut v = Vec::with_capacity(inner.len() + 5);
        v.push(Instruction::Block(BlockType::Result(ValType::I64)));
        v.push(Instruction::Loop(BlockType::Empty));
        v.extend(inner);
        v.push(Instruction::End);
        v.push(Instruction::I64Const(0));
        v.push(Instruction::Unreachable);
        v.push(Instruction::End);
        Ok(v)
    }

    fn tc(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let LispVal::List(items) = e else { return self.expr(e) };
        if items.is_empty() { return self.expr(e) }
        let LispVal::Sym(op) = &items[0] else { return self.expr(e) };
        let a = &items[1..];
        match op.as_str() {
            "if" => self.tc_if(a),
            "begin" => {
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() { v.extend(self.expr(x)?); if i < a.len()-1 { v.push(Instruction::Drop); } }
                Ok(v)
            }
            "let" => self.tc_let(a),
            _ if Some(op.as_str()) == self.current_func.as_deref() && a.len() == self.current_param_count => {
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() { v.extend(self.expr(x)?); v.push(Instruction::LocalSet(i as u32)); }
                v.push(Instruction::Br(0));
                Ok(v)
            }
            // Any other expression inside TC loop: evaluate and exit block via Br(2)
            _ => { let mut v = self.expr(e)?; v.push(Instruction::Br(2)); Ok(v) }
        }
    }

    fn tc_let(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        if let LispVal::List(bs) = &a[0] {
            for b in bs { if let LispVal::List(p) = b { if p.len()==2 { if let LispVal::Sym(n) = &p[0] {
                let idx = self.local_idx(n);
                v.extend(self.expr(&p[1])?); v.push(Instruction::LocalSet(idx));
            }}}}
        }
        // Implicit begin in let body
        for (i, x) in a[1..].iter().enumerate() {
            if i < a.len() - 2 { v.extend(self.expr(x)?); v.push(Instruction::Drop); }
            else { v.extend(self.tc(x)?); }
        }
        Ok(v)
    }

    fn tc_if(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.push(Instruction::I32WrapI64);
        let t = &a[1]; let default = LispVal::Num(0);
        let e = if a.len()>2 { &a[2] } else { &default };
        match (self.is_self(t), self.is_self(e)) {
            (true, true) => {
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.self_sets(t)?); v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                v.extend(self.self_sets(e)?); v.push(Instruction::Br(0));
                v.push(Instruction::End);
            }
            (true, false) => {
                v.push(Instruction::I32Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.expr(e)?); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                v.extend(self.self_sets(t)?); v.push(Instruction::Br(0)); // loop
            }
            (false, true) => {
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.expr(t)?); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                v.extend(self.self_sets(e)?); v.push(Instruction::Br(0)); // loop
            }
            (false, false) => {
                // Inside TC loop: must either Br(0) loop or Br(2) exit — never fall through
                v.push(Instruction::If(BlockType::Empty));
                // Check if then-branch is a self-call
                if self.is_self(t) {
                    v.extend(self.self_sets(t)?); v.push(Instruction::Br(0));
                } else {
                    v.extend(self.expr(t)?); v.push(Instruction::Br(2)); // exit block with value
                }
                v.push(Instruction::Else);
                if self.is_self(e) {
                    v.extend(self.self_sets(e)?); v.push(Instruction::Br(0));
                } else {
                    v.extend(self.expr(e)?); v.push(Instruction::Br(2)); // exit block with value
                }
                v.push(Instruction::End);
            }
        }
        Ok(v)
    }

    fn self_sets(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let LispVal::List(items) = e else { return Ok(Vec::new()) };
        let mut v = Vec::new();
        for (i, a) in items[1..].iter().enumerate().take(self.current_param_count) {
            v.extend(self.expr(a)?); v.push(Instruction::LocalSet(i as u32));
        }
        Ok(v)
    }

    // ── Expression ──

    fn expr(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        match e {
            LispVal::Num(n) => Ok(vec![Instruction::I64Const(*n as i64)]),
            LispVal::Bool(true) => Ok(vec![Instruction::I64Const(1)]),
            LispVal::Bool(false) => Ok(vec![Instruction::I64Const(0)]),
            LispVal::Nil => Ok(vec![Instruction::I64Const(0)]),
            LispVal::Sym(n) => self.locals.get(n).map(|&i| vec![Instruction::LocalGet(i)]).ok_or_else(|| self.fmt_undef_error(n)),
            LispVal::Str(s) => {
                let off = self.alloc_data(s.as_bytes()) as u64;
                Ok(vec![Instruction::I64Const((off | ((s.len() as u64) << 32)) as i64)])
            }
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(op) = &items[0] { self.call(op, &items[1..]) } else { Err("expected symbol".into()) }
            }
            _ => Err(format!("unsupported: {:?}", e)),
        }
    }

    fn call(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "+" => self.fold_binop(a, Instruction::I64Add, 0),
            "*" => self.fold_binop(a, Instruction::I64Mul, 1),
            "-" if a.len()==1 => {
                let mut v = vec![Instruction::I64Const(0)];
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Sub);
                Ok(v)
            }
            "-" => self.fold_binop(a, Instruction::I64Sub, i64::MIN as _),
            "/" => self.fold_binop(a, Instruction::I64DivS, i64::MIN as _),
            "mod" => {
                let mut v = self.expr(&a[0])?;
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64RemS);
                Ok(v)
            }
            "abs" => {
                let temp = self.local_idx("__abs_tmp");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalTee(temp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64LtS);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(temp));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(temp));
                v.push(Instruction::End);
                Ok(v)
            }

            ">"  => self.cmp(a, Instruction::I64GtS),
            "<"  => self.cmp(a, Instruction::I64LtS),
            ">=" => self.cmp(a, Instruction::I64GeS),
            "<=" => self.cmp(a, Instruction::I64LeS),
            "="  => self.cmp(a, Instruction::I64Eq),
            "!=" => self.cmp(a, Instruction::I64Ne),

            "and" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::Else); v.push(Instruction::I64Const(0)); v.push(Instruction::End);
                Ok(v)
            }
            "or" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else); v.extend(self.expr(&a[1])?); v.push(Instruction::End);
                Ok(v)
            }
            "not" => { let mut v = self.expr(&a[0])?; v.push(Instruction::I64Eqz); v.push(Instruction::I64ExtendI32U); Ok(v) }

            "if" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::Else);
                if a.len()>2 { v.extend(self.expr(&a[2])?); } else { v.push(Instruction::I64Const(0)); }
                v.push(Instruction::End); Ok(v)
            }
            "begin" => {
                let mut v = Vec::new();
                for (i,x) in a.iter().enumerate() { v.extend(self.expr(x)?); if i<a.len()-1 { v.push(Instruction::Drop); } }
                Ok(v)
            }
            "let" => {
                let mut v = Vec::new();
                if let LispVal::List(bs) = &a[0] {
                    for b in bs { if let LispVal::List(p) = b { if p.len()==2 { if let LispVal::Sym(n) = &p[0] {
                        let idx = self.local_idx(n); v.extend(self.expr(&p[1])?); v.push(Instruction::LocalSet(idx));
                    }}}}
                }
                // Implicit begin: evaluate all body expressions, drop intermediates, keep last
                for (i, x) in a[1..].iter().enumerate() {
                    v.extend(self.expr(x)?);
                    if i < a.len() - 2 { v.push(Instruction::Drop); }
                }
                Ok(v)
            }
            "while" => {
                let id = self.while_id.get(); self.while_id.set(id+1);
                let mut v = Vec::new();
                // block $exit (result i64)
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                // loop $loop
                v.push(Instruction::Loop(BlockType::Empty));
                // cond
                v.extend(self.expr(&a[0])?); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Eqz);
                // if !cond → exit with 0
                v.push(Instruction::If(BlockType::Empty));
                // Push current value of last expression for the block result
                // Actually while returns 0 by spec, just use i64.const 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2)); // br $exit with i64
                v.push(Instruction::End); // if — no else needed
                // body
                for x in &a[1..] { v.extend(self.expr(x)?); v.push(Instruction::Drop); }
                // loop back
                v.push(Instruction::Br(0)); // br $loop
                v.push(Instruction::End); // loop
                // unreachable — loop either exits via br 1 or loops forever
                v.push(Instruction::I64Const(0)); // fallback (unreachable in practice)
                v.push(Instruction::End); // block
                Ok(v)
            }
            "set!" => {
                let LispVal::Sym(n) = &a[0] else { return Err("set!: expected symbol".into()) };
                let idx = self.local_idx(n);
                let mut v = self.expr(&a[1])?;
                v.push(Instruction::LocalSet(idx)); v.push(Instruction::I64Const(0)); Ok(v)
            }

            // Memory
            // ── Higher-order loop macros (expand to while loops) ──

            // (range start end) → returns start as initial counter, used with map/filter/reduce
            // Actually: (for i start end body) — like a for loop
            "for" => {
                // (for var start end body...)
                if a.len() < 4 { return Err("for: need (for var start end body...)".into()); }
                let LispVal::Sym(var) = &a[0] else { return Err("for: var must be symbol".into()) };
                let idx = self.local_idx(var);
                let mut v = Vec::new();
                // init: var = start
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(idx));
                // block (result i64) { loop { if (>= var end) break; body...; var += 1; br loop } }
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // condition: var >= end → exit
                v.push(Instruction::LocalGet(idx));
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                // body expressions (drop all but last)
                for (i, x) in a[3..].iter().enumerate() {
                    v.extend(self.expr(x)?);
                    if i < a.len() - 4 { v.push(Instruction::Drop); }
                }
                v.push(Instruction::Drop); // drop body result
                // increment: var += 1
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0)); // loop
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (reduce init start end accumulator body)
            // accumulator is a symbol, body can reference `it` (current) and accumulator
            // (reduce 0 1 100 acc (+ acc it))
            "reduce" => {
                if a.len() < 5 { return Err("reduce: need (reduce init start end acc_var body)".into()) }
                let LispVal::Sym(acc_var) = &a[3] else { return Err("reduce: acc must be symbol".into()) };
                let acc_idx = self.local_idx(acc_var);
                let it_idx = self.local_idx("__it");
                // acc = init, it = start, while it < end: acc = body, it += 1
                let mut v = Vec::new();
                // acc = init
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(acc_idx));
                // it = start
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(it_idx));
                // while loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // it >= end → exit with acc
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // acc = body
                v.extend(self.expr(&a[4])?);
                v.push(Instruction::LocalSet(acc_idx));
                // it += 1
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (hof/map (lambda (x) body) start end [offset])
            "hof/map" => {
                if a.len() < 3 { return Err("hof/map: need (hof/map (lambda (x) body) start end [offset])".into()); }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] { LispVal::Num(n) => *n as i64, _ => return Err("hof/map: offset must be number".into()) }
                } else { 2048i64 };
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let tmp = self.local_idx("__hof_tmp");
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?); v.push(Instruction::LocalSet(tmp));
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            // (hof/filter (lambda (x) pred) start end [offset])
            "hof/filter" => {
                if a.len() < 3 { return Err("hof/filter: need (hof/filter (lambda (x) pred) start end [offset])".into()); }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] { LispVal::Num(n) => *n as i64, _ => return Err("hof/filter: offset must be number".into()) }
                } else { 2048i64 };
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // addr then value for i64.store
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            // (hof/reduce (lambda (acc x) body) init start end)
            "hof/reduce" => {
                if a.len() < 4 { return Err("hof/reduce: need (hof/reduce (lambda (acc x) body) init start end)".into()); }
                let (params, body) = Self::extract_lambda_2param(&a[0])?;
                let acc_idx = self.local_idx(&params[0]);
                let param_idx = self.local_idx(&params[1]);
                let it_idx = self.local_idx("__hof_it");
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(acc_idx));
                v.extend(self.expr(&a[2])?); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[3])?); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?); v.push(Instruction::LocalSet(acc_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (map-into mem_offset start end body)
            // Writes (body it) into memory at mem_offset + (it-start)*8
            // Returns count
            "map-into" => {
                if a.len() < 4 { return Err("map-into: need (map-into offset start end body)".into()) }
                let it_idx = self.local_idx("__it");
                let off_idx = self.local_idx("__off");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                // off = mem_offset, it = start, count = 0
                v.extend(self.expr(&a[0])?); v.push(Instruction::LocalSet(off_idx));
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // it >= end → exit
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[off] = body(it)
                v.push(Instruction::LocalGet(off_idx));
                v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[3])?);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // off += 8, it += 1, count += 1
                v.push(Instruction::LocalGet(off_idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(off_idx));
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (filter-count start end pred) — count items where pred(it) is truthy
            "filter-count" => {
                if a.len() < 3 { return Err("filter-count: need (filter-count start end pred)".into()) }
                let it_idx = self.local_idx("__it");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // if pred(it): count += 1
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block
                Ok(v)
            }
            "mem-set8!" => {
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[1])?); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "mem-get8" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); Ok(v)
            }
            "mem-set!" => {
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?); v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "mem-get" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // NEAR host calls — capture all sub-expressions first to avoid borrow conflicts
            "near/store" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Store val at mem[0]
                v.push(Instruction::I32Const(0)); v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=0, register_id=0) — idx 17
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/load" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_read(key_len, key_ptr, register_id=0) — idx 18
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                // read_register(0, 0) — idx 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "near/remove" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_remove(key_len, key_ptr, register_id=0) — idx 19
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19));
                Ok(v)
            }
            "near/has_key" => {
                let key = self.expr(&a[0])?;
                let mut v = Vec::new();
                // storage_has_key(key_len, key_ptr) — idx 20
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                Ok(v)
            }
            // --- storage aliases (near/storage_*) using STORAGE_BUF at offset 8192 ---
            "near/storage_set" => {
                let key_expr = self.expr(&a[0])?;
                let val_expr = self.expr(&a[1])?;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Store value at STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.extend(val_expr);
                v.push(Instruction::I64Store(ma));
                // Extract key ptr and len (packed string: low32=ptr, high32=len)
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=STORAGE_BUF, register_id=0)
                v.push(Instruction::I64Const(8));           // val_len
                v.push(Instruction::I64Const(STORAGE_BUF)); // val_ptr
                v.push(Instruction::I64Const(0));           // register_id
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/storage_get" => {
                let key_expr = self.expr(&a[0])?;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Extract key ptr and len
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key_expr);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(0)); // register_id
                // storage_read(key_len, key_ptr, register_id=0) — idx 18
                v.push(Self::host_call(18));
                // Result is 0 (not found) or 1 (found)
                v.push(Instruction::I32WrapI64); // condition as i32
                v.push(Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
                    // Found: read_register(0, STORAGE_BUF) then load i64
                    v.push(Instruction::I64Const(0));           // register_id
                    v.push(Instruction::I64Const(STORAGE_BUF)); // ptr
                    v.push(Self::host_call(0));                 // read_register — idx 0
                    v.push(Instruction::I32Const(STORAGE_BUF as i32));
                    v.push(Instruction::I64Load(ma));
                v.push(Instruction::Else);
                    v.push(Instruction::I64Const(0)); // not found → 0
                v.push(Instruction::End);
                Ok(v)
            }
            "near/storage_has" => {
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key_expr);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                Ok(v)
            }
            "near/storage_remove" => {
                let key_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(key_expr.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key_expr);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(19));
                Ok(v)
            }
            "near/return" => {
                let val = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(0)); v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // value_return(len=8, ptr=0) — idx 25
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            // (near/return_str packed_string) — returns variable-length string bytes
            // packed = low32=ptr, high32=len. Calls value_return(len, ptr) directly.
            "near/return_str" => {
                self.need_host(25);
                let packed = self.expr(&a[0])?;
                let mut v = Vec::new();
                // value_return(len = packed >> 32, ptr = packed & 0xFFFFFFFF)
                v.extend(packed.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // len
                v.extend(packed);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/log" => {
                // (near/log "string") — log string
                // (near/log "prefix" num) — log string then number
                if a.len() == 1 {
                    let msg = self.expr(&a[0])?;
                    let mut v = Vec::new();
                    v.extend(msg.clone());
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.extend(msg);
                    v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(0)); Ok(v)
                } else {
                    // String + number: copy string to LOG_BUF, convert number to ASCII,
                    // append digits after string, single log_utf8 call
                    let packed = self.expr(&a[0])?;  // i64 = ptr | (len << 32)
                    let num_expr = self.expr(&a[1])?;
                    let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                    let str_len = self.local_idx("__clog_strlen");
                    let copy_i = self.local_idx("__clog_i");
                    let abs_val = self.local_idx("__logn_abs");
                    let digit_count = self.local_idx("__logn_digits");
                    let is_neg = self.local_idx("__logn_neg");
                    let tmp_digit = self.local_idx("__logn_d");
                    let num_ptr = self.local_idx("__logn_ptr");
                    let copy2_i = self.local_idx("__clog_i2");
                    let packed_local = self.local_idx("__clog_packed");
                    let mut v = Vec::new();

                    // Store packed string value in local
                    v.extend(packed.clone());
                    v.push(Instruction::LocalSet(packed_local));

                    // Extract str_len = packed >> 32
                    v.push(Instruction::LocalGet(packed_local));
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(str_len));

                    // Copy string bytes: for i in 0..str_len: mem[4096+i] = mem[str_ptr+i]
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(copy_i));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    // condition: copy_i >= str_len
                    v.push(Instruction::LocalGet(copy_i));
                    v.push(Instruction::LocalGet(str_len));
                    v.push(Instruction::I64GeS);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    // dest addr: (4096 + copy_i) as i32 — DEEPER
                    v.push(Instruction::I64Const(4096));
                    v.push(Instruction::LocalGet(copy_i));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // src byte: load from (str_ptr + copy_i)
                    v.push(Instruction::LocalGet(packed_local));
                    v.push(Instruction::I32WrapI64);         // str_ptr as i32
                    v.push(Instruction::LocalGet(copy_i));
                    v.push(Instruction::I32WrapI64);         // copy_i as i32
                    v.push(Instruction::I32Add);             // src addr
                    v.push(Instruction::I32Load8U(ma8));     // load byte (value on top)
                    v.push(Instruction::I32Store8(ma8));     // store: pops value, then addr
                    // i++
                    v.push(Instruction::LocalGet(copy_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(copy_i));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);

                    // Now convert number to ASCII in NUM_BUF (4160..4184)
                    v.extend(num_expr);
                    v.push(Instruction::LocalSet(abs_val));
                    // is_neg = abs_val < 0
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64LtS);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(is_neg));
                    // abs_val = if is_neg { -abs_val } else { abs_val }
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
                    // num_ptr = 4184 (write backwards)
                    v.push(Instruction::I64Const(4184));
                    v.push(Instruction::LocalSet(num_ptr));
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(digit_count));
                    // digit conversion loop
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    // tmp_digit = abs_val % 10
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64RemS);
                    v.push(Instruction::LocalSet(tmp_digit));
                    // abs_val /= 10
                    v.push(Instruction::LocalGet(abs_val));
                    v.push(Instruction::I64Const(10));
                    v.push(Instruction::I64DivS);
                    v.push(Instruction::LocalSet(abs_val));
                    // ptr--, store digit
                    v.push(Instruction::LocalGet(num_ptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(num_ptr));
                    v.push(Instruction::LocalGet(num_ptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(tmp_digit));
                    v.push(Instruction::I64Const(48));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store8(ma8));
                    // digit_count++
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);

                    // Handle zero case
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(4183));
                    v.push(Instruction::LocalSet(num_ptr));
                    v.push(Instruction::I64Const(4183));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(48));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store8(ma8));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);

                    // Handle negative: prepend '-'
                    v.push(Instruction::LocalGet(is_neg));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(num_ptr));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(num_ptr));
                    v.push(Instruction::LocalGet(num_ptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(45));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Store8(ma8));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(digit_count));
                    v.push(Instruction::End);

                    // Copy number digits from NUM_BUF to after string in LOG_BUF
                    // for i in 0..digit_count: mem[4096+str_len+i] = mem[num_ptr+i]
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(copy2_i));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(copy2_i));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64GeS);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::Br(2));
                    v.push(Instruction::End);
                    // dest addr: (4096 + str_len + copy2_i) as i32 — DEEPER
                    v.push(Instruction::I64Const(4096));
                    v.push(Instruction::LocalGet(str_len));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(copy2_i));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // src byte: load from (num_ptr + copy2_i)
                    v.push(Instruction::LocalGet(num_ptr));
                    v.push(Instruction::LocalGet(copy2_i));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I32Load8U(ma8));
                    v.push(Instruction::I32Store8(ma8));
                    // i++
                    v.push(Instruction::LocalGet(copy2_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(copy2_i));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);

                    // Single log_utf8(str_len + digit_count, 4096)
                    v.push(Instruction::LocalGet(str_len));
                    v.push(Instruction::LocalGet(digit_count));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Const(4096));
                    v.push(Self::host_call(28));
                    v.push(Instruction::I64Const(0));
                    Ok(v)
                }
            }
            "near/panic" => {
                let msg = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(msg.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(msg);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // panic_utf8(len, ptr) — idx 27
                v.push(Self::host_call(27));
                v.push(Instruction::I64Const(0)); Ok(v)
            }
            "near/abort" => {
                // panic() — idx 26, traps unconditionally
                Ok(vec![Self::host_call(26), Instruction::I64Const(0)])
            }
            // (near/log_num expr) — converts i64 to decimal string and logs via env.log_utf8
            "near/log_num" => {
                self.need_host(28);
                let num_expr = self.expr(&a[0])?;
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let abs_val = self.local_idx("__logn_abs");
                let digit_count = self.local_idx("__logn_digits");
                let is_neg = self.local_idx("__logn_neg");
                let tmp_digit = self.local_idx("__logn_d");
                let ptr = self.local_idx("__logn_ptr");
                let mut v = Vec::new();
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
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp_digit));
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma8));
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Zero special case
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(4183));
                v.push(Instruction::LocalSet(ptr));
                v.push(Instruction::I64Const(4183));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma8));
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
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(45));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma8));
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(digit_count));
                v.push(Instruction::End);
                // log_utf8(count, ptr)
                v.push(Instruction::LocalGet(digit_count));
                v.push(Instruction::LocalGet(ptr));
                v.push(Self::host_call(28));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "near/current_account_id" => self.read_to_register(3, a),
            "near/signer_account_id" => self.read_to_register(4, a),
            "near/predecessor_account_id" => self.read_to_register(6, a),
            "near/input" => self.read_to_register(7, a),
            "near/block_index" => Ok(vec![Self::host_call(8)]),
            "near/block_timestamp" => Ok(vec![Self::host_call(9)]),
            "near/epoch_height" => Ok(vec![Self::host_call(10)]),
            "near/prepaid_gas" => Ok(vec![Self::host_call(15)]),
            "near/used_gas" => Ok(vec![Self::host_call(16)]),
            "near/attached_deposit" => self.read_u128_low(14),
            "near/attached_deposit_high" => self.read_u128_high(14),
            "near/account_balance" => self.read_u128_low(12),
            "near/sha256" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // sha256(data_len, data_ptr, register_id=0) — idx 21
                v.extend(data.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(21));
                // read_register(0, TEMP_MEM) — idx 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0) — idx 1
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                Ok(v)
            }
            "near/random_seed" => self.read_to_register(23, a),

            // ── Promises / Cross-contract calls ──

            // (near/promise_create account_id method args amount gas) → promise_index: i64
            // All args are packed strings except amount (i64) and gas (i64)
            "near/promise_create" => {
                // promise_create(account_id_len, account_id_ptr, method_name_len, method_name_ptr,
                //                arguments_len, arguments_ptr, amount_ptr, gas) → i64  (idx 30)
                let account = self.expr(&a[0])?;
                let method = self.expr(&a[1])?;
                let args = self.expr(&a[2])?;
                let amount = self.expr(&a[3])?;
                let gas = self.expr(&a[4])?;
                let mut v = Vec::new();
                // account_id: len >> 32, ptr & 0xFFFF_FFFF
                v.extend(account.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // method_name: len >> 32, ptr
                v.extend(method.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // arguments: len >> 32, ptr
                v.extend(args.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // amount: store at mem[0], pass ptr=0
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); // amount_ptr
                v.extend(gas);
                v.push(Self::host_call(30)); // returns promise_index
                Ok(v)
            }

            // (near/promise_then promise_idx account_id method args amount gas) → new_promise_idx
            "near/promise_then" => {
                let pidx = self.expr(&a[0])?;
                let account = self.expr(&a[1])?;
                let method = self.expr(&a[2])?;
                let args = self.expr(&a[3])?;
                let amount = self.expr(&a[4])?;
                let gas = self.expr(&a[5])?;
                let mut v = Vec::new();
                v.extend(pidx);
                v.extend(account.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(account); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(method.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(method); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(args.clone()); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(args); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I32Const(0)); v.extend(amount);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0));
                v.extend(gas);
                v.push(Self::host_call(31));
                Ok(v)
            }

            // (near/promise_and promise_idx1 promise_idx2 ...) → combined_promise_idx
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

            // (near/promise_results_count) → count: i64
            "near/promise_results_count" => {
                Ok(vec![Self::host_call(33)])
            }

            // (near/promise_result idx) → packed result string
            // promise_result(result_idx, register_id=0) → void, then read_register
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

            // (near/promise_return promise_idx) — return promise result to caller
            "near/promise_return" => {
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(35));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // ── Promise batch actions (host funcs 39-49) ──

            // (near/promise_batch_create account_ptr account_len) → promise_id
            "near/promise_batch_create" => {
                let ptr = self.expr(&a[0])?;
                let len = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(len); v.extend(ptr);
                v.push(Self::host_call(39));
                Ok(v)
            }

            // (near/promise_batch_then promise_idx account_ptr account_len) → promise_id
            "near/promise_batch_then" => {
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(len); v.extend(ptr);
                v.push(Self::host_call(40));
                Ok(v)
            }

            // (near/promise_batch_action_create_account promise_idx)
            "near/promise_batch_action_create_account" => {
                let idx = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(idx);
                v.push(Self::host_call(41));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_deploy_contract promise_idx code_ptr code_len)
            "near/promise_batch_action_deploy_contract" => {
                let idx = self.expr(&a[0])?;
                let code_ptr = self.expr(&a[1])?;
                let code_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(code_len); v.extend(code_ptr);
                v.push(Self::host_call(42));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_function_call promise_idx method_ptr method_len args_ptr args_len amount_ptr gas)
            "near/promise_batch_action_function_call" => {
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

            // (near/promise_batch_action_transfer promise_idx amount_ptr amount_len)
            "near/promise_batch_action_transfer" => {
                let idx = self.expr(&a[0])?;
                let amount_ptr = self.expr(&a[1])?;
                let amount_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(amount_ptr); v.extend(amount_len);
                v.push(Self::host_call(44));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_stake promise_idx amount_ptr amount_len pk_ptr pk_len)
            "near/promise_batch_action_stake" => {
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

            // (near/promise_batch_action_add_key_with_full_access promise_idx pk_ptr pk_len nonce)
            "near/promise_batch_action_add_key_with_full_access" => {
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

            // (near/promise_batch_action_add_key_with_function_call promise_idx pk_ptr pk_len nonce method_ptr method_len allowance)
            "near/promise_batch_action_add_key_with_function_call" => {
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

            // (near/promise_batch_action_delete_key promise_idx pk_ptr pk_len)
            "near/promise_batch_action_delete_key" => {
                let idx = self.expr(&a[0])?;
                let pk_ptr = self.expr(&a[1])?;
                let pk_len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(pk_ptr); v.extend(pk_len);
                v.push(Self::host_call(48));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (near/promise_batch_action_delete_account promise_idx beneficiary_ptr beneficiary_len)
            "near/promise_batch_action_delete_account" => {
                let idx = self.expr(&a[0])?;
                let ptr = self.expr(&a[1])?;
                let len = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(idx); v.extend(ptr); v.extend(len);
                v.push(Self::host_call(49));
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // ── Iterator support ──

            // (near/iter_prefix prefix_ptr prefix_len) → iterator_id: i64
            // storage_iter_prefix writes prefix to register, then calls host(36)
            "near/iter_prefix" => {
                let prefix = self.expr(&a[0])?;
                let prefix_len = self.expr(&a[1])?;
                let mut v = Vec::new();
                // write_register(register_id=0, prefix_ptr, prefix_len)
                // Store prefix data at mem[0] first — prefix is a packed string or raw ptr+len
                // For packed string input: extract ptr and len
                // prefix is packed (low32=ptr, high32=len), prefix_len is explicit
                // Actually: prefix_ptr and prefix_len are separate args
                // Write prefix data to register: write_register(register_id=0, len=prefix_len, ptr=prefix_ptr)
                // write_register(idx 2): (register_id, data_len, data_ptr)
                v.push(Instruction::I64Const(0)); // register_id = 0
                v.extend(prefix_len.clone());
                v.extend(prefix);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // ptr as i64
                // Swap to get (register_id, data_ptr, data_len) — nope, write_register is (register_id, data_len, data_ptr)
                // Actually HOST_FUNCS[2] = write_register: (I64, I64, I64) = (register_id, data_len, data_ptr)
                // We pushed: reg_id=0, prefix_len, prefix_ptr. That's correct order.
                v.push(Self::host_call(2)); // write_register — returns void, no drop
                // storage_iter_prefix(prefix_len, register_id=0) — idx 36
                // But wait: HOST_FUNCS[36] = storage_iter_prefix: (I64, I64) = (prefix_len, register_id)
                // We need to pass the length again and register_id
                v.extend(prefix_len.clone());
                v.push(Instruction::I64Const(0)); // register_id = 0
                v.push(Self::host_call(36));
                Ok(v)
            }

            // (near/iter_range start_ptr start_len end_ptr end_len) → iterator_id: i64
            "near/iter_range" => {
                let start = self.expr(&a[0])?;
                let start_len = self.expr(&a[1])?;
                let end = self.expr(&a[2])?;
                let end_len = self.expr(&a[3])?;
                let mut v = Vec::new();
                // Write start to register 0
                v.push(Instruction::I64Const(0)); // register_id
                v.extend(start_len.clone());
                v.extend(start); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2)); // write_register — void
                // Write end to register 1
                v.push(Instruction::I64Const(1)); // register_id
                v.extend(end_len.clone());
                v.extend(end); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(2)); // write_register — void
                // storage_iter_range(start_len, register_id=0, end_len, register_id=1) — idx 37
                v.extend(start_len);
                v.push(Instruction::I64Const(0));
                v.extend(end_len);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(37));
                Ok(v)
            }

            // (near/iter_next iter_id key_ptr val_ptr) → i64 (1 if found, 0 if done)
            "near/iter_next" => {
                let iter_id = self.expr(&a[0])?;
                let key_ptr = self.expr(&a[1])?;
                let val_ptr = self.expr(&a[2])?;
                let mut v = Vec::new();
                // storage_iter_next(iter_id, key_register_id, value_register_id) — idx 38
                v.extend(iter_id);
                v.extend(key_ptr);
                v.extend(val_ptr);
                v.push(Self::host_call(38));
                Ok(v)
            }

            // ── u128 Arithmetic (two i64s: low at addr, high at addr+8) ──
            // Scratch area: 128-191 (4 i64 slots at offsets 128,136,144,152)

            // (u128/store addr low high) — store u128 at addr
            "u128/store" => {
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                // store low at addr
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // store high at addr+8
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (u128/load addr) → low 64 bits
            "u128/load" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (u128/load_high addr) → high 64 bits
            "u128/load_high" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (u128/add dst_addr src_addr) — dst += src
            // Uses scratch at 128: dst_lo_result, 136: dst_hi, 144: carry
            "u128/add" => {
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128a");
                let src_i = self.local_idx("__u128b");
                let lo_i = self.local_idx("__u128lo");
                let hi_i = self.local_idx("__u128hi");
                let c_i = self.local_idx("__u128c");
                let mut v = Vec::new();
                // Save addresses
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load dst_low, src_low
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // lo_i = dst_low + src_low
                v.push(Instruction::LocalGet(lo_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(lo_i));
                // Carry: if result < src_low (unsigned), carry=1
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // hi = dst_high + src_high + carry
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(hi_i));
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (u128/sub dst_addr src_addr) — dst -= src
            "u128/sub" => {
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128sa");
                let src_i = self.local_idx("__u128sb");
                let lo_i = self.local_idx("__u128slo");
                let hi_i = self.local_idx("__u128shi");
                let b_i = self.local_idx("__u128borrow");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load dst_low into lo_i
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                // Load src_low into b_i
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(b_i));
                // borrow = lo_i < b_i (unsigned)
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(b_i));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // lo_i = lo_i - src_low
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(lo_i));
                // hi = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(hi_i));
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (u128/mul dst_addr val_i64) — dst *= val_i64 (unsigned)
            // Simplified: dst_lo * val, dst_hi = dst_hi * val + (dst_lo * val) >> 64
            // Uses scratch 128-159: stores intermediate (low_result at 128, high_result at 136)
            // We use i64.mul for low part and handle overflow via comparison
            "u128/mul" => {
                let dst = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128ma");
                let val_i = self.local_idx("__u128mv");
                let dl_i = self.local_idx("__u128mdl");
                let dh_i = self.local_idx("__u128mdh");
                let rl_i = self.local_idx("__u128mrl");
                let rh_i = self.local_idx("__u128mrh");
                let t_i = self.local_idx("__u128mt");
                let carry_i = self.local_idx("__u128mc");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // Load dst_lo, dst_hi
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dl_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dh_i));
                // rl = dl * val (i64.mul, wraps on overflow)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));
                // carry from low mul: if rl < dl (assuming val >= 2, but edge cases...)
                // Better: split dl into high32 and low32, multiply separately
                // Simpler approach: carry = (dl * val) >> 64 ≈ (dl >> 32) * val + ...
                // Approximation using (dl >> 32) * (val) + ((dl & 0xFFFFFFFF) * (val >> 32))
                // This gives the high 64 bits of the 128-bit product of low halves
                // carry = (dl >> 32) * val (shifted left 0, but this is 96-bit...)
                // Actually: carry = ((dl >> 32) * val) + (((dl & 0xFFFFFFFF) * val) >> 32)
                // But we need >>64 not >>32. Let's do:
                // carry = (dl >> 32) * (val >> 32) is wrong too.
                // Correct approach for full carry:
                // carry = dl_hi * val_lo + dl_lo * val_hi + (dl_lo * val_lo >> 64)
                // But we can't easily get >> 64 of a 64x64->128 mul in WASM i64.
                //
                // PRAGMATIC: For DeFi amounts, values are typically < 2^53 (exact i64).
                // We use: carry = (dl != 0 && val != 0 && rl < dl) as rough carry estimate
                // This is WRONG for large values. Let me use the split approach properly.
                //
                // Split: dl = (dl_hi << 32) | dl_lo where dl_hi = dl >> 32, dl_lo = dl & 0xFFFF_FFFF
                // full_lo = dl_lo * val_lo  (fits in 64 bits since both < 2^32)
                // mid1 = dl_hi * val_lo
                // mid2 = dl_lo * val_hi
                // rl = full_lo + ((mid1 + mid2) << 32)   — but this can overflow too
                //
                // SIMPLEST CORRECT: Use the comparison trick.
                // If dl != 0 and val != 0 and rl / dl != val, there was overflow.
                // But division is expensive and can trap.
                //
                // Let me just do: carry = 0 for now, and document that mul is correct
                // only when the product of the low halves fits in 64 bits.
                // For NEAR FT amounts (u128 low part usually < 2^60), multiplying by
                // prices < 2^20, this is fine.
                //
                // Actually the simplest correct approach for full 64x64->128:
                // We can't do it with just i64 ops without splitting into 32-bit halves.
                // Let's do the 32-bit split:

                // dl_hi = dl >> 32, dl_lo = dl & 0xFFFFFFFF
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(t_i)); // t = dl_hi

                // carry = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(carry_i));

                // rl = dl_lo * val_lo (both < 2^32, product < 2^64)
                // rl = (dl & 0xFFFF_FFFF) * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));

                // carry += (dl_lo * val_lo) >> 32
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += dl_hi * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(t_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += (dl & 0xFFFF_FFFF) * (val >> 32)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // rl &= 0xFFFF_FFFF (keep only low 32 bits)
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(rl_i));

                // Now carry has bits [32..95] of the 128-bit low product
                // rh = dh * val + carry + (dl_hi * (val >> 32) shifted)
                // Actually carry already accumulated everything above bit 32.
                // rh = dh * val + carry
                v.push(Instruction::LocalGet(dh_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rh_i));

                // Store results
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (u128/lt addr1 addr2) → i64 (0 or 1)
            "u128/lt" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128lt1");
                let a2_i = self.local_idx("__u128lt2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // Compare high first: if a1_hi < a2_hi → 1; if a1_hi > a2_hi → 0; else compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // a1_hi < a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // Check a1_hi > a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64GtU); // a1_hi > a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // Highs equal, compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End);
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/eq addr1 addr2) → i64 (0 or 1)
            "u128/eq" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128eq1");
                let a2_i = self.local_idx("__u128eq2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // high_eq = a1_hi == a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                // I64Eq returns i32, which If consumes directly
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // low_eq = a1_lo == a2_lo
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/is_zero addr) → i64
            "u128/is_zero" => {
                let mut v = self.expr(&a[0])?;
                let addr_i = self.local_idx("__u128zz");
                v.push(Instruction::LocalSet(addr_i));
                // low == 0 && high == 0
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (u128/from_yocto "amount" offset) — compile-time parse decimal, store at offset, return offset
            "u128/from_yocto" => {
                if a.len() != 2 { return Err("u128/from_yocto: expected (\"amount\" offset)".into()); }
                let offset_expr = self.expr(&a[1])?;
                let (lo, hi) = match &a[0] {
                    LispVal::Str(s) => Self::parse_u128(s)?,
                    _ => return Err("u128/from_yocto: first arg must be a string literal".into()),
                };
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(lo));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            // (u128/new hi lo offset) — store hi:lo at offset, return offset
            "u128/new" => {
                if a.len() != 3 { return Err("u128/new: expected (hi lo offset)".into()); }
                let hi_e = self.expr(&a[0])?;
                let lo_e = self.expr(&a[1])?;
                let off_e = self.expr(&a[2])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(lo_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(hi_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            // (u128/from_i64 n offset) — zero-extend i64 to u128 at offset, return offset
            "u128/from_i64" => {
                if a.len() != 2 { return Err("u128/from_i64: expected (n offset)".into()); }
                let n_e = self.expr(&a[0])?;
                let off_e = self.expr(&a[1])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(n_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }

            // (u128/to_i64 offset) — load low 64 bits
            "u128/to_i64" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (u128/store_storage "key" src) — store u128 at src to NEAR storage under key
            "u128/store_storage" => {
                if a.len() != 2 { return Err("u128/store_storage: expected (\"key\" src)".into()); }
                let key = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let os = self.local_idx("__u128_s");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(src); v.push(Instruction::LocalSet(os));
                // Copy 16 bytes from src to STORAGE_U128_BUF
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Store(ma));
                // storage_write(key_len, key_ptr, 16, STORAGE_U128_BUF, 0) — idx 17
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Const(STORAGE_U128_BUF)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0));
                Ok(v)
            }

            // (u128/load_storage "key" dst) — load u128 from NEAR storage to dst, return dst
            "u128/load_storage" => {
                if a.len() != 2 { return Err("u128/load_storage: expected (\"key\" dst)".into()); }
                let key = self.expr(&a[0])?;
                let dst = self.expr(&a[1])?;
                let od = self.local_idx("__u128_d");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(od));
                // storage_read(key_len, key_ptr, 0) — idx 18
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                // read_register(0, STORAGE_U128_BUF)
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(STORAGE_U128_BUF));
                v.push(Self::host_call(0)); v.push(Instruction::Drop);
                // Copy 16 bytes from STORAGE_U128_BUF to dst
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(od));
                Ok(v)
            }

            // ── CLMM / Uniswap V3 Primitives ──
            // All use 64.64 fixed-point (Q64.64): value = raw >> 64, raw = value << 64
            // Price stored as sqrtPriceX96 equivalent: Q64.96

            // (fp/mul a b) → i64 — Q32.32 multiply: (a * b) >> 32
            // 64x32 multiply: split a into hi/lo 16-bit parts vs 32-bit b
            // a*b = (a_hi * b)<<16 | a_lo*b, then >> 32
            "fp/mul" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpm_a");
                let b_i = self.local_idx("__fpm_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                // result = (a >> 16) * (b >> 16) + ((a & 0xFFFF) * b) >> 32
                // For Q32.32: just use (a * b) >> 32
                // a*b won't overflow if a < 2^48 and b < 2^16... but they can be larger
                // Safe method: (a >> 16) * b doesn't overflow if a < 2^48 and b < 2^16
                // For our use: a and b are Q32.32, max ~2^32 each, so a>>16 is ~2^16, *b ~2^48 fine
                // But for larger values, need full split:
                // result = ((a >> 16) * (b >> 16)) + (((a >> 16) * (b & 0xFFFF)) >> 16) + (((a & 0xFFFF) * (b >> 16)) >> 16)
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // a_hi * b_hi
                // + (a_hi * b_lo) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // + (a_lo * b_hi) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // a_lo * b_lo is negligible after >> 32 for Q32.32 precision
                Ok(v)
            }

            // (fp/div a b) → i64 — Q32.32 divide: (a << 32) / b
            "fp/div" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpd_a");
                let b_i = self.local_idx("__fpd_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                Ok(v)
            }

            // (fp/to_int x) → i64 — Q32.32 → integer: x >> 32
            "fp/to_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // (fp/from_int x) → i64 — integer → Q32.32: x << 32
            "fp/from_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                Ok(v)
            }

            // (fp/one) → i64 — 1.0 in Q32.32 = 1 << 32
            "fp/one" => {
                Ok(vec![Instruction::I64Const(1), Instruction::I64Const(32), Instruction::I64Shl])
            }

            // ── Q64.64 fixed-point (NEAR standard, dual-i64 in memory) ──
            // Layout: mem[addr] = low 64 bits, mem[addr+8] = high 64 bits
            // Value = (high << 64 | low) / 2^64 = high + low/2^64

            // (fp64/set_int addr val) — store integer as Q64.64
            "fp64/set_int" => {
                let addr = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // mem[addr] = 0 (low)
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // mem[addr+8] = val (high)
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (fp64/get_int addr) → i64 — integer part = mem[addr+8]
            "fp64/get_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (fp64/get_frac addr) → i64 — fractional part = mem[addr]
            "fp64/get_frac" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (fp64/set addr lo hi) — store raw Q64.64 parts
            "fp64/set" => {
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (fp64/add dst_addr src_addr) — dst += src (both Q64.64 in memory)
            "fp64/add" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64_dl");
                let dh = self.local_idx("__fp64_dh");
                let sl = self.local_idx("__fp64_sl");
                let sh = self.local_idx("__fp64_sh");
                let carry = self.local_idx("__fp64_c");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // dst_low += src_low, detect carry
                v.push(Instruction::LocalGet(sl)); v.push(Instruction::LocalGet(dl)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dl));
                // carry = 1 if dl < sl (overflow)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high += src_high + carry
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::LocalGet(dh)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (fp64/mul dst_addr src_addr) — dst *= src (Q64.64, full 128-bit multiply via 32-bit splits)
            // result = (a * b) >> 64, where a={dl,dh}, b={sl,sh}
            // Uses 32-bit splits for each 64x64 multiply to get full 128-bit precision
            "fp64/mul" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fm_dl");
                let dh = self.local_idx("__fm_dh");
                let sl = self.local_idx("__fm_sl");
                let sh = self.local_idx("__fm_sh");
                // temps for 32-bit split multiply: mulh(x,y) → hi64(x*y)
                let x_lo = self.local_idx("__fm_xlo");
                let x_hi = self.local_idx("__fm_xhi");
                let y_lo = self.local_idx("__fm_ylo");
                let y_hi = self.local_idx("__fm_yhi");
                let ll = self.local_idx("__fm_ll");
                let lh = self.local_idx("__fm_lh");
                let hl = self.local_idx("__fm_hl");
                let hh = self.local_idx("__fm_hh");
                let mid = self.local_idx("__fm_mid");
                let mc = self.local_idx("__fm_mc");
                let lo = self.local_idx("__fm_lo");
                let lc = self.local_idx("__fm_lc");
                let hi = self.local_idx("__fm_hi");
                // Cross-term storage
                let cross1_lo = self.local_idx("__fm_c1l");
                let cross1_hi = self.local_idx("__fm_c1h");
                let cross2_lo = self.local_idx("__fm_c2l");
                let cross2_hi = self.local_idx("__fm_c2h");
                let albl_hi = self.local_idx("__fm_abh");
                let rl = self.local_idx("__fm_rl");
                let rh = self.local_idx("__fm_rh");
                let tmp = self.local_idx("__fm_tmp");
                let tmp2 = self.local_idx("__fm_tmp2");
                let mut v = Vec::new();

                // Helper macro-like: emit code to compute hi=high64(x*y), lo=low64(x*y)
                // Stack should have x, y when called. Uses x_lo,x_hi,y_lo,y_hi,ll,lh,hl,hh,mid,mc,lo,lc,hi
                // After: hi and lo locals are set. Nothing on stack.
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi: u32, lo: u32| {
                    // x_lo = x & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(x_lo));
                    // x_hi = x >> 32
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(x_hi));
                    // y_lo = y & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(y_lo));
                    // y_hi = y >> 32
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(y_hi));
                    // ll = x_lo * y_lo
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(ll));
                    // lh = x_lo * y_hi
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(lh));
                    // hl = x_hi * y_lo
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hl));
                    // hh = x_hi * y_hi
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hh));
                    // mid = lh + hl, mid_carry = mid < lh
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::LocalGet(hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(mid));
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(mc));
                    // lo = ll + (mid << 32), lo_carry = lo < ll
                    v.push(Instruction::LocalGet(ll));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(lo));
                    v.push(Instruction::LocalGet(ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(lc));
                    // hi = hh + (mid >> 32) + (mc << 32) + lc
                    v.push(Instruction::LocalGet(hh));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi));
                    // lo result
                };

                // Load dst {dl, dh}
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // Load src {sl, sh}
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));

                // Step 1: Compute high64(dl*sl) → albl_hi (we only need high part)
                emit_mul128(&mut v, dl, sl, albl_hi, tmp);

                // Step 2: Compute full 128-bit ah*bl → {cross1_lo, cross1_hi}
                emit_mul128(&mut v, dh, sl, cross1_hi, cross1_lo);

                // Step 3: Compute full 128-bit al*bh → {cross2_lo, cross2_hi}
                emit_mul128(&mut v, dl, sh, cross2_hi, cross2_lo);

                // Step 4: cross = cross1 + cross2 (128-bit add)
                // cross_lo = cross1_lo + cross2_lo, carry_a
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::LocalGet(cross2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2)); // tmp2 = carry_a
                // tmp = cross_lo
                // cross_hi = cross1_hi + cross2_hi + carry_a
                v.push(Instruction::LocalGet(cross1_hi)); v.push(Instruction::LocalGet(cross2_hi)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(mid));
                // mid = cross_hi, tmp = cross_lo

                // Step 5: result_lo = cross_lo + albl_hi (may carry)
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(albl_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(rl));
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp)); // tmp = carry_b

                // Step 6: result_hi = dh*sh + cross_hi + carry_b
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rh));

                // Store result to dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (fp64/lt addr1 addr2) → i64 — compare Q64.64 values
            "fp64/lt" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let h1 = self.local_idx("__fplt_h1");
                let h2 = self.local_idx("__fplt_h2");
                let mut v = Vec::new();
                // Compare high parts first
                v.extend(a1.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h1));
                v.extend(a2.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h2));
                // if h1 < h2: return 1
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // if h1 > h2: return 0
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // High equal, compare low
                v.extend(a1); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(a2); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // returns i32
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End); // inner if
                v.push(Instruction::End); // outer if
                Ok(v)
            }

            // (fp64/is_zero addr) → i64
            "fp64/is_zero" => {
                let addr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // high == 0?
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (fp64/sub dst_addr src_addr) — dst -= src (Q64.64, subtract with borrow)
            "fp64/sub" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64s_dl");
                let dh = self.local_idx("__fp64s_dh");
                let sl = self.local_idx("__fp64s_sl");
                let sh = self.local_idx("__fp64s_sh");
                let borrow = self.local_idx("__fp64s_b");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // borrow = dl < sl (unsigned)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(borrow));
                // dst_low -= src_low
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dl));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(borrow)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (fp64/div dst_addr src_addr) — dst /= src (Q64.64, Newton reciprocal + full-precision mul)
            // a/b = a * (1/b), compute reciprocal via Newton, then multiply
            "fp64/div" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fpd_d");
                let src_i = self.local_idx("__fpd_s");
                let ah = self.local_idx("__fpd_ah");
                let al = self.local_idx("__fpd_al");
                let bh = self.local_idx("__fpd_bh");
                let bl = self.local_idx("__fpd_bl");
                // Newton state: x_lo, x_hi (reciprocal estimate)
                let x_lo = self.local_idx("__fpd_xl");
                let x_hi = self.local_idx("__fpd_xh");
                // Temp for b*x
                let tx_lo = self.local_idx("__fpd_txl");
                let tx_hi = self.local_idx("__fpd_txh");
                // Temp for correction = 2.0 - b*x
                let cl = self.local_idx("__fpd_cl");
                let ch = self.local_idx("__fpd_ch");
                // mul128 temps (shared with mul)
                let m_xlo = self.local_idx("__fm_xlo");
                let m_xhi = self.local_idx("__fm_xhi");
                let m_ylo = self.local_idx("__fm_ylo");
                let m_yhi = self.local_idx("__fm_yhi");
                let m_ll = self.local_idx("__fm_ll");
                let m_lh = self.local_idx("__fm_lh");
                let m_hl = self.local_idx("__fm_hl");
                let m_hh = self.local_idx("__fm_hh");
                let m_mid = self.local_idx("__fm_mid");
                let m_mc = self.local_idx("__fm_mc");
                let m_lo = self.local_idx("__fm_lo");
                let m_lc = self.local_idx("__fm_lc");
                let m_hi = self.local_idx("__fm_hi");
                // Cross-term temps for mul
                let c1_lo = self.local_idx("__fpd_c1l");
                let c1_hi = self.local_idx("__fpd_c1h");
                let c2_lo = self.local_idx("__fpd_c2l");
                let c2_hi = self.local_idx("__fpd_c2h");
                let ab_hi = self.local_idx("__fpd_abh");
                let rl = self.local_idx("__fpd_rl");
                let rh = self.local_idx("__fpd_rh");
                let tmp = self.local_idx("__fpd_tmp");
                let tmp2 = self.local_idx("__fpd_tmp2");
                let mut v = Vec::new();

                // emit_mul128: computes hi=high64(x*y), lo=low64(x*y)
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi_dst: u32, lo_dst: u32| {
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_xlo));
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_xhi));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_ylo));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_yhi));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_ll));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_lh));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hl));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hh));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::LocalGet(m_hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_mid));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_mc));
                    v.push(Instruction::LocalGet(m_ll));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_lo));
                    v.push(Instruction::LocalGet(m_ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_lc));
                    v.push(Instruction::LocalGet(m_hh));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi_dst));
                    v.push(Instruction::LocalGet(m_lo)); v.push(Instruction::LocalSet(lo_dst));
                };

                // emit_fp64_mul: full Q64.64 multiply of {a_lo,a_hi} * {b_lo,b_hi} → {dst_lo,dst_hi}
                let emit_fp64_mul = |v: &mut Vec<Instruction<'static>>, a_lo: u32, a_hi: u32, b_lo: u32, b_hi: u32, dst_lo: u32, dst_hi: u32| {
                    // high64(a_lo * b_lo) → ab_hi (don't need low)
                    emit_mul128(v, a_lo, b_lo, ab_hi, tmp);
                    // full 128: a_hi * b_lo → {c1_lo, c1_hi}
                    emit_mul128(v, a_hi, b_lo, c1_hi, c1_lo);
                    // full 128: a_lo * b_hi → {c2_lo, c2_hi}
                    emit_mul128(v, a_lo, b_hi, c2_hi, c2_lo);
                    // cross = c1 + c2 (128-bit add)
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::LocalGet(c2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2));
                    v.push(Instruction::LocalGet(c1_hi)); v.push(Instruction::LocalGet(c2_hi)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(m_mid));
                    // result_lo = cross_lo + ab_hi
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(ab_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(dst_lo));
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // result_hi = a_hi*b_hi + cross_hi + carry
                    v.push(Instruction::LocalGet(a_hi)); v.push(Instruction::LocalGet(b_hi)); v.push(Instruction::I64Mul);
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_hi));
                };

                v.extend(da); v.push(Instruction::LocalSet(dst_i));
                v.extend(sa); v.push(Instruction::LocalSet(src_i));
                // Load a = dst (numerator)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(al));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ah));
                // Load b = src (denominator)
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bh));

                // Initial reciprocal estimate: x0 ≈ 1/b in Q64.64
                // For Q64.64 value b = bh + bl/2^64, 1/b ≈ 2^64/bh (for bh > 0)
                // As Q64.64: 1/b ≈ {2^64/bh, 0} if 1/b < 1, or {0, 2^64/bh} if 1/b >= 1
                // Since 2^64 doesn't fit in i64, use (2^64-1)/bh as approximation
                // If bh == 1: x0 = {0, 1} (exact reciprocal ≈ 1.0)
                // If bh >= 2: x0 = {(2^64-1)/bh, 0} (reciprocal < 1.0, stored in low word)
                // If bh == 0: b < 1.0, 1/b > 1.0. x0 = {0, (2^64-1)/bl}
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 0: reciprocal > 1.0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 1
                // Check if bh == 1
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 1: x0 = {0, 1} (≈ 1.0)
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 2: x0 = {(2^64-1)/bh, 0}
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::End); // bh == 1
                v.push(Instruction::End); // bh == 0

                // Newton iterations: x = x * (2 - b*x), 3 iterations
                for _ in 0..3 {
                    // t = b * x (Q64.64 multiply)
                    emit_fp64_mul(&mut v, bl, bh, x_lo, x_hi, tx_lo, tx_hi);
                    // correction = 2.0 - t (Q64.64 subtraction)
                    // cl = 0 - tx_lo (with borrow)
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(tx_lo)); v.push(Instruction::I64Sub); v.push(Instruction::LocalTee(cl));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtU); // borrow if cl wrapped (cl > 0 when it should be 0-tx_lo)
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // ch = 2 - tx_hi - borrow
                    v.push(Instruction::I64Const(2));
                    v.push(Instruction::LocalGet(tx_hi)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(ch));
                    // x = x * correction (Q64.64 multiply)
                    emit_fp64_mul(&mut v, x_lo, x_hi, cl, ch, x_lo, x_hi);
                }

                // Final: result = a * x (Q64.64 multiply)
                emit_fp64_mul(&mut v, al, ah, x_lo, x_hi, rl, rh);

                // Store result to dst
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }


            // ── fp64/sqrt: Q64.64 square root via 128-bit Newton's method ──
            // (fp64/sqrt dst src) — reads Q64.64 from src, writes sqrt(src) to dst
            // Computes isqrt(V) for V = vh*2^64+vl (128-bit), then stores as Q64.64
            "fp64/sqrt" => {
                // Q64.64 Newton: r = (r + V/r) / 2, iterated
                // Work directly in Q64.64 with {rl, rh} as the estimate
                // V/r approximated with high-word division (Newton is self-correcting)
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fsqrt_d");
                let src_i = self.local_idx("__fsqrt_s");
                let vh = self.local_idx("__fsqrt_vh");
                let vl = self.local_idx("__fsqrt_vl");
                let rh = self.local_idx("__fsqrt_rh");
                let rl = self.local_idx("__fsqrt_rl");
                let prev_rh = self.local_idx("__fsqrt_prh");
                let qh = self.local_idx("__fsqrt_qh");
                let ql = self.local_idx("__fsqrt_ql");
                let sum_l = self.local_idx("__fsqrt_sl");
                let sum_h = self.local_idx("__fsqrt_sh");
                let tmp = self.local_idx("__fsqrt_tmp");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load Q64.64 value V = {vl, vh}
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vh));

                // Handle V == 0
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Eqz);
                v.push(Instruction::I32And);
                v.push(Instruction::If(BlockType::Empty));
                // V == 0: result = 0
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // Initial guess: r = isqrt(vh) as Q64.64 {0, isqrt(vh)}
                // Use 64-bit Newton to compute isqrt(vh)
                let r64 = self.local_idx("__fsqrt_r64");
                let p64 = self.local_idx("__fsqrt_p64");
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vh));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);

                // Handle isqrt(vh) == 0 (vh was 0 or 1)
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // r64 == 0: do isqrt(vl) instead, result = isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vl));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Store isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // r64 > 0: initial Q64.64 guess r = {0, r64}
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rl));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(rh));

                // Q64.64 Newton: r = (r + V/r) / 2, 6 iterations
                // V/r uses high-word division with refinement: q_hi = vh/rh, q_lo estimated
                for _ in 0..6 {
                    // V/r: simplified Q64.64 division
                    // If rh == 0: q = {0xFFFFFFFFFFFFFFFF / max(rl,1), 0} (rough)
                    // Else: q_hi = vh / rh, q_lo from remainder refinement
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    // rh == 0: rough estimate
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::Else);
                    v.push(Instruction::I64Const(-1)); v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::End);
                    v.push(Instruction::Else);
                    // rh > 0: q_hi = vh / rh, remainder for q_lo refinement
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(qh));
                    // remainder_hi = vh % rh
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64RemU);
                    v.push(Instruction::LocalSet(tmp));
                    // q_lo ≈ (remainder_hi << 32 + (vl >> 32)) / rh << 32 ... simplified:
                    // q_lo ≈ (remainder_hi * 2^64) / rh, but use 64-bit approx:
                    // q_lo = (remainder_hi << 32 | vl >> 32) / rh ... but this might overflow
                    // Simpler: q_lo = ((tmp << 32) + (vl >> 32)) / rh
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::End);

                    // sum = r + q (Q64.64 add with carry)
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::LocalGet(ql)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(sum_l));
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::LocalGet(qh)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(sum_h));

                    // r = sum >> 1 (Q64.64 right shift by 1)
                    // new_rl = (sum_l >> 1) | (sum_h << 63)
                    // new_rh = sum_h >> 1
                    v.push(Instruction::LocalGet(sum_l)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(63)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or); v.push(Instruction::LocalSet(rl));
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(rh));
                }

                // Store result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // r64 == 0
                v.push(Instruction::End); // V == 0
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // ── tick_to_price64: Q64.64 via Q32.32 + shift ──
            // (tick_to_price64 addr tick) — writes Q64.64 1.0001^tick to mem[addr..addr+15]
            // Uses proven Q32.32 binary exponentiation, then shifts left by 32 for Q64.64
            "tick_to_price64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tp64_a");
                let t_i = self.local_idx("__tp64_t");
                let neg_i = self.local_idx("__tp64_neg");
                let r_i = self.local_idx("__tp64_r");
                let b_i = self.local_idx("__tp64_b");
                let mut v = Vec::new();
                v.extend(addr_expr); v.push(Instruction::LocalSet(addr_i));
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // result = 1.0 in Q32.32 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Binary exponentiation loop (same proven Q32.32 mul with 16-bit split)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // Q32.32 mul: r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Convert Q32.32 → Q64.64: shift left by 32
                // Store lo = (r << 32) at addr
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Store hi = (r >> 32) at addr+8
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }


            // ── tick_to_sqrtPrice64: sqrt(1.0001^tick) in Q64.64 ──
            // (tick_to_sqrtPrice64 addr tick) — writes Q64.64 sqrtPrice to mem[addr]
            // sqrtPrice = sqrt(1.0001^tick) = 1.0001^(tick/2)
            // Uses Q32.32 binary exponentiation with tick/2, then shifts to Q64.64
            // This avoids the full price → sqrt pipeline and gives better precision
            "tick_to_sqrtPrice64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tsp_a");
                let half_tick = self.local_idx("__tsp_ht");
                let is_odd = self.local_idx("__tsp_odd");
                let t_i = self.local_idx("__tsp_t");
                let neg_i = self.local_idx("__tsp_neg");
                let r_i = self.local_idx("__tsp_r");
                let b_i = self.local_idx("__tsp_b");
                let mut v = Vec::new();
                v.extend(addr_expr); v.push(Instruction::LocalSet(addr_i));
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // Remember if odd: is_odd = tick & 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(is_odd));
                // half_tick = tick >> 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(half_tick));
                // Compute 1.0001^half_tick in Q32.32
                // result = 1.0 in Q32.32 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Loop: while half_tick > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if half_tick & 1: r *= b
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                v.push(Instruction::LocalGet(half_tick)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(half_tick));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // If tick was odd: multiply by sqrt(1.0001) ≈ 1.00005 in Q32.32
                // 1.00005 * 2^32 = 4294970534 ≈ 0x1000068DA
                v.push(Instruction::LocalGet(is_odd)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0x10000)); // 1.00005 hi
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0x68DA)); // 1.00005 lo
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(0x10000)); // 1.00005 hi
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Invert if negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Convert Q32.32 → Q64.64: shift left by 32
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (sqrt x) → i64 — integer square root via Newton's method
            // For CLMM: use on price values. Returns floor(sqrt(x))
            "sqrt" => {
                let x = self.expr(&a[0])?;
                let x_i = self.local_idx("__sq_x");
                let r_i = self.local_idx("__sq_r");
                let prev_i = self.local_idx("__sq_p");
                let mut v = Vec::new();
                v.extend(x); v.push(Instruction::LocalSet(x_i));
                // if x == 0: return 0
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::I64Eqz); // → i32
                v.push(Instruction::I32Eqz); // invert: x != 0 → enter then branch
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Initial guess: x >> 1 (rough sqrt)
                // Better: r = 1 << ((64 - clz(x)) / 2)
                // Simple: r = x, iterate r = (r + x/r) / 2
                v.push(Instruction::LocalGet(x_i)); v.push(Instruction::LocalSet(r_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::LocalSet(prev_i));
                // r = (r + x/r) / 2
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::I64DivU); // x / r
                v.push(Instruction::I64Add); // r + x/r
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); // / 2
                v.push(Instruction::LocalSet(r_i));
                // if r >= prev: converged, break
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(prev_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2)); // exit outer block
                v.push(Instruction::End);
                v.push(Instruction::Br(0)); // loop
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(r_i)); // return prev (last decreasing value)
                // Actually if r >= prev, prev is the answer (converged from above)
                // But we want the one that stopped decreasing
                v.pop(); // remove the LocalGet r_i
                v.push(Instruction::LocalGet(prev_i)); // prev was last decreasing
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0)); // x == 0 case
                v.push(Instruction::End);
                Ok(v)
            }

            // (fp/sqrt x) → i64 — Q64.64 square root: sqrt(x << 64) >> 32 = sqrt(x) << 32
            // Returns Q64.64 fixed-point sqrt
            "fp/sqrt" => {
                // sqrt(Q64.64) = sqrt(x * 2^64) = sqrt(x) * 2^32
                // = sqrt(x) << 32 in Q64.64
                // Use integer sqrt of (x >> 32) then << 48... 
                // Actually: want sqrt(x) where x is Q64.64
                // = isqrt(x) if x were the full number
                // Since x = (real_val) << 64, sqrt(x) = sqrt(real_val) << 32
                // = isqrt(x >> 64) << 32 ... no
                // Better: isqrt(x >> 32) << 16 ... losing precision
                // Best: isqrt(x) then shift. x is Q64.64 so ~64 bits of fraction
                // isqrt(x) gives sqrt with ~32 bits of fraction implicitly
                // But x can be up to 128 bits. Use two-part method:
                // Split x = hi << 64 | lo
                // sqrt = sqrt(hi) << 32 + adjustment
                // For CLMM: we mostly sqrt prices ~1-1000, so hi is small
                // Just use: (sqrt(x >> 32)) << 16 as approximation? No.
                // Correct approach: isqrt(x) where x is treated as uint128
                // We can do: r = isqrt(high * 2^64 + low)
                // ≈ isqrt(high) << 32 + low / (2 * isqrt(high) << 32)
                // For simplicity: (sqrt (x >> 32)) << 16 gives OK precision for CLMM
                // Actually the correct Q64.64 sqrt: 
                //   result = isqrt(x) where we need 128-bit isqrt
                //   Split: a = x >> 64, b = x & ((1<<64)-1)
                //   r = isqrt(a) << 32
                //   remainder = a - r^2 (in high bits)  
                //   r = (r << 64 + b) correction via Newton
                // Simplest correct: compute integer sqrt of (x >> 32), then << 16
                // This gives Q64.32 result, need to shift to Q64.64: << 32 more = << 48
                // NO. Let me think again.
                // Q64.64 value V represents real number v = V / 2^64
                // We want sqrt(v) * 2^64 = sqrt(V/2^64) * 2^64 = sqrt(V) * 2^32
                // So: fp/sqrt(x) = isqrt(x) * 2^32 ... but isqrt of a Q64.64 number
                // that's at most ~2^127 gives result ~2^63, then * 2^32 overflows
                // 
                // Better: fp/sqrt(x) = isqrt(x) >> 0, since isqrt(Q64.64) already has
                // the right scale? No.
                //
                // Simplest: fp/sqrt(x) = isqrt(x) for Q64.64 input
                // If x = 1.0 = 2^64, isqrt(2^64) = 2^32 = 0.5 in Q64.64... wrong
                // We want sqrt(1.0) = 1.0 = 2^64
                // So: fp/sqrt(x) = isqrt(x << 64) ... but that overflows
                //
                // Practical CLMM: use Q64 for sqrt price, separate from Q64.64
                // (fp/sqrt x) = (sqrt x) gives integer sqrt, caller manages scaling
                // Just delegate to integer sqrt
                // User does: (fp/from_int (sqrt (fp/to_int price_approx)))
                // Or: (sqrt x) << 32 for Q64.64 sqrt of integer
                // I'll just delegate — fp/sqrt is an alias for careful scaling
                let inner = &LispVal::List(vec![
                    LispVal::Sym("sqrt".into()), a[0].clone()
                ]);
                // After sqrt, shift left by 32 to get Q64.64 result from Q64.0 input
                // Wait — sqrt of Q64.64 = isqrt(x) which gives wrong scale
                // For Q64.64 input x representing value X: x = X * 2^64
                // sqrt(x) in same format = sqrt(X) * 2^64 = sqrt(X * 2^64) * 2^32
                // = isqrt(x) * 2^32
                let mut v = self.expr(inner)?;
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                Ok(v)
            }

            // (clz x) → i64 — count leading zeros (for tick bitmap)
            // WASM doesn't have clz for i64, use i64.clz
            "clz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Clz);
                Ok(v)
            }

            // (ctz x) → i64 — count trailing zeros
            "ctz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Ctz);
                Ok(v)
            }

            // (popcnt x) → i64 — population count (for tick bitmap)
            "popcnt" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Popcnt);
                Ok(v)
            }

            // (bit_get x idx) → i64 — get bit at index (0 or 1)
            "bit_get" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.extend(idx);
                v.push(Instruction::I64ShrU); // x >> idx
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And); // & 1
                Ok(v)
            }

            // (bit_set x idx) → i64 — set bit at index
            "bit_set" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Or); // x | (1 << idx)
                Ok(v)
            }

            // (bit_clr x idx) → i64 — clear bit at index
            "bit_clr" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Const(-1i64)); // all ones
                v.push(Instruction::I64Xor); // ~(1 << idx)
                v.push(Instruction::I64And); // x & ~(1 << idx)
                Ok(v)
            }

            // (tick_to_price tick) → i64 — 1.0001^tick in Q64.64
            // Uses binary exponentiation with Q64.64 multiply
            // 1.0001^tick = exp(tick * ln(1.0001))
            // ln(1.0001) ≈ 0.000099995 in Q64.64 ≈ 0x29C3E3
            // For small ticks (|tick| < 887272), iterative multiply works
            // We do: result = 1.0; for each bit of tick, square base; if bit set, multiply
            "tick_to_price" => {
                // Binary exponentiation: 1.0001^tick in Q32.32
                let tick = self.expr(&a[0])?;
                let t_i = self.local_idx("__ttp_t");
                let r_i = self.local_idx("__ttp_r");
                let b_i = self.local_idx("__ttp_b");
                let neg_i = self.local_idx("__ttp_neg");
                let c_i = self.local_idx("__ttp_c");
                let mut v = Vec::new();
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // result = 1.0 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Loop: while tick > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b (Q32.32 mul with 16-bit split)
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // r_hi * b_hi
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End); // if
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative: r = (1<<48) / r << ... actually just (1<<32) * (1<<16) / r
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // 1/r ≈ (1 << 48) / r, then >> 16 to get back to Q32.32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(r_i));
                Ok(v)
            }

            // (price_to_tick price_q64) → i64 — inverse of tick_to_price (approximate)
            // tick = log(price) / log(1.0001)
            // log(1.0001) ≈ 0.000099995 ≈ Q64.64: 0x29C3E3
            // log(price) via binary log: find msb, iterate
            // For CLMM: usually price is from tick_to_price, so exact inverse via lookup
            // Approximation: tick ≈ (price_q64 - 1<<64) * 10000 (first-order Taylor)
            "price_to_tick" => {
                let p = self.expr(&a[0])?;
                let p_i = self.local_idx("__ptp_p");
                let mut v = Vec::new();
                v.extend(p); v.push(Instruction::LocalSet(p_i));
                // First order: tick ≈ (p - 1.0) / log(1.0001) ≈ (p - 1<<64) * 10000
                // More precisely: (p - (1<<64)) >> 64 * 10000 gives integer approximation
                v.push(Instruction::LocalGet(p_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(64)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU); // to integer
                v.push(Instruction::I64Const(10000));
                v.push(Instruction::I64Mul);
                Ok(v)
            }

            // (liquidity_amount0 sqrt_price_a sqrt_price_b liquidity) → Q64.64
            // amount0 = L * (1/sqrtPa - 1/sqrtPb) for Pa < Pb
            // = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
            "liq_amount0" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la0_a"); let spb_i = self.local_idx("__la0_b"); let liq_i = self.local_idx("__la0_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // numerator = liq * (spb - spa) — Q64.64 mul
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(liq_i)); // reuse as numerator
                // denominator = spa * spb — Q64.64 mul
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                // result = numerator / denominator
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64DivU);
                Ok(v)
            }

            // (liquidity_amount1 sqrt_price_a sqrt_price_b liquidity) → Q64.64
            // amount1 = L * (sqrtPb - sqrtPa)
            "liq_amount1" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la1_a"); let spb_i = self.local_idx("__la1_b"); let liq_i = self.local_idx("__la1_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // liq * (spb - spa) — Q64.64 multiply
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // ── String Operations (packed: low32=ptr, high32=len) ──

            // ── Q64.64 Memory-based CLMM operations ──

            // (liq_amount0_64 dst spa_addr spb_addr liq_addr)
            // amount0 = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
            // All Q64.64 in memory. Writes Q64.64 result to dst.
            "liq_amount0_64" => {
                // (liq_amount0_64 dst spa_addr spb_addr liq_addr)
                // amount0 = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
                // All Q64.64 memory. Uses high-word arithmetic for CLMM (prices ≈ 1.0)
                let dst = self.expr(&a[0])?;
                let spa_a = self.expr(&a[1])?;
                let spb_a = self.expr(&a[2])?;
                let liq_a = self.expr(&a[3])?;
                let dst_i = self.local_idx("__la0_d");
                let spa_lo = self.local_idx("__la0_sl");
                let spa_hi = self.local_idx("__la0_sh");
                let spb_lo = self.local_idx("__la0_bl");
                let spb_hi = self.local_idx("__la0_bh");
                let liq_hi = self.local_idx("__la0_lh");
                let diff_lo = self.local_idx("__la0_dl");
                let diff_hi = self.local_idx("__la0_dh");
                let num_hi = self.local_idx("__la0_nh");
                let den_hi = self.local_idx("__la0_dnh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                // Load all values upfront into locals
                v.extend(spa_a); v.push(Instruction::LocalSet(spa_lo)); // spa addr
                v.extend(spb_a); v.push(Instruction::LocalSet(spb_lo)); // spb addr
                v.extend(liq_a); v.push(Instruction::LocalSet(liq_hi)); // liq addr
                // Load spa Q64.64
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(diff_lo)); // spa low temporarily
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                v.push(Instruction::LocalGet(diff_lo)); v.push(Instruction::LocalSet(spa_lo)); // proper spa_lo
                // Load spb Q64.64
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_lo)); // spb low
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64); // need spb addr for hi
                // Wait, spb_lo is now the spb value, not addr. Need separate addr local.
                // Let me restructure with addr locals
                v.clear();
                // Redo with proper addr locals
                let dst2 = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la0_as");
                let addr_spb = self.local_idx("__la0_ab");
                let addr_liq = self.local_idx("__la0_al");
                v.extend(dst2); v.push(Instruction::LocalSet(dst_i));
                // Store addresses in locals
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load spa
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                // Load spb
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_hi));
                // Load liq
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_hi));
                // diff_hi = spb_hi - spa_hi
                v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff_hi));
                // numerator = liq_hi * diff_hi
                v.push(Instruction::LocalGet(liq_hi)); v.push(Instruction::LocalGet(diff_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(num_hi));
                // denominator = spa_hi * spb_hi (both ≈ 1, so ≈ 1)
                v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(den_hi));
                // result = numerator / denominator
                v.push(Instruction::LocalGet(num_hi)); v.push(Instruction::LocalGet(den_hi)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(num_hi));
                // Store: lo=0, hi=result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(num_hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            "liq_amount1_64" => {
                // (liq_amount1_64 dst spa_addr spb_addr liq_addr)
                // amount1 = L * (sqrtPb - sqrtPa)
                let dst = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la1_as");
                let addr_spb = self.local_idx("__la1_ab");
                let addr_liq = self.local_idx("__la1_al");
                let dst_i = self.local_idx("__la1_d");
                let spa_h = self.local_idx("__la1_sh");
                let spb_h = self.local_idx("__la1_bh");
                let liq_h = self.local_idx("__la1_lh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load high words
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_h));
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_h));
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_h));
                // result_hi = liq_h * (spb_h - spa_h)
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::LocalGet(spb_h)); v.push(Instruction::LocalGet(spa_h)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(liq_h));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (price64_to_tick addr) → i64
            // Reads Q64.64 price from addr, returns tick = log(price) / log(1.0001)
            // Uses binary log: find MSB, iterate for fractional bits
            "price64_to_tick" => {
                // (price64_to_tick addr) → i64
                // Linear approximation: tick ≈ (price-1) * 10001
                // Good for ±500 ticks (< 0.5% error), acceptable for CLMM range queries
                // For wider range: iterate with tick_to_price64 refinement
                let pa = self.expr(&a[0])?;
                let addr_i = self.local_idx("__p2t_a");
                let ph = self.local_idx("__p2t_ph");
                let pl = self.local_idx("__p2t_pl");
                let diff = self.local_idx("__p2t_d");
                let tick = self.local_idx("__p2t_t");
                let mut v = Vec::new();
                v.extend(pa); v.push(Instruction::LocalSet(addr_i));
                // Load Q64.64 and convert to Q32.32
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ph));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(pl));
                // q32 = (ph << 32) | (pl >> 32)
                v.push(Instruction::LocalGet(ph)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(pl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or);
                // diff = q32 - (1<<32)
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff));
                // tick = diff * 10001 >> 32
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Const(10001)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                // Quadratic correction for larger range: subtract diff^2 * 5002 >> 64
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(5002)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(tick));
                v.push(Instruction::LocalGet(tick)); Ok(v)
            }


            // (str_len s) → i64 — extract high 32 bits
            "str_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }

            // (str_cat s1 s2) → packed string (allocates new memory)
            // Uses __stralloc counter for bump allocation
            "str_cat" => {
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__sc1");
                let s2_i = self.local_idx("__sc2");
                let l1_i = self.local_idx("__scl1");
                let l2_i = self.local_idx("__scl2");
                let dst_i = self.local_idx("__scdst");
                let i_i = self.local_idx("__sci");
                let alloc_i = self.local_idx("__stralloc");
                let mut v = Vec::new();
                // Save packed strings
                v.extend(s1); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.push(Instruction::LocalSet(s2_i));
                // Extract lengths
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l2_i));
                // dst = bump allocator (starts at 2048, or wherever data ends)
                // Use self.next_data_offset as compile-time base, increment for each str_cat
                let alloc_base = self.next_data_offset.max(2048);
                // We'll use a data-offset approach: bump alloc from a high address
                v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(dst_i));
                // Update next_data_offset for future allocations
                self.next_data_offset = alloc_base; // will be bumped at end
                // Copy s1: for i in 0..l1 { mem[dst+i] = mem[ptr1+i] }
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // while i < l1
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // dst[i] = s1_ptr[i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block

                // Copy s2: for i in 0..l2 { mem[dst+l1+i] = mem[ptr2+i] }
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block

                // Return packed: ((l1+l2) << 32) | dst
                let total_len = 0; // computed at runtime
                v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Or);
                // Bump allocator
                let new_off = (alloc_base + 1024) & !7; // reserve 1KB max
                self.next_data_offset = new_off;
                Ok(v)
            }

            // (str_eq s1 s2) → i64 (0 or 1)
            "str_eq" => {
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__se1");
                let s2_i = self.local_idx("__se2");
                let l1_i = self.local_idx("__sel1");
                let i_i = self.local_idx("__sei");
                let res_i = self.local_idx("__seres");
                let mut v = Vec::new();
                v.extend(s1); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.push(Instruction::LocalSet(s2_i));
                // l1 = s1 >> 32
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                // if l1 != (s2 >> 32) → 0
                v.push(Instruction::LocalGet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Compare byte by byte
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(res_i)); // assume equal
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if s1_ptr[i] != s2_ptr[i]: res=0, break
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Ne);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(res_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(res_i));
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }

            // (str_to_int s) → i64 — parse decimal string
            "str_to_int" => {
                let s = self.expr(&a[0])?;
                let s_i = self.local_idx("__sti_s");
                let len_i = self.local_idx("__sti_len");
                let i_i = self.local_idx("__sti_i");
                let acc_i = self.local_idx("__sti_acc");
                let ch_i = self.local_idx("__sti_ch");
                let neg_i = self.local_idx("__sti_neg");
                let mut v = Vec::new();
                v.extend(s); v.push(Instruction::LocalSet(s_i));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Check for leading '-'
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // if ch == '-' (45)
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(45)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(i_i)); // skip '-'
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // ch = s_ptr[i]
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // acc = acc * 10 + (ch - 48)
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0)); // fallback
                v.push(Instruction::End); // block
                // Apply negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Sub); // 0 - acc
                v.push(Instruction::Else);
                // Identity — but we need the value on stack. It's already there from the block.
                // Hmm, the block result is already on the stack. The if consumes it.
                // We need to save it to a local first.
                v.pop(); // remove the Else we just added
                // Save block result, then branch
                v.push(Instruction::LocalSet(acc_i)); // save
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::End);
                Ok(v)
            }

            // (int_to_str n) → packed string
            "int_to_str" => {
                let n = self.expr(&a[0])?;
                let n_i = self.local_idx("__its_n");
                let neg_i = self.local_idx("__its_neg");
                let tmp_i = self.local_idx("__its_tmp");
                let len_i = self.local_idx("__its_len");
                let dst_i = self.local_idx("__its_dst");
                let dig_i = self.local_idx("__its_dig");
                let i_i = self.local_idx("__its_i");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 64) & !7;
                let mut v = Vec::new();
                v.extend(n); v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(dst_i));
                // Handle negative: if n < 0, neg=1, n = -n
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::End);
                // Handle n == 0
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // Write '0' at dst, len=1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(48)); // '0'
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::Else);
                // Extract digits in reverse: write to dst+31 backward
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(31)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(tmp_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // dig = n % 10
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64RemU); v.push(Instruction::LocalSet(dig_i));
                // n /= 10
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64DivU); v.push(Instruction::LocalSet(n_i));
                // mem[tmp] = '0' + dig
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dig_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(tmp_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Now digits are at [tmp+8 .. dst+31], need to move to dst[0..len-1]
                // Copy forward
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // dst[i] = (tmp+8+len-1-i)  ... actually source is at dst + (31 - len + 1 + i) = dst + 32 - len + i
                // We wrote backward from dst+31, so digits start at tmp+8 (= dst+31-len+1 = dst+32-len)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Sub); // dst+32 ... wait
                // Actually: we started at tmp=dst+31, wrote at tmp, then tmp-=8 each step.
                // After len digits: tmp = dst+31 - (len-1)*8 ... no wait, we sub 8 not 1!
                // BUG: we're storing bytes but subtracting 8. Should subtract 1.
                // Let me fix: use I32Store8 so we should subtract 1 from the pointer.
                // Actually I used I32Store8 which stores a single byte, but tmp is i64 and I subtract 8.
                // That's wrong — should subtract 1 for byte addressing.
                v.push(Instruction::End); // end the broken block early
                v.push(Instruction::End); // end if/else
                // This is getting messy. Let me restart int_to_str with a cleaner approach.
                // Actually, let me just rewrite the whole thing properly.
                return self.int_to_str_clean(&a);
            }

            // ── Array Operations ──
            // Layout: length at (offset-8), elements at offset + idx*8

            // (arr_new offset size) — zero-fill
            "arr_new" => {
                let offset_expr = self.expr(&a[0])?;
                let size_expr = self.expr(&a[1])?;
                let off_i = self.local_idx("__an_off");
                let sz_i = self.local_idx("__an_sz");
                let i_i = self.local_idx("__an_i");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off_i));
                v.extend(size_expr); v.push(Instruction::LocalSet(sz_i));
                // Store length at offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(sz_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Zero-fill loop
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(sz_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[offset + i*8] = 0
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block
                Ok(v)
            }

            // (arr_get offset idx) → i64
            "arr_get" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (arr_set offset idx val)
            "arr_set" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let val = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (arr_len offset) → i64 — reads from offset-8
            "arr_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }

            // (arr_push offset val) — append, increment length
            "arr_push" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__ap_off");
                let len_i = self.local_idx("__ap_len");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // Load current length from offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(len_i));
                // Store val at offset + len*8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Increment length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (arr_sort offset) — bubble sort in-place
            "arr_sort" => {
                // Bubble sort: arr[offset..offset+n*8]
                // Length stored at offset-8
                let off = self.expr(&a[0])?;
                let off_i = self.local_idx("__as_off");
                let n_i = self.local_idx("__as_n");
                let i_i = self.local_idx("__as_i");
                let j_i = self.local_idx("__as_j");
                let tmp_i = self.local_idx("__as_tmp");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // n = mem[(offset-8)]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                // Outer loop: i = 0..n-1
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n-1: br 2 (exit)
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // j = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                // Inner loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= n-i-1: br 2
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Sub); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // tmp = arr[j], load arr[j+1]
                // Compare: if arr[j] > arr[j+1], swap
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(tmp_i)); // tmp = arr[j]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); // arr[j+1]
                // stack: arr[j+1]; tmp_i = arr[j]
                // if arr[j] > arr[j+1] → swap
                v.push(Instruction::LocalGet(tmp_i)); // tmp, arr[j+1] on stack
                v.push(Instruction::I64LtS); // arr[j+1] < arr[j] i.e. arr[j] > arr[j+1]
                v.push(Instruction::If(BlockType::Empty));
                // arr[j] = arr[j+1]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // arr[j+1] = tmp
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // if swap
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // (arr_find offset val) → index or -1 (linear search)
            "arr_find" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__af_off");
                let val_i = self.local_idx("__af_val");
                let n_i = self.local_idx("__af_n");
                let i_i = self.local_idx("__af_i");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // Load length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(-1)); v.push(Instruction::Br(2)); // not found
                v.push(Instruction::End);
                // if arr[i] == val → return i
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::Br(2)); // found
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(-1)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }

            // User function call
            _ => {
                let pos = self.funcs.iter().position(|f| f.name == op).ok_or_else(|| {
                    let mut msg = format!("in {}: function '{}' is not defined", self.current_func.as_deref().unwrap_or("top"), op);
                    // Suggest closest match from user-defined functions
                    let candidates: Vec<&str> = self.funcs.iter()
                        .filter(|f| !f.name.starts_with("__"))
                        .map(|f| f.name.as_str())
                        .collect();
                    let builtins = ["+", "-", "*", "/", "mod", "abs", "=", "!=", "<", ">", "<=", ">=",
                        "and", "or", "not", "if", "let", "begin", "while", "for", "set!", "quote",
                        "near/log", "near/return", "near/store", "near/load", "near/storage_set", "near/storage_get", "near/storage_has", "near/storage_remove", "near/log_num",
                        "hof/map", "hof/filter", "hof/reduce"];
                    let all_candidates: Vec<&str> = candidates.iter().chain(builtins.iter()).copied().collect();
                    let mut best: Option<(&str, usize)> = None;
                    for c in &all_candidates {
                        let dist = levenshtein(op, c);
                        if dist <= 3 {
                            match best {
                                Some((_, best_dist)) if dist >= best_dist => {}
                                _ => best = Some((*c, dist)),
                            }
                        }
                    }
                    if let Some((suggestion, _)) = best {
                        msg.push_str(&format!(". Did you mean '{}'?", suggestion));
                    }
                    msg
                })?;
                let mut v = Vec::new();
                for x in a { v.extend(self.expr(x)?); }
                v.push(Instruction::Call(USER_BASE | pos as u32));
                Ok(v)
            }
        }
    }

    // Helper: call host(register_id=0), read_register(0, TEMP_MEM), register_len(0), return packed (ptr|len<<32)
    fn read_to_register(&mut self, host_idx: usize, _a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
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
    fn read_u128_low(&mut self, host_idx: usize) -> Result<Vec<Instruction<'static>>, String> {
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
    fn read_u128_high(&mut self, host_idx: usize) -> Result<Vec<Instruction<'static>>, String> {
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

    // Clean int_to_str implementation
    fn int_to_str_clean(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let n = self.expr(&a[0])?;
        let n_i = self.local_idx("__its2_n");
        let neg_i = self.local_idx("__its2_neg");
        let len_i = self.local_idx("__its2_len");
        let dst_i = self.local_idx("__its2_dst");
        let tmp_i = self.local_idx("__its2_tmp");
        let dig_i = self.local_idx("__its2_dig");
        let i_i = self.local_idx("__its2_i");
        let src_i = self.local_idx("__its2_src");
        let alloc_base = self.next_data_offset.max(3072);
        self.next_data_offset = (alloc_base + 64) & !7;
        let mut v = Vec::new();
        v.extend(n); v.push(Instruction::LocalSet(n_i));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(dst_i));
        // Handle negative
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(n_i));
        v.push(Instruction::End);
        // Handle n == 0
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(48));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::Else);
        // Extract digits backward at dst+31
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(31)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(tmp_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // dig = n % 10
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64RemU); v.push(Instruction::LocalSet(dig_i));
        v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64DivU); v.push(Instruction::LocalSet(n_i));
        // mem[tmp] = '0' + dig
        v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(dig_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(tmp_i));
        v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        // Digits are at [tmp+1 .. dst+31], copy to dst[0..len-1]
        v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(src_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        v.push(Instruction::End); // if/else n==0
        // Prepend '-' if negative
        v.push(Instruction::LocalGet(neg_i));
        v.push(Instruction::If(BlockType::Empty));
        // Shift digits right by 1, write '-' at dst[0]
        v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(45)); // '-'
        v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::End);
        // Return packed: (len << 32) | dst
        v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Or);
        Ok(v)
    }

    fn fold_binop(&mut self, a: &[LispVal], op: Instruction<'static>, identity: i64) -> Result<Vec<Instruction<'static>>, String> {
        if a.is_empty() { return Ok(vec![Instruction::I64Const(identity)]) }
        let mut v = self.expr(&a[0])?;
        for x in &a[1..] { v.extend(self.expr(x)?); v.push(op.clone()); }
        Ok(v)
    }

    fn cmp(&mut self, a: &[LispVal], op: Instruction<'static>) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.expr(&a[1])?);
        v.push(op); v.push(Instruction::I64ExtendI32U); Ok(v)
    }

    // ── Module assembly ──

    pub fn finish(&mut self, default_export: &str) -> Vec<u8> {
        // Ensure host functions needed by export wrappers are included
        if !self.exports.is_empty() {
            self.need_host(7);  // input
            self.need_host(1);  // register_len
            self.need_host(0);  // read_register
            self.need_host(25); // value_return
        }
        let mut m = Module::new();
        let host_list: Vec<usize> = (0..HOST_FUNCS.len()).filter(|i| self.host_needed.contains(i)).collect();
        let host_count = host_list.len() as u32;

        // Type section
        let mut types = TypeSection::new();
        types.ty().function([], []); // type 0: () -> ()
        let max_p = self.funcs.iter().map(|f| f.param_count).max().unwrap_or(0);
        for p in 0..=max_p {
            let params: Vec<ValType> = (0..p).map(|_| ValType::I64).collect();
            types.ty().function(params, [ValType::I64]);
        }
        let host_type_base = (max_p + 2) as u32;
        for &hi in &host_list {
            types.ty().function(HOST_FUNCS[hi].1.iter().copied(), HOST_FUNCS[hi].2.iter().copied());
        }
        m.section(&types);

        // Import section (host functions only)
        let mut imports = ImportSection::new();
        let mut host_idx: HashMap<usize, u32> = HashMap::new();
        for (i, &hi) in host_list.iter().enumerate() {
            imports.import("env", HOST_FUNCS[hi].0, EntityType::Function(host_type_base + i as u32));
            host_idx.insert(hi, i as u32);
        }
        m.section(&imports);

        // Function section
        let mut funcs = FunctionSection::new();
        for f in &self.funcs { funcs.function(f.param_count as u32 + 1); }
        if self.exports.is_empty() {
            funcs.function(0); // default wrapper: () -> ()
        } else {
            for (fn_name, _, _) in &self.exports {
                let func = self.funcs.iter().find(|f| f.name.as_str() == fn_name.as_str());
                let param_count = func.map(|f| f.param_count).unwrap_or(0);
                // Wrapper type: (i64 × param_count) -> () — same as type param_count+1 but returns nothing
                // For simplicity, use type 0 for now (NEAR passes args via input() anyway)
                // TODO: create proper wrapper types
                let _ = param_count;
                funcs.function(0);
            }
        }
        m.section(&funcs);

        // Memory (internal, exported — same as near-sdk output)
        let mut mems = MemorySection::new();
        mems.memory(MemoryType { minimum: self.memory_pages.max(1) as u64, maximum: None, memory64: false, shared: false, page_size_log2: None });
        m.section(&mems);

        // Global section: mutable i64 for call depth tracking
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType { val_type: ValType::I64, mutable: true, shared: false },
            &ConstExpr::i64_const(0),
        );
        m.section(&globals);

        // Exports
        let mut exps = ExportSection::new();
        exps.export("memory", ExportKind::Memory, 0);
        let internal_base = host_count;
        let wrapper_base = internal_base + self.funcs.len() as u32;
        if self.exports.is_empty() {
            if !self.funcs.is_empty() { exps.export(default_export, ExportKind::Func, wrapper_base); }
        } else {
            for (i, (_, en, _)) in self.exports.iter().enumerate() {
                exps.export(en, ExportKind::Func, wrapper_base + i as u32);
            }
        }
        m.section(&exps);

        // Code
        let name_map: HashMap<&str, u32> = self.funcs.iter().enumerate()
            .map(|(i, f)| (f.name.as_str(), internal_base + i as u32)).collect();
        let mut code = wasm_encoder::CodeSection::new();
        for f in &self.funcs {
            let extra = f.local_count.saturating_sub(f.param_count);
            let locals: Vec<(u32, ValType)> = if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] };
            let resolved = Self::resolve_static(&f.instrs, &host_idx, &name_map, &self.funcs);
            let mut fb = Function::new(locals);
            for instr in &resolved { fb.instruction(instr); }
            fb.instruction(&Instruction::End);
            code.function(&fb);
        }
        // Wrappers
        if self.exports.is_empty() {
            if let Some(f) = self.funcs.last() {
                let idx = internal_base + (self.funcs.len()-1) as u32;
                let mut fb = Function::new(vec![(1u32, ValType::I64)]); // local 0 for result swapping
                // Pass default args: for each param, push 100000 (for tight loop benchmarking)
                for _ in 0..f.param_count {
                    fb.instruction(&Instruction::I64Const(100000));
                }
                fb.instruction(&Instruction::Call(idx));
                fb.instruction(&Instruction::Drop);
                fb.instruction(&Instruction::End);
                code.function(&fb);
            }
        } else {
            for (fn_name, _, _) in &self.exports {
                if let Some(&idx) = name_map.get(fn_name.as_str()) {
                    let func = self.funcs.iter().find(|f| f.name.as_str() == fn_name.as_str());
                    let param_count = func.map(|f| f.param_count).unwrap_or(0);
                    let mut fb = Function::new(vec![(1u32, ValType::I64)]); // local 0 for result swapping
                    let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                    if param_count == 0 {
                        fb.instruction(&Instruction::Call(idx));
                        fb.instruction(&Instruction::LocalSet(0));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::I32WrapI64);
                        fb.instruction(&Instruction::LocalGet(0));
                        fb.instruction(&Instruction::I64Store(ma));
                        fb.instruction(&Instruction::I64Const(8));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&25]));
                    } else {
                        // input(0)
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::Call(host_idx[&7]));
                        // register_len(0) — drop
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::Call(host_idx[&1]));
                        fb.instruction(&Instruction::Drop);
                        // read_register(0, TEMP_MEM)
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&0]));
                        // Load args
                        for i in 0..param_count {
                            fb.instruction(&Instruction::I64Const(TEMP_MEM + (i as i64) * 8));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::I64Load(ma));
                        }
                        fb.instruction(&Instruction::Call(idx));
                        // Store result at TEMP_MEM: i64.store needs [i32 addr, i64 val]
                        // Stack: [i64 result]. Save to local 0, push addr, load local, store
                        fb.instruction(&Instruction::LocalSet(0)); // save result to local 0
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::I32WrapI64);   // addr as i32
                        fb.instruction(&Instruction::LocalGet(0));  // restore result
                        fb.instruction(&Instruction::I64Store(ma));
                        // value_return(8, TEMP_MEM)
                        fb.instruction(&Instruction::I64Const(8));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&25]));
                    }
                    fb.instruction(&Instruction::End);
                    code.function(&fb);
                }
            }
        }
        m.section(&code);

        // Data (section 11 — must come after code section 10)
        if !self.data_segments.is_empty() {
            let mut data = DataSection::new();
            for (off, bytes) in &self.data_segments {
                data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
            }
            m.section(&data);
        }

        m.finish()
    }

    fn resolve_static(
        instrs: &[Instruction<'static>],
        host_map: &HashMap<usize, u32>,
        name_map: &HashMap<&str, u32>,
        funcs: &[FuncDef],
    ) -> Vec<Instruction<'static>> {
        instrs.iter().map(|i| match i {
            Instruction::Call(idx) if *idx >= HOST_BASE && *idx < USER_BASE => {
                Instruction::Call(host_map[&((*idx - HOST_BASE) as usize)])
            }
            Instruction::Call(idx) if *idx >= USER_BASE => {
                let pos = (*idx - USER_BASE) as usize;
                Instruction::Call(name_map[funcs[pos].name.as_str()])
            }
            other => other.clone(),
        }).collect()
    }
}

// ── Compile helpers ──

fn parse_and_compile(source: &str, near: bool) -> Result<WasmEmitter, String> {
    let exprs = crate::parser::parse_all(source)?;
    let mut em = WasmEmitter::new();
    for e in &exprs {
        if let LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (LispVal::Sym(s), LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                LispVal::Sym(s) => Ok(s.clone()), _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            // Implicit begin: wrap multiple body expressions
                            let body = if items.len() > 3 {
                                LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                    .chain(items[2..].iter().cloned()).collect())
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
                if near { if let LispVal::Sym(s) = &items[0] {
                    if s == "export" { if let (LispVal::Str(en), LispVal::Sym(fn_)) = (&items[1], &items[2]) {
                        let view = items.len()>3 && matches!(&items[3], LispVal::Bool(true));
                        em.add_export(fn_, en, view);
                    }}
                }}
            }
            if let (LispVal::Sym(s), Some(LispVal::Num(n))) = (&items[0], items.get(1)) {
                if s == "memory" { em.set_memory(*n as u32); }
            }
        }
    }
    Ok(em)
}

pub fn compile_pure(source: &str) -> Result<Vec<u8>, String> {
    Ok(parse_and_compile(source, false)?.finish("run"))
}

pub fn compile_near(source: &str) -> Result<Vec<u8>, String> {
    Ok(parse_and_compile(source, true)?.finish("_run"))
}

/// Compile to NEAR WASM and return function names in code-section order (for error reporting)
pub fn compile_near_named(source: &str) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut em = parse_and_compile(source, true)?;
    let names: Vec<String> = em.funcs.iter().map(|f| f.name.clone()).collect();
    let wasm = em.finish("_run");
    Ok((wasm, names))
}

/// Compile pre-parsed LispVal expressions to NEAR WASM
pub fn compile_near_from_exprs(exprs: &[LispVal]) -> Result<Vec<u8>, String> {
    let mut em = WasmEmitter::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (LispVal::Sym(s), LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                LispVal::Sym(s) => Ok(s.clone()), _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }
    Ok(em.finish("_run"))
}

/// Compile pre-parsed LispVal expressions to NEAR WAT
pub fn compile_near_to_wat_from_exprs(exprs: &[LispVal]) -> Result<String, String> {
    let b = compile_near_from_exprs(exprs)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}

pub fn compile_pure_to_wat(source: &str) -> Result<String, String> {
    let b = compile_pure(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}

pub fn compile_near_to_wat(source: &str) -> Result<String, String> {
    let b = compile_near(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}

// ── TypeChecker: lightweight pre-pass ──

struct TypeChecker<'a> {
    funcs: &'a Vec<FuncDef>,
    local_names: Vec<String>,
}

impl<'a> TypeChecker<'a> {
    fn check(&mut self, e: &LispVal) -> Result<Ty, String> {
        match e {
            LispVal::Num(_) => Ok(Ty::Num),
            LispVal::Bool(_) => Ok(Ty::Bool),
            LispVal::Nil => Ok(Ty::Void),
            LispVal::Str(_) => Ok(Ty::Str),
            LispVal::Sym(name) => {
                if self.local_names.contains(&name.to_string()) {
                    Ok(Ty::Any)
                } else {
                    Err(format!("Undefined variable '{}'", name))
                }
            }
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(op) = &items[0] {
                    self.check_call(op, &items[1..])
                } else {
                    Err(format!("expected symbol in call position, got {:?}", items[0]))
                }
            }
            LispVal::List(items) if items.is_empty() => Ok(Ty::Void),
            _ => Ok(Ty::Any),
        }
    }

    fn check_call(&mut self, op: &str, args: &[LispVal]) -> Result<Ty, String> {
        let numeric_binops = ["+", "-", "*", "/", "mod"];
        let comparison_ops = ["=", "!=", "<", ">", "<=", ">="];
        let bool_ops = ["and", "or", "not"];

        if numeric_binops.contains(&op) {
            if op != "+" && op != "*" && args.len() != 2 {
                return Err(format!("Type error: '{}' expects exactly 2 arguments, got {}", op, args.len()));
            }
            // Allow + and * with 2+ args (they fold), but check each arg is numeric
            for (i, arg) in args.iter().enumerate() {
                let ty = self.check(arg)?;
                match ty {
                    Ty::Str => return Err(format!("Type error: '{}' expects numeric arguments, got string at argument {}", op, i + 1)),
                    Ty::Void => return Err(format!("Type error: '{}' expects numeric arguments, got void at argument {}", op, i + 1)),
                    _ => {}
                }
            }
            return Ok(Ty::Num);
        }

        if comparison_ops.contains(&op) {
            if args.len() != 2 {
                return Err(format!("Type error: '{}' expects exactly 2 arguments, got {}", op, args.len()));
            }
            for (i, arg) in args.iter().enumerate() {
                let ty = self.check(arg)?;
                match ty {
                    Ty::Str => return Err(format!("Type error: '{}' expects numeric arguments, got string at argument {}", op, i + 1)),
                    Ty::Void => return Err(format!("Type error: '{}' expects numeric arguments, got void at argument {}", op, i + 1)),
                    _ => {}
                }
            }
            return Ok(Ty::Bool);
        }

        if bool_ops.contains(&op) {
            for arg in args {
                let ty = self.check(arg)?;
                match ty {
                    Ty::Str => return Err(format!("Type error: '{}' expects bool/numeric arguments, got string", op)),
                    _ => {}
                }
            }
            return Ok(Ty::Bool);
        }

        match op {
            "abs" => {
                if args.len() != 1 {
                    return Err(format!("Type error: 'abs' expects 1 argument, got {}", args.len()));
                }
                let ty = self.check(&args[0])?;
                match ty {
                    Ty::Str => Err("Type error: 'abs' expects a numeric argument, got string".into()),
                    _ => Ok(Ty::Num),
                }
            }
            "if" => {
                if args.len() < 2 {
                    return Err("Type error: 'if' expects at least 2 arguments (condition, then, [else])".into());
                }
                let cond_ty = self.check(&args[0])?;
                if cond_ty == Ty::Str {
                    return Err("Type error: 'if' condition must be numeric or bool, got string".into());
                }
                let then_ty = self.check(&args[1])?;
                if args.len() > 2 {
                    let else_ty = self.check(&args[2])?;
                    if then_ty != Ty::Any && else_ty != Ty::Any && then_ty != else_ty {
                        return Err(format!("Type error: 'if' branches have mismatched types: {} vs {}", then_ty, else_ty));
                    }
                }
                Ok(then_ty)
            }
            "begin" => {
                if args.is_empty() { return Ok(Ty::Void); }
                for arg in &args[..args.len()-1] { self.check(arg)?; }
                self.check(args.last().unwrap())
            }
            "let" => {
                // (let ((x val) ...) body...)
                if args.is_empty() { return Ok(Ty::Void); }
                let mut saved = self.local_names.clone();
                if let LispVal::List(bindings) = &args[0] {
                    for b in bindings {
                        if let LispVal::List(p) = b {
                            if p.len() == 2 {
                                if let LispVal::Sym(n) = &p[0] {
                                    self.local_names.push(n.clone());
                                }
                                self.check(&p[1])?;
                            }
                        }
                    }
                }
                // body
                let result = if args.len() > 1 {
                    for arg in &args[1..args.len()-1] { self.check(arg)?; }
                    self.check(args.last().unwrap())
                } else {
                    Ok(Ty::Void)
                };
                self.local_names = saved;
                result
            }
            "while" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Void)
            }
            "for" => {
                if args.len() < 4 { return Ok(Ty::Any); }
                let mut saved = self.local_names.clone();
                if let LispVal::Sym(var) = &args[0] {
                    self.local_names.push(var.clone());
                }
                self.check(&args[1])?;
                self.check(&args[2])?;
                for arg in &args[3..] { self.check(arg)?; }
                self.local_names = saved;
                Ok(Ty::Void)
            }
            "reduce" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Any)
            }
            "set!" => {
                if args.len() >= 2 { self.check(&args[1])?; }
                Ok(Ty::Void)
            }
            "quote" => Ok(Ty::Any),
            // NEAR builtins
            "near/log" | "near/log_num" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Void)
            }
            "near/return" | "near/return_str" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Void)
            }
            "near/store" | "near/storage_set" | "near/remove" | "near/storage_remove" | "near/has_key" | "near/storage_has" | "near/panic" | "near/abort" | "u128/store_storage" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Any)
            }
            "near/load" | "near/storage_get" | "near/current_account_id" | "near/signer_account_id" |
            "near/predecessor_account_id" | "near/input" | "near/block_index" |
            "near/block_timestamp" | "near/epoch_height" | "near/prepaid_gas" |
            "near/used_gas" | "near/attached_deposit" | "near/attached_deposit_high" |
            "near/account_balance" | "near/sha256" | "near/random_seed" |
            "near/promise_create" | "near/promise_then" | "near/promise_and" |
            "near/promise_results_count" | "near/promise_result" | "near/promise_return" |
            "near/promise_batch_create" | "near/promise_batch_then" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Any)
            }
            // u128 operations
            "u128/from_yocto" | "u128/new" | "u128/from_i64" | "u128/to_i64" |
            "u128/load" | "u128/store" | "u128/add" | "u128/sub" |
            "u128/eq" | "u128/is_zero" | "u128/lt" | "u128/load_storage" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Any)
            }
            // HOF macros
            "hof/map" | "hof/filter" | "hof/reduce" => {
                for arg in args { self.check(arg)?; }
                Ok(Ty::Any)
            }
            // User-defined function call
            _ => {
                // Check if function exists
                if let Some(func) = self.funcs.iter().find(|f| f.name == op) {
                    if args.len() != func.param_count {
                        return Err(format!("Type error: '{}' expects {} argument(s), got {}", op, func.param_count, args.len()));
                    }
                    for arg in args { self.check(arg)?; }
                    Ok(Ty::Any)
                } else {
                    let suggestion = self.suggest_function(op);
                    if let Some(s) = suggestion {
                        Err(format!("Error: function '{}' is not defined. Did you mean '{}'?", op, s))
                    } else {
                        Err(format!("Error: function '{}' is not defined", op))
                    }
                }
            }
        }
    }

    fn suggest_function(&self, name: &str) -> Option<String> {
        let mut best: Option<(String, usize)> = None;
        for f in self.funcs {
            // skip internal functions
            if f.name.starts_with("__") { continue; }
            let dist = levenshtein(name, &f.name);
            if dist <= 3 {
                match &best {
                    Some((_, best_dist)) if dist >= *best_dist => {}
                    _ => best = Some((f.name.clone(), dist)),
                }
            }
        }
        // Also check common builtins
        let builtins = ["+", "-", "*", "/", "mod", "abs", "=", "!=", "<", ">", "<=", ">=",
            "and", "or", "not", "if", "let", "begin", "while", "for", "set!", "quote",
            "near/log", "near/return", "near/store", "near/load", "near/storage_set", "near/storage_get", "near/storage_has", "near/storage_remove", "hof/map", "hof/filter", "hof/reduce"];
        for b in &builtins {
            let dist = levenshtein(name, b);
            if dist <= 3 {
                match &best {
                    Some((_, best_dist)) if dist >= *best_dist => {}
                    _ => best = Some((b.to_string(), dist)),
                }
            }
        }
        best.map(|(s, _)| s)
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (m, n) = (a.len(), b.len());
    if m == 0 { return n; }
    if n == 0 { return m; }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i-1] == b[j-1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j-1] + 1).min(prev[j-1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_near_counter() {
        let src = r#"
(memory 1)
(define (get_counter) (near/load "c"))
(define (set_counter val) (near/store "c" val))
(define (new) (set_counter 0))
(define (increment) (set_counter (+ (get_counter) 1)))
(define (get) (near/return (get_counter)))
(export "new" new false)
(export "increment" increment false)
(export "get" get true)
"#;
        let wasm = compile_near(src).unwrap();
        assert!(!wasm.is_empty());
        // Verify it's valid WASM by roundtripping through wasmprinter
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Counter WAT:\n{}", wat);
    }

    #[test]
    fn test_pure_fib() {
        let src = "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))";
        let wasm = compile_pure(src).unwrap();
        assert!(!wasm.is_empty());
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Fib WAT:\n{}", wat);
    }

    #[test]
    fn test_tc_count() {
        // Tail-recursive count — must use loop, not recursive call
        let src = "(define (count n) (if (= n 0) 0 (count (- n 1))))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Count WAT:\n{}", wat);
        // The body should contain a loop (TC optimization) and no call to self
        assert!(wat.contains("loop"), "TC should generate a loop");
        // Count "call 0" occurrences — should be 0 inside the function body
        // (only the wrapper calls func 0)
        let lines: Vec<&str> = wat.lines().collect();
        let func_start = lines.iter().position(|l| l.contains("(func (;0;)")).unwrap();
        let func_end = lines.iter().rposition(|l| l.contains("(func (;1;)")).unwrap();
        let func_body = &lines[func_start..func_end];
        let call_count = func_body.iter().filter(|l| l.trim().starts_with("call")).count();
        assert_eq!(call_count, 0, "TC body should not contain any call instructions, found {}", call_count);
    }

    #[test]
    fn test_tc_nested_if() {
        // TC with nested if where neither branch is directly a self-call
        let src = "(define (f n) (if (= n 0) 0 (if (= n 1) 1 (f (- n 2)))))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Nested IF WAT:\n{}", wat);
        assert!(wat.contains("loop"), "TC should generate a loop");
    }

    #[test]
    fn test_implicit_begin_let() {
        let src = "(define (f x) (let ((y 1)) (set! y 2) y))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Implicit begin let WAT:\n{}", wat);
        // Should compile without error — multiple body exprs in let
    }

    #[test]
    fn test_implicit_begin_define() {
        let src = r#"(define (f x) (set! x (+ x 1)) x)"#;
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Implicit begin define WAT:\n{}", wat);
    }
}
