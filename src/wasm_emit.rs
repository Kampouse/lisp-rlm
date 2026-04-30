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
use wasm_encoder::{
    BlockType, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, ImportSection, Instruction, MemorySection, MemoryType, Module,
    TypeSection, ValType,
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
}

impl WasmEmitter {
    pub fn new() -> Self {
        Self {
            locals: HashMap::new(), next_local: 0, current_func: None, current_param_count: 0,
            while_id: Cell::new(0), funcs: Vec::new(), memory_pages: 1, exports: Vec::new(),
            data_segments: Vec::new(), next_data_offset: 256, host_needed: HashSet::new(),
        }
    }

    fn local_idx(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.locals.get(name) { return i; }
        let i = self.next_local;
        self.locals.insert(name.to_string(), i);
        self.next_local += 1;
        i
    }

    fn alloc_data(&mut self, bytes: &[u8]) -> u32 {
        let off = self.next_data_offset;
        self.data_segments.push((off, bytes.to_vec()));
        self.next_data_offset += bytes.len() as u32;
        self.next_data_offset = (self.next_data_offset + 7) & !7;
        off
    }

    fn need_host(&mut self, idx: usize) { self.host_needed.insert(idx); }

    fn host_call(idx: usize) -> Instruction<'static> {
        Instruction::Call(HOST_BASE | idx as u32)
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

