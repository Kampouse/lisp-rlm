//! u128 Level 1 — limb locals (2026-09-15).
//!
//! A local whose EVERY store (let-init + every set!) is u128-pure — a
//! `u128/add|sub|mul|div|mod` result, an all-digits string literal, or a
//! copy of another such local — is kept as a LIMB PAIR (two raw i64 wasm
//! locals: lo, hi) instead of a tagged decimal-string heap value.
//!
//! Parse/render round-trips (≈7 Ggas of every ≈9 Ggas u128 op) vanish for
//! values that stay in limb form: loop accumulators (fib, interest
//! accrual, batch payouts) now do limb math directly through the shared
//! `__h_u128_*` helpers, serializing only at true edges (storage write,
//! return, log, strcat operand) via lazy materialization in the Sym-read
//! path.
//!
//! Semantics are preserved exactly:
//! - eligibility is conservative — any store the analysis cannot prove
//!   u128-pure demotes the local back to a tagged slot (zero divergence);
//! - every value that reaches a limb slot was produced by a u128 helper
//!   (which traps on invalid input) or is an all-digits literal, so no
//!   trap can fire earlier than the tagged path would have trapped;
//! - `u128/from-i64` stores are NOT eligible (negative payloads render
//!   "-n", which parses fine as a tagged string but would trap at the
//!   limb-parse — trap-timing divergence for a program that never uses
//!   the value in a u128 op);
//! - captured (closure) variables materialize to their tagged form at
//!   closure-creation time (lambda.rs).
//!
//! Cosmetic deviation (GAPS.md): materializing a limb local renders the
//! CANONICAL decimal form — a literal like "007" reads back as "7".

use super::*;
use crate::wasm_emit::call_u128_str::{ma8, U128_A, U128_B, U128_R};
use std::collections::{HashMap, HashSet};

/// u128-pure initializer/store forms. `sym_eligible` resolves copies of
/// other eligible locals (fixpoint done by the caller).
fn store_is_limb_pure(e: &LispVal, eligible: &HashSet<String>) -> bool {
    match e {
        // digits-only AND ≤ u128::MAX — a 40-digit literal parses fine as a
        // tagged string (strcat prints it verbatim) but would trap at the
        // limb binding, so oversized literals demote (zero divergence)
        LispVal::Str(s) => s.parse::<u128>().is_ok(),
        LispVal::Sym(n) => eligible.contains(n),
        LispVal::List(items) if !items.is_empty() => {
            let LispVal::Sym(head) = &items[0] else {
                return false;
            };
            matches!(
                head.as_str(),
                "u128/add" | "u128/sub" | "u128/mul" | "u128/div" | "u128/mod"
            ) && items.len() == 3
        }
        _ => false,
    }
}

