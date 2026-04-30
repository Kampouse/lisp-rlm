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
            "near/abort" => self.need_host(26),
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

            // User function call
            _ => {
                let pos = self.funcs.iter().position(|f| f.name == op).ok_or_else(|| format!("unknown function: {}", op))?;
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