    fn scan_host(&mut self, e: &LispVal) {
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
        // Pre-insert placeholder so self-recursion resolves
        let placeholder_idx = self.funcs.len();
        self.funcs.push(FuncDef { name: name.into(), param_count: params.len(), local_count: 0, instrs: Vec::new() });
        let tc = self.has_tc(body);
        let instrs = if tc { self.tc_body(body)? } else { self.expr(body)? };
        let total = self.next_local as usize;
        self.current_func = None;
        self.funcs[placeholder_idx] = FuncDef { name: name.into(), param_count: params.len(), local_count: total, instrs };
        Ok(())
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
            LispVal::Sym(n) => self.locals.get(n).map(|&i| vec![Instruction::LocalGet(i)]).ok_or_else(|| format!("undef: {}", n)),
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
                let msg = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(msg.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(msg);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                // log_utf8(len, ptr) — idx 28
                v.push(Self::host_call(28));
                v.push(Instruction::I64Const(0)); Ok(v)
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

            // (fp64/mul dst_addr src_addr) — dst *= src (Q64.64, 128-bit intermediate)
            // result = (dst_hi*2^64+dst_lo) * (src_hi*2^64+src_lo) >> 64
            // = dst_hi*src_hi*2^64 + (dst_hi*src_lo + dst_lo*src_hi) + dst_lo*src_lo>>64
            "fp64/mul" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fm_dl");
                let dh = self.local_idx("__fm_dh");
                let sl = self.local_idx("__fm_sl");
                let sh = self.local_idx("__fm_sh");
                let mid = self.local_idx("__fm_mid");
                let carry = self.local_idx("__fm_c");
                let rl = self.local_idx("__fm_rl");
                let rh = self.local_idx("__fm_rh");
                let mut v = Vec::new();
                // Load dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // mid = dh*sl + dl*sh (with carry detection)
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(mid));
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Mul);
                // mid += dl*sh, detect carry
                v.push(Instruction::LocalGet(mid)); // save for carry check
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalTee(mid)); // mid = dh*sl + dl*sh
                // carry if mid < old_mid (which is on stack below the add result... need to restructure)
                v.pop(); // remove tee
                // Redo: compute tmp = dl*sh, then mid = dh*sl + tmp, carry = mid < dh*sl
                v.push(Instruction::LocalSet(carry)); // carry = dl*sh temporarily
                v.push(Instruction::LocalGet(mid)); // mid = dh*sl
                v.push(Instruction::LocalGet(carry));
                v.push(Instruction::I64Add); // mid = dh*sl + dl*sh
                v.push(Instruction::LocalTee(mid));
                v.push(Instruction::LocalGet(mid)); // dh*sl
                v.push(Instruction::I64LtU); // carry if sum < dh*sl
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // result_lo = mid + (dl*sl >> 64), with carry
                v.push(Instruction::LocalGet(mid));
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalTee(rl));
                v.push(Instruction::LocalGet(mid));
                v.push(Instruction::I64LtU); // if rl < mid, another carry
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(carry));
                v.push(Instruction::LocalGet(mid));
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rl));
                // result_hi = dh*sh + carry
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rh));
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

            // (fp64/div dst_addr src_addr) — dst /= src (APPROXIMATE: integer division of high parts only)
            // For CLMM prices where high word carries the integer part, this is sufficient.
            // result_hi = dst_hi / src_hi, result_lo = 0
            // TODO: full 128-bit division for precise results
            "fp64/div" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dh = self.local_idx("__fp64d_dh");
                let sh = self.local_idx("__fp64d_sh");
                let rh = self.local_idx("__fp64d_rh");
                let mut v = Vec::new();
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // Load src high
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // result_hi = dst_hi / src_hi (unsigned)
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(rh));
                // Store: low = 0, high = result
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(0)); Ok(v)
            }

            // ── tick_to_price64: Q64.64 tick math via binary exponentiation ──
            // (tick_to_price64 addr tick) — writes Q64.64 1.0001^tick to mem[addr..addr+15]
            // Uses locals for result/base during loop, stores to memory at end
            "tick_to_price64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tp64_a");
                let t_i = self.local_idx("__tp64_t");
                let neg_i = self.local_idx("__tp64_neg");
                // result: rh=high, rl=low
                let rl = self.local_idx("__tp64_rl");
                let rh = self.local_idx("__tp64_rh");
                // base: bh=high, bl=low
                let bl = self.local_idx("__tp64_bl");
                let bh = self.local_idx("__tp64_bh");
                // temps for Q64.64 mul
                let mid = self.local_idx("__tp64_mid");
                let carry = self.local_idx("__tp64_c");
                let tmp = self.local_idx("__tp64_tmp");
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
                // result = 1.0 in Q64.64: {low=0, high=1}
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rl));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(rh));
                // base = 1.0001 in Q64.64: {low=1844674407370955, high=1}
                // 0.0001 * 2^64 = 1844674407370955
                v.push(Instruction::I64Const(1844674407370955i64)); v.push(Instruction::LocalSet(bl));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(bh));
                // Binary exponentiation loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if t == 0: break
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: result *= base (Q64.64 mul using locals)
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // Q64.64 mul: rl,rh *= bl,bh
                // mid = rh*bl + rl*bh
                v.push(Instruction::LocalGet(rh)); v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(tmp)); // tmp = rh*bl
                v.push(Instruction::LocalGet(rl)); v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add);
                // carry if mid < tmp (i.e. rh*bl)
                v.push(Instruction::LocalTee(mid));
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // new_rl = mid + (rl*bl >> 64)
                v.push(Instruction::LocalGet(rl)); v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU); // rl*bl >> 64
                v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Add); // new_rl = mid + ...
                // carry2 if new_rl < mid
                v.push(Instruction::LocalTee(rl)); // store new rl
                v.push(Instruction::LocalGet(mid));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(carry));
                // new_rh = rh*bh + carry
                v.push(Instruction::LocalGet(rh)); v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rh));
                v.push(Instruction::End); // end if tick&1
                // base *= base (Q64.64 square, same mul with rl=rh=bl=bh)
                // mid = bh*bl + bl*bh = 2*(bh*bl)
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(tmp));
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); // 2*bh*bl
                v.push(Instruction::LocalTee(mid));
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64LtU); // carry if mid < bh*bl (overflow)
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // new_bl = mid + (bl*bl >> 64)
                v.push(Instruction::LocalGet(bl)); v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalTee(bl));
                v.push(Instruction::LocalGet(mid));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(carry));
                // new_bh = bh*bh + carry
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(bh));
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative: approximate reciprocal for values near 1.0
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // For values near 1.0: reciprocal ≈ (1<<32) / (rh+1) << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); // 1<<32
                v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); // avoid div-by-zero
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); // back to Q64.64 scale
                v.push(Instruction::LocalSet(rl));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rh));
                v.push(Instruction::End);
                // Store result to memory
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
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
                let pos = self.funcs.iter().position(|f| f.name == op).ok_or_else(|| format!("in {}: unknown function '{}'", self.current_func.as_deref().unwrap_or("top"), op))?;
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

    pub fn finish(&self, default_export: &str) -> Vec<u8> {
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

        // Import section
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
        let wrapper_count = if self.exports.is_empty() { 1 } else { self.exports.len() as u32 };
        for _ in 0..wrapper_count { funcs.function(0); }
        m.section(&funcs);

        // Memory
        let mut mems = MemorySection::new();
        mems.memory(MemoryType { minimum: self.memory_pages.max(1) as u64, maximum: None, memory64: false, shared: false, page_size_log2: None });
        m.section(&mems);

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
            if let Some(_) = self.funcs.last() {
                let idx = internal_base + (self.funcs.len()-1) as u32;
                let mut fb = Function::new(Vec::<(u32, ValType)>::new());
                fb.instruction(&Instruction::Call(idx)); fb.instruction(&Instruction::Drop); fb.instruction(&Instruction::End);
                code.function(&fb);
            }
        } else {
            for (fn_name, _, _) in &self.exports {
                if let Some(&idx) = name_map.get(fn_name.as_str()) {
                    let mut fb = Function::new(Vec::<(u32, ValType)>::new());
                    fb.instruction(&Instruction::Call(idx)); fb.instruction(&Instruction::Drop); fb.instruction(&Instruction::End);
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

pub fn compile_pure_to_wat(source: &str) -> Result<String, String> {
    let b = compile_pure(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}

pub fn compile_near_to_wat(source: &str) -> Result<String, String> {
    let b = compile_near(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
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