/// Collect name → every stored expr (let-inits + set!s), walking all
/// nested forms. `(= n v)` is ambiguous with numeric equality in this
/// dialect's IR, so it is treated as a store too (over-conservative:
/// may demote, never wrongly promote).
fn collect_stores(forms: &[LispVal], out: &mut HashMap<String, Vec<LispVal>>) {
    for f in forms {
        if let LispVal::List(items) = f {
            if let Some(LispVal::Sym(head)) = items.first() {
                match head.as_str() {
                    "set!" | "=" => {
                        if items.len() >= 3 {
                            if let LispVal::Sym(n) = &items[1] {
                                out.entry(n.clone()).or_default().push(items[2].clone());
                            }
                        }
                    }
                    "let" | "let*" => {
                        if let Some(LispVal::List(bs)) = items.get(1) {
                            for b in bs {
                                if let LispVal::List(p) = b {
                                    if p.len() == 2 {
                                        if let LispVal::Sym(n) = &p[0] {
                                            out.entry(n.clone()).or_default().push(p[1].clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            collect_stores(items, out);
        }
    }
}

impl WasmEmitter {
    /// Per-function eligibility scan. A local is limb-eligible iff it has
    /// ≥1 store, every store is limb-pure (fixpoint over local copies),
    /// and it is not a parameter (params arrive tagged through the
    /// one-i64 calling convention).
    pub(crate) fn scan_limb_eligible(&self, body: &LispVal, params: &[String]) -> HashSet<String> {
        let mut stores: HashMap<String, Vec<LispVal>> = HashMap::new();
        collect_stores(std::slice::from_ref(body), &mut stores);
        let mut eligible: HashSet<String> = stores
            .keys()
            .filter(|n| !params.iter().any(|p| p == *n))
            .cloned()
            .collect();
        // fixpoint: a Sym store is pure only while its source is eligible
        loop {
            let mut changed = false;
            for n in stores.keys() {
                if params.iter().any(|p| p == n) {
                    continue;
                }
                let ok = stores[n].iter().all(|e| store_is_limb_pure(e, &eligible));
                if ok != eligible.contains(n) {
                    changed = true;
                    if ok {
                        eligible.insert(n.clone());
                    } else {
                        eligible.remove(n);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        eligible
    }

    /// Allocate a fresh limb pair for `name` (two i64 locals), saving the
    /// previous mapping (shadowing) onto `saved`.
    pub(crate) fn limb_bind(
        &mut self,
        name: &str,
        saved: &mut Vec<(String, Option<(u32, u32)>)>,
    ) -> (u32, u32) {
        let old = self.limb_slots.remove(name);
        let lo = self.free_locals.pop().unwrap_or(self.next_local);
        if lo == self.next_local {
            self.next_local += 1;
            self.local_type_map.push(ValType::I64);
        }
        let hi = self.free_locals.pop().unwrap_or(self.next_local);
        if hi == self.next_local {
            self.next_local += 1;
            self.local_type_map.push(ValType::I64);
        }
        self.limb_slots.insert(name.to_string(), (lo, hi));
        saved.push((name.to_string(), old));
        (lo, hi)
    }

    /// Restore limb mappings at scope exit (reverse order), releasing
    /// pair slots whose mapping was freshly created in this scope.
    pub(crate) fn limb_restore(&mut self, saved: Vec<(String, Option<(u32, u32)>)>) {
        for (n, old) in saved.into_iter().rev() {
            match old {
                Some(prev) => {
                    self.limb_slots.insert(n, prev);
                }
                None => {
                    if let Some((lo, hi)) = self.limb_slots.remove(&n) {
                        self.free_locals.push(lo);
                        self.free_locals.push(hi);
                    }
                }
            }
        }
    }

    /// limbs at (lo, hi) → 16 bytes at addr. Address is an i64 const.
    pub(crate) fn limb_pair_store(&self, lo: u32, hi: u32, addr: i64) -> Vec<Instruction<'static>> {
        vec![
            Instruction::I32Const(addr as i32),
            Instruction::LocalGet(lo),
            Instruction::I64Store(ma8()),
            Instruction::I32Const((addr + 8) as i32),
            Instruction::LocalGet(hi),
            Instruction::I64Store(ma8()),
        ]
    }

    /// 16 bytes at addr → limbs into locals (lo, hi).
    pub(crate) fn limb_pair_load(&self, addr: i64, lo: u32, hi: u32) -> Vec<Instruction<'static>> {
        vec![
            Instruction::I32Const(addr as i32),
            Instruction::I64Load(ma8()),
            Instruction::LocalSet(lo),
            Instruction::I32Const((addr + 8) as i32),
            Instruction::I64Load(ma8()),
            Instruction::LocalSet(hi),
        ]
    }

    /// Compile `e` into limb locals (lo, hi). The u128-pure fast shapes
    /// (limb-local copy, digit literal, direct u128 arith op) skip the
    /// tagged round-trip entirely; anything else compiles generically and
    /// parses once.
    pub(crate) fn emit_u128_val(
        &mut self,
        e: &LispVal,
        lo: u32,
        hi: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        match e {
            // direct copy from another limb local
            LispVal::Sym(n) if self.limb_slots.contains_key(n) => {
                let (slo, shi) = self.limb_slots[n];
                Ok(vec![
                    Instruction::LocalGet(slo),
                    Instruction::LocalSet(lo),
                    Instruction::LocalGet(shi),
                    Instruction::LocalSet(hi),
                ])
            }
            // digit literal — parse at COMPILE time into two i64 consts
            LispVal::Str(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                match s.parse::<u128>() {
                    Ok(v) => Ok(vec![
                        Instruction::I64Const((v as u64) as i64),
                        Instruction::LocalSet(lo),
                        Instruction::I64Const((v >> 64) as u64 as i64),
                        Instruction::LocalSet(hi),
                    ]),
                    // > u128::MAX — the tagged path would trap at parse
                    // too; keep the runtime trap (identical semantics)
                    Err(_) => self.emit_u128_val_generic(e, lo, hi),
                }
            }
            // direct u128 arith op with a LIMB exit (result stays limbs)
            LispVal::List(items)
                if matches!(
                    items.first(),
                    Some(LispVal::Sym(h)) if matches!(
                        h.as_str(),
                        "u128/add" | "u128/sub" | "u128/mul" | "u128/div" | "u128/mod"
                    )
                ) && items.len() == 3 =>
            {
                self.emit_u128_op_limb(items, lo, hi)
            }
            _ => self.emit_u128_val_generic(e, lo, hi),
        }
    }

    /// Generic fallback: tagged value on stack → parse → limbs.
    fn emit_u128_val_generic(
        &mut self,
        e: &LispVal,
        lo: u32,
        hi: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        // Level 1.5 runtime parse-cache for tagged Sym sources (see the
        // operand path — same guard + fill + invalidation contract)
        if let LispVal::Sym(n) = e {
            if self.locals.contains_key(n) && !self.captured_map.contains_key(n) {
                let key = n.clone();
                let (flag, clo, chi) = self.parse_cache_alloc(&key);
                let h = self.ensure_u128_str_helpers();
                let mut v = Vec::new();
                v.push(Instruction::LocalGet(flag));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(clo));
                v.push(Instruction::LocalSet(lo));
                v.push(Instruction::LocalGet(chi));
                v.push(Instruction::LocalSet(hi));
                v.push(Instruction::Else);
                let tv = self.expr(e)?;
                let t = self.local_idx(&format!("__u128lv_{}", self.limb_call_count));
                self.limb_call_count += 1;
                v.extend(tv);
                v.push(Instruction::LocalSet(t));
                self.u128_parse_call(&mut v, t, U128_A, &h);
                v.extend(self.limb_pair_load(U128_A, lo, hi));
                v.extend(self.limb_pair_load(U128_A, clo, chi));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalSet(flag));
                v.push(Instruction::End);
                return Ok(v);
            }
        }
        let h = self.ensure_u128_str_helpers();
        let tv = self.expr(e)?;
        let t = self.local_idx(&format!("__u128lv_{}", self.limb_call_count));
        self.limb_call_count += 1;
        let mut v = tv;
        v.push(Instruction::LocalSet(t));
        // parse (tagged → U128_A), then load the pair into (lo, hi)
        self.u128_parse_call(&mut v, t, U128_A, &h);
        v.extend(self.limb_pair_load(U128_A, lo, hi));
        Ok(v)
    }

    /// Allocate (or fetch) the runtime memo triple for a cached name:
    /// (flag, lo, hi) — flag defaults to 0 (invalid) in a fresh local.
    fn parse_cache_alloc(&mut self, n: &str) -> (u32, u32, u32) {
        if let Some(&t) = self.parse_cache.get(n) {
            return t;
        }
        let flag = self.local_idx(&format!("__pc_f_{}", n));
        let clo = self.local_idx(&format!("__pc_lo_{}", n));
        let chi = self.local_idx(&format!("__pc_hi_{}", n));
        let t = (flag, clo, chi);
        self.parse_cache.insert(n.to_string(), t);
        t
    }

    /// Emit `flag = 0` for the name's memo (if one exists — lazily
    /// allocated at first use). Stack-neutral; safe to splice anywhere.
    pub(crate) fn emit_parse_cache_invalidate(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        n: &str,
    ) {
        if let Some(&(flag, _, _)) = self.parse_cache.get(n) {
            v.push(Instruction::I64Const(0));
            v.push(Instruction::LocalSet(flag));
        }
    }

    /// (u128/op x y) with the result left in limb locals (lo, hi) instead
    /// of a tagged string. Operands go through the limb-aware operand
    /// path (limb locals and nested u128 arith never stringify).
    fn emit_u128_op_limb(
        &mut self,
        items: &[LispVal],
        lo: u32,
        hi: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let op = match &items[0] {
            LispVal::Sym(h) => h.as_str(),
            _ => unreachable!(),
        };
        let h = self.ensure_u128_str_helpers();
        let gen = self.limb_call_count;
        self.limb_call_count += 1;
        let alo = self.local_idx(&format!("__u128la_{gen}"));
        let ahi = self.local_idx(&format!("__u128ha_{gen}"));
        let blo = self.local_idx(&format!("__u128lb_{gen}"));
        let bhi = self.local_idx(&format!("__u128hb_{gen}"));
        let mut v = Vec::new();
        self.emit_u128_operand(&mut v, &items[1], alo, ahi)?;
        self.emit_u128_operand(&mut v, &items[2], blo, bhi)?;
        v.extend(self.limb_pair_store(alo, ahi, U128_A));
        v.extend(self.limb_pair_store(blo, bhi, U128_B));
        let is_divmod = op == "u128/div" || op == "u128/mod";
        let (hf, hf_ck, r_src) = match op {
            "u128/add" => (h.add, h.add_ck, U128_A),
            "u128/sub" => (h.sub, h.sub_ck, U128_A),
            "u128/mul" => (h.mul, h.mul_ck, U128_A),
            _ => (
                h.divmod,
                h.divmod_ck,
                if op == "u128/div" { U128_A } else { U128_R },
            ),
        };
        v.push(Instruction::I64Const(U128_A));
        v.push(Instruction::I64Const(U128_B));
        if is_divmod {
            v.push(Instruction::I64Const(U128_R));
        }
        if self.try_stack.is_empty() {
            v.push(Self::call_user(hf));
            v.push(Instruction::Drop);
        } else {
            let msg = if is_divmod {
                "u128: division by zero"
            } else {
                "u128: overflow/underflow"
            };
            let call = Self::call_user(hf_ck);
            self.ck_guarded(&mut v, call, msg);
        }
        v.extend(self.limb_pair_load(r_src, lo, hi));
        Ok(v)
    }

    /// Evaluate one u128 op operand into limb locals (lo, hi):
    /// limb-local reads and nested u128 arith stay in limb form; anything
    /// else compiles tagged and parses once. Operands always land in
    /// FRESH per-gen locals (the 2026-08-31 nested-clobber rule).
    pub(crate) fn emit_u128_operand(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        e: &LispVal,
        lo: u32,
        hi: u32,
    ) -> Result<(), String> {
        match e {
            LispVal::Sym(n) if self.limb_slots.contains_key(n) => {
                let (slo, shi) = self.limb_slots[n];
                v.push(Instruction::LocalGet(slo));
                v.push(Instruction::LocalSet(lo));
                v.push(Instruction::LocalGet(shi));
                v.push(Instruction::LocalSet(hi));
                Ok(())
            }
            LispVal::List(items)
                if matches!(
                    items.first(),
                    Some(LispVal::Sym(h)) if matches!(
                        h.as_str(),
                        "u128/add" | "u128/sub" | "u128/mul" | "u128/div" | "u128/mod"
                    )
                ) && items.len() == 3 =>
            {
                let inner = self.emit_u128_op_limb(items, lo, hi)?;
                v.extend(inner);
                Ok(())
            }
            LispVal::Str(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                // operand digit literal — compile-time constant limbs.
                // Oversized (> u128::MAX) falls to the generic path so the
                // RUNTIME parse trap fires exactly as the tagged path did.
                match s.parse::<u128>() {
                    Ok(val) => {
                        v.push(Instruction::I64Const((val as u64) as i64));
                        v.push(Instruction::LocalSet(lo));
                        v.push(Instruction::I64Const((val >> 64) as u64 as i64));
                        v.push(Instruction::LocalSet(hi));
                        Ok(())
                    }
                    Err(_) => {
                        let h = self.ensure_u128_str_helpers();
                        let tv = self.expr(e)?;
                        let t = self.local_idx(&format!("__u128lt_{}", self.limb_call_count));
                        self.limb_call_count += 1;
                        v.extend(tv);
                        v.push(Instruction::LocalSet(t));
                        self.u128_parse_call(v, t, U128_A, &h);
                        v.extend(self.limb_pair_load(U128_A, lo, hi));
                        Ok(())
                    }
                }
            }
            _ => {
                // tagged path: expr → temp local → parse → limb pair.
                // Level 1.5 runtime parse-cache: a plain tagged Sym operand
                // emits `if flag { copy } else { parse+fill+flag }` — a
                // loop-invariant string (bigint param, storage-read local)
                // parses ONCE at runtime instead of ~140 Mgas/use. Binding
                // events emit flag=0 (see emit_parse_cache_invalidate).
                let is_cacheable_sym = matches!(e, LispVal::Sym(n)
                    if self.locals.contains_key(n) && !self.captured_map.contains_key(n));
                if is_cacheable_sym {
                    let n = match e {
                        LispVal::Sym(n) => n.clone(),
                        _ => unreachable!(),
                    };
                    let (flag, clo, chi) = self.parse_cache_alloc(&n);
                    // hit: copy cached limbs (flag is i64 — wrap to i32 for the if)
                    v.push(Instruction::LocalGet(flag));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(clo));
                    v.push(Instruction::LocalSet(lo));
                    v.push(Instruction::LocalGet(chi));
                    v.push(Instruction::LocalSet(hi));
                    v.push(Instruction::Else);
                    // miss: parse once, fill operand slots AND the cache
                    let h = self.ensure_u128_str_helpers();
                    let tv = self.expr(e)?;
                    let t = self.local_idx(&format!("__u128lt_{}", self.limb_call_count));
                    self.limb_call_count += 1;
                    v.extend(tv);
                    v.push(Instruction::LocalSet(t));
                    self.u128_parse_call(v, t, U128_A, &h);
                    v.extend(self.limb_pair_load(U128_A, lo, hi));
                    v.extend(self.limb_pair_load(U128_A, clo, chi));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalSet(flag));
                    v.push(Instruction::End);
                    return Ok(());
                }
                let h = self.ensure_u128_str_helpers();
                let tv = self.expr(e)?;
                let t = self.local_idx(&format!("__u128lt_{}", self.limb_call_count));
                self.limb_call_count += 1;
                v.extend(tv);
                v.push(Instruction::LocalSet(t));
                self.u128_parse_call(v, t, U128_A, &h);
                v.extend(self.limb_pair_load(U128_A, lo, hi));
                Ok(())
            }
        }
    }

    /// Materialize a limb local as a tagged decimal string (Sym read in a
    /// generic context). Stores the pair to U128_A (transient — the
    /// operand-save discipline guarantees no live op state there) and
    /// renders via the chunked to_str helper.
    pub(crate) fn emit_limb_materialize(
        &mut self,
        lo: u32,
        hi: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let h = self.ensure_u128_str_helpers();
        let mut v = self.limb_pair_store(lo, hi, U128_A);
        v.push(Instruction::I64Const(U128_A));
        v.push(Self::call_user(h.to_str));
        Ok(v)
    }
}
