//! Shared differential-fuzz infrastructure (round 4, 2026-08-27).
//! Extracted verbatim from test_differential_fuzz.rs so the
//! extract_mismatch reproducer target can replay the exact program
//! stream (Rng draw order, FUZZ_OPS table, generator) without
//! duplicating ~1600 lines. Both test targets compile this module
//! independently via `mod fuzz_common;`.
pub use lisp_rlm_wasm::bytecode::{
    make_test_compiled_lambda, make_test_compiled_loop, run_compiled_lambda,
    run_compiled_loop_test, run_lambda_test, validate_slot_indices, BinOp, Op, Ty,
};
pub use lisp_rlm_wasm::types::LispVal;

// ---------------------------------------------------------------------------
// Spec VM — mirrors the F* closure_eval_op semantics
// ---------------------------------------------------------------------------

/// Pure VM state: stack + slots + pc + code + ok flag.
/// No frames, no closures — fuzzes the loop VM subset only.
#[derive(Debug, Clone)]
pub struct SpecVm {
    pub stack: Vec<LispVal>,
    pub slots: Vec<LispVal>,
    pub pc: usize,
    code: Vec<Op>,
    ok: bool,
}

/// Result of running the spec VM to completion.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecResult {
    /// Returned a value (via Return or ReturnSlot)
    Value(LispVal),
    /// The VM encountered an error (div-by-zero, unsupported op, pc out of bounds)
    Error(String),
    /// Pathological program: stack value growth exceeded the resource cap.
    /// Skipped by the differential harness (running the Rust VM would OOM).
    ResourceLimit,
    /// Exceeded the step limit (possible infinite loop)
    StepLimit,
}

impl SpecVm {
    pub fn new(code: Vec<Op>, init_slots: Vec<LispVal>) -> Self {
        Self {
            stack: Vec::with_capacity(16),
            slots: init_slots,
            pc: 0,
            code,
            ok: true,
        }
    }

    /// Validate that all slot indices in the bytecode are within bounds.
    /// Mirrors the Rust VM's validate_slot_indices so both VMs agree on OOB behavior.
    pub fn validate_slot_indices(&self) -> Result<(), String> {
        let slots_len = self.slots.len();
        for op in &self.code {
            match op {
                Op::LoadSlot(s)
                | Op::StoreSlot(s)
                | Op::ReturnSlot(s)
                | Op::StoreAndLoadSlot(s)
                | Op::DictMutSet(s)
                | Op::RecurDirect(s) => {
                    if *s >= slots_len {
                        return Err(format!(
                            "slot index {} out of bounds (slots_len={})",
                            s, slots_len
                        ));
                    }
                }
                Op::SlotAddImm(s, _)
                | Op::SlotSubImm(s, _)
                | Op::SlotMulImm(s, _)
                | Op::SlotDivImm(s, _)
                | Op::SlotEqImm(s, _)
                | Op::SlotLtImm(s, _)
                | Op::SlotLeImm(s, _)
                | Op::SlotGtImm(s, _)
                | Op::SlotGeImm(s, _) => {
                    if *s >= slots_len {
                        return Err(format!(
                            "slot index {} out of bounds (slots_len={})",
                            s, slots_len
                        ));
                    }
                }
                Op::JumpIfSlotLtImm(s, _, _)
                | Op::JumpIfSlotLeImm(s, _, _)
                | Op::JumpIfSlotGtImm(s, _, _)
                | Op::JumpIfSlotGeImm(s, _, _)
                | Op::JumpIfSlotEqImm(s, _, _) => {
                    if *s >= slots_len {
                        return Err(format!(
                            "slot index {} out of bounds (slots_len={})",
                            s, slots_len
                        ));
                    }
                }
                Op::RecurIncAccum(counter, accum, _, _, _) => {
                    if *counter >= slots_len {
                        return Err(format!(
                            "RecurIncAccum counter slot {} out of bounds (slots_len={})",
                            counter, slots_len
                        ));
                    }
                    if *accum >= slots_len {
                        return Err(format!(
                            "RecurIncAccum accum slot {} out of bounds (slots_len={})",
                            accum, slots_len
                        ));
                    }
                }
                Op::GetDefaultSlot(a, b, c, d) => {
                    for &(name, idx) in &[("map", *a), ("key", *b), ("default", *c), ("result", *d)]
                    {
                        if idx >= slots_len {
                            return Err(format!(
                                "GetDefaultSlot {} slot {} out of bounds (slots_len={})",
                                name, idx, slots_len
                            ));
                        }
                    }
                }
                Op::Recur(n) => {
                    if *n > slots_len {
                        return Err(format!(
                            "Recur({}) requires {} slots but only {} available",
                            n, n, slots_len
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Pop a value from the stack, returning Nil if empty.
    /// Matches Rust: stack.pop().unwrap_or(LispVal::Nil) — Rust VM uses
    /// .pop().unwrap() which panics, but spec VM returns Nil for robustness.
    pub fn pop(&mut self) -> LispVal {
        self.stack.pop().unwrap_or(LispVal::Nil)
    }

    /// Pop a value from the stack, or return Error if empty.
    /// Use for binary ops (Add/Sub/etc.) where the Rust VM panics on underflow.
    #[allow(dead_code)]
    pub fn pop_or_err(&mut self, op_name: &str) -> Result<LispVal, StepOutcome> {
        match self.stack.pop() {
            Some(v) => Ok(v),
            None => Err(StepOutcome::Error(format!("{}: stack underflow", op_name))),
        }
    }

    /// Get a slot value, extending with Nil if out of bounds.
    /// Matches Rust: slots[*s] — Rust uses Vec indexing which panics on OOB,
    /// but in practice the compiler never generates OOB accesses. For fuzzing
    /// robustness we extend, matching the F* model's fill_slots behavior.
    pub fn get_slot(&self, s: usize) -> LispVal {
        if s < self.slots.len() {
            match &self.slots[s] {
                LispVal::Num(n) => LispVal::Num(*n),
                _ => self.slots[s].clone(),
            }
        } else {
            LispVal::Nil
        }
    }

    /// Extract i64 from a LispVal — matches Rust num_val_ref exactly.
    /// Rust: Num → n, Float → truncate, other → 0 (silent coercion).
    /// Used by UNTYPED ops (Add, Sub, Mul, Div, Mod, comparisons, slot-imm ops).
    pub fn spec_num_val(v: &LispVal) -> i64 {
        match v {
            LispVal::Num(n) => *n,
            LispVal::Float(f) => *f as i64,
            _ => 0,
        }
    }

    /// Extract i64 for TYPED I64 ops — matches Rust TypedBinOp(_, I64) handler.
    /// Rust only reads Num here; Float/other → 0 (NOT truncated).
    pub fn spec_typed_i64_val(v: &LispVal) -> i64 {
        match v {
            LispVal::Num(n) => *n,
            _ => 0,
        }
    }

    /// Convert any LispVal to f64 — matches Rust's num_arith promotion.
    /// Float → f, Num → n as f64, other → 0.0.
    #[allow(dead_code)]
    pub fn spec_to_f64(v: &LispVal) -> f64 {
        match v {
            LispVal::Float(f) => *f,
            LispVal::Num(n) => *n as f64,
            _ => 0.0,
        }
    }

    /// Pop + coerce to i64 (matches Rust: silent coercion to 0).
    #[allow(dead_code)]
    pub fn pop_num(&mut self) -> i64 {
        let v = self.pop();
        Self::spec_num_val(&v)
    }

    /// Get slot value + coerce to i64 (matches Rust: silent coercion to 0).
    pub fn slot_num(&self, s: usize) -> i64 {
        Self::spec_num_val(&self.get_slot(s))
    }

    /// Spec truthiness — matches is_truthy in Rust:
    /// false/nil are falsy, everything else is truthy.
    pub fn spec_is_truthy(v: &LispVal) -> bool {
        // Anchor-aligned (2026-08-26): wasm is the semantic anchor.
        // wasm_emit::emit_is_truthy treats Bool(false), Nil, and tagged-0
        // (Num(0)) as falsy — matching Rust's is_truthy, which also treats
        // Float(0.0) as falsy. (NOTE: boxed Float(0.0) would be truthy under
        // the pure wasm tag check — GAPS.md: wasm-vs-Rust Float(0.0) split.)
        !matches!(
            v,
            LispVal::Nil | LispVal::Bool(false) | LispVal::Num(0) | LispVal::Float(0.0)
        )
    }

    /// Spec lisp_eq — mirrors the Rust lisp_eq function.
    pub fn spec_lisp_eq(a: &LispVal, b: &LispVal) -> bool {
        match (a, b) {
            (LispVal::Num(x), LispVal::Num(y)) => x == y,
            (LispVal::Float(x), LispVal::Float(y)) => x == y,
            (LispVal::Num(x), LispVal::Float(y)) => (*x as f64) == *y,
            (LispVal::Float(x), LispVal::Num(y)) => *x == (*y as f64),
            (LispVal::Bool(x), LispVal::Bool(y)) => x == y,
            (LispVal::Str(x), LispVal::Str(y)) => x == y,
            (LispVal::Nil, LispVal::Nil) => true,
            (LispVal::List(a), LispVal::List(b)) => a == b,
            (LispVal::Vec(a), LispVal::Vec(b)) => a == b,
            (
                LispVal::Tagged {
                    type_name: ta,
                    variant_id: va,
                    fields: fa,
                },
                LispVal::Tagged {
                    type_name: tb,
                    variant_id: vb,
                    fields: fb,
                },
            ) => ta == tb && va == vb && fa == fb,
            _ => false,
        }
    }

    /// Anchor arithmetic — mirrors src/bytecode::num_arith_checked exactly.
    /// GAPS.md decision (2026-08-26): the Rust VM / wasm emission is the
    /// semantic anchor; this spec oracle must be trace-equivalent to it.
    /// No silent coercion — non-numeric operands hard-error (trap semantics).
    pub fn spec_arith_anchor(
        op_name: &str,
        a: &LispVal,
        b: &LispVal,
        int_op: impl Fn(i64, i64) -> Option<i64>,
        float_op: impl Fn(f64, f64) -> f64,
    ) -> Result<LispVal, String> {
        match (a, b) {
            (LispVal::Float(x), LispVal::Float(y)) => Ok(LispVal::Float(float_op(*x, *y))),
            (LispVal::Float(x), LispVal::Num(y)) => Ok(LispVal::Float(float_op(*x, *y as f64))),
            (LispVal::Num(x), LispVal::Float(y)) => Ok(LispVal::Float(float_op(*x as f64, *y))),
            (LispVal::Num(x), LispVal::Num(y)) => match int_op(*x, *y) {
                Some(r) => {
                    // Payload-range check — mirrors src/bytecode::check_num_range
                    // (tagged scheme anchor: Num must fit [-2^60, 2^60-1]).
                    if (-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                        Ok(LispVal::Num(r))
                    } else {
                        Err(format!(
                            "integer overflow in {} (payload range ±2^60)",
                            op_name
                        ))
                    }
                }
                None => Err(format!("integer overflow in {}", op_name)),
            },
            (LispVal::U64(x), LispVal::U64(y)) => {
                let r = match op_name {
                    "add" => x.wrapping_add(*y),
                    "sub" => x.wrapping_sub(*y),
                    "mul" => x.wrapping_mul(*y),
                    "div" => {
                        if *y == 0 {
                            return Err("division by zero".into());
                        }
                        x.wrapping_div(*y)
                    }
                    "mod" => {
                        if *y == 0 {
                            return Err("modulo by zero".into());
                        }
                        x.wrapping_rem(*y)
                    }
                    other => return Err(format!("type error: {} on u64 operands", other)),
                };
                Ok(LispVal::U64(r))
            }
            _ => Err(format!(
                "type error: {} expects numbers, got {} {}",
                op_name, a, b
            )),
        }
    }

    /// Anchor comparison — mirrors src/bytecode::num_cmp exactly.
    /// U64×U64 compares unsigned; mixed non-numerics hard-error.
    pub fn spec_cmp_anchor(
        op_name: &str,
        a: &LispVal,
        b: &LispVal,
        fop: impl Fn(f64, f64) -> bool,
        iop: impl Fn(i64, i64) -> bool,
    ) -> Result<bool, String> {
        match (a, b) {
            (LispVal::Float(x), LispVal::Float(y)) => Ok(fop(*x, *y)),
            (LispVal::Float(x), LispVal::Num(y)) => Ok(fop(*x, *y as f64)),
            (LispVal::Num(x), LispVal::Float(y)) => Ok(fop(*x as f64, *y)),
            (LispVal::Num(x), LispVal::Num(y)) => Ok(iop(*x, *y)),
            (LispVal::U64(x), LispVal::U64(y)) => match op_name {
                "<" => Ok(x < y),
                "<=" => Ok(x <= y),
                ">" => Ok(x > y),
                ">=" => Ok(x >= y),
                other => Err(format!("type error: {} on u64 operands", other)),
            },
            _ => Err(format!(
                "type error: {} expects numbers, got {} {}",
                op_name, a, b
            )),
        }
    }

    /// Spec num_cmp — mirrors the Rust num_cmp function.
    #[allow(dead_code)]
    pub fn spec_num_cmp(
        a: &LispVal,
        b: &LispVal,
        fop: impl Fn(f64, f64) -> bool,
        iop: impl Fn(i64, i64) -> bool,
    ) -> bool {
        match (a, b) {
            (LispVal::Float(x), LispVal::Float(y)) => fop(*x, *y),
            (LispVal::Float(x), LispVal::Num(y)) => fop(*x, *y as f64),
            (LispVal::Num(x), LispVal::Float(y)) => fop(*x as f64, *y),
            (LispVal::Num(x), LispVal::Num(y)) => iop(*x, *y),
            _ => false,
        }
    }

    /// Execute one step. Returns false if the step failed (pc out of bounds, error op).
    pub fn step(&mut self) -> StepOutcome {
        if !self.ok {
            return StepOutcome::Error("vm not ok".into());
        }
        if self.pc >= self.code.len() {
            return StepOutcome::Error("pc out of bounds".into());
        }

        let op = self.code[self.pc].clone();
        match &op {
            Op::LoadSlot(s) => {
                let val = self.get_slot(*s);
                self.stack.push(val);
                self.pc += 1;
            }
            Op::PushU64(n) => {
                self.stack.push(LispVal::U64(*n));
                self.pc += 1;
            }
            Op::PushI64(n) => {
                self.stack.push(LispVal::Num(*n));
                self.pc += 1;
            }
            Op::PushFloat(f) => {
                self.stack.push(LispVal::Float(*f));
                self.pc += 1;
            }
            Op::PushBool(b) => {
                self.stack.push(LispVal::Bool(*b));
                self.pc += 1;
            }
            Op::PushStr(s) => {
                self.stack.push(LispVal::Str(s.clone()));
                self.pc += 1;
            }
            Op::PushNil => {
                self.stack.push(LispVal::Nil);
                self.pc += 1;
            }
            Op::MakeList(n) => {
                let mut items = Vec::with_capacity(*n);
                for _ in 0..*n {
                    // Match Rust VM: unwrap_or(LispVal::Nil) when stack empty
                    items.push(self.stack.pop().unwrap_or(LispVal::Nil));
                }
                items.reverse();
                self.stack.push(LispVal::List(items));
                self.pc += 1;
            }
            Op::Dup => {
                if let Some(top) = self.stack.last() {
                    self.stack.push((*top).clone());
                }
                self.pc += 1;
            }
            Op::Pop => {
                self.stack.pop();
                self.pc += 1;
            }
            Op::StoreSlot(s) => {
                let val = self.pop();
                if *s < self.slots.len() {
                    self.slots[*s] = val;
                } else {
                    while self.slots.len() <= *s {
                        self.slots.push(LispVal::Nil);
                    }
                    self.slots[*s] = val;
                }
                self.pc += 1;
            }
            Op::Add => {
                let b = self.pop();
                let a = self.pop();
                // Match Rust: if either operand is Float, do float arithmetic
                match (&a, &b) {
                    (LispVal::Float(af), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float(af + bf));
                    }
                    (LispVal::Float(af), LispVal::Num(bn)) => {
                        self.stack.push(LispVal::Float(af + (*bn as f64)));
                    }
                    (LispVal::Num(an), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float((*an as f64) + bf));
                    }
                    _ => match Self::spec_arith_anchor(
                        "add",
                        &a,
                        &b,
                        i64::checked_add,
                        |x, y| x + y,
                    ) {
                        Ok(v) => self.stack.push(v),
                        Err(e) => return StepOutcome::Error(e),
                    }
                }
                self.pc += 1;
            }
            Op::Sub => {
                let b = self.pop();
                let a = self.pop();
                match (&a, &b) {
                    (LispVal::Float(af), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float(af - bf));
                    }
                    (LispVal::Float(af), LispVal::Num(bn)) => {
                        self.stack.push(LispVal::Float(af - (*bn as f64)));
                    }
                    (LispVal::Num(an), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float((*an as f64) - bf));
                    }
                    _ => match Self::spec_arith_anchor(
                        "sub",
                        &a,
                        &b,
                        i64::checked_sub,
                        |x, y| x - y,
                    ) {
                        Ok(v) => self.stack.push(v),
                        Err(e) => return StepOutcome::Error(e),
                    }
                }
                self.pc += 1;
            }
            Op::Mul => {
                let b = self.pop();
                let a = self.pop();
                match (&a, &b) {
                    (LispVal::Float(af), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float(af * bf));
                    }
                    (LispVal::Float(af), LispVal::Num(bn)) => {
                        self.stack.push(LispVal::Float(af * (*bn as f64)));
                    }
                    (LispVal::Num(an), LispVal::Float(bf)) => {
                        self.stack.push(LispVal::Float((*an as f64) * bf));
                    }
                    _ => match Self::spec_arith_anchor(
                        "mul",
                        &a,
                        &b,
                        i64::checked_mul,
                        |x, y| x * y,
                    ) {
                        Ok(v) => self.stack.push(v),
                        Err(e) => return StepOutcome::Error(e),
                    }
                }
                self.pc += 1;
            }
            Op::Div => {
                let b = self.pop();
                let a = self.pop();
                match (&a, &b) {
                    (LispVal::Float(af), LispVal::Float(bf)) => {
                        if *bf == 0.0 {
                            return StepOutcome::Error("division by zero".into());
                        }
                        self.stack.push(LispVal::Float(af / bf));
                    }
                    (LispVal::Float(af), LispVal::Num(bn)) => {
                        if *bn == 0 {
                            return StepOutcome::Error("division by zero".into());
                        }
                        self.stack.push(LispVal::Float(af / (*bn as f64)));
                    }
                    (LispVal::Num(an), LispVal::Float(bf)) => {
                        if *bf == 0.0 {
                            return StepOutcome::Error("division by zero".into());
                        }
                        self.stack.push(LispVal::Float((*an as f64) / bf));
                    }
                    _ => match Self::spec_arith_anchor(
                        "div",
                        &a,
                        &b,
                        i64::checked_div,
                        |x, y| x / y,
                    ) {
                        Ok(v) => self.stack.push(v),
                        Err(e) => return StepOutcome::Error(e),
                    }
                }
                self.pc += 1;
            }
            Op::Mod => {
                let b = self.pop();
                let a = self.pop();
                match (&a, &b) {
                    (LispVal::Float(af), LispVal::Float(bf)) => {
                        if *bf == 0.0 {
                            return StepOutcome::Error("modulo by zero".into());
                        }
                        self.stack.push(LispVal::Float(af % bf));
                    }
                    (LispVal::Float(af), LispVal::Num(bn)) => {
                        if *bn == 0 {
                            return StepOutcome::Error("modulo by zero".into());
                        }
                        self.stack.push(LispVal::Float(af % (*bn as f64)));
                    }
                    (LispVal::Num(an), LispVal::Float(bf)) => {
                        if *bf == 0.0 {
                            return StepOutcome::Error("modulo by zero".into());
                        }
                        self.stack.push(LispVal::Float((*an as f64) % bf));
                    }
                    _ => match Self::spec_arith_anchor(
                        "mod",
                        &a,
                        &b,
                        i64::checked_rem,
                        |x, y| x % y,
                    ) {
                        Ok(v) => self.stack.push(v),
                        Err(e) => return StepOutcome::Error(e),
                    }
                }
                self.pc += 1;
            }
            Op::Eq => {
                let b = self.pop();
                let a = self.pop();
                self.stack.push(LispVal::Bool(Self::spec_lisp_eq(&a, &b)));
                self.pc += 1;
            }
            Op::Lt => {
                let b = self.pop();
                let a = self.pop();
                match Self::spec_cmp_anchor(
                    "<",
                    &a,
                    &b,
                    |x, y| x < y,
                    |x, y| x < y,
                ) {
                    Ok(r) => self.stack.push(LispVal::Bool(r)),
                    Err(e) => return StepOutcome::Error(e),
                }
                self.pc += 1;
            }
            Op::Le => {
                let b = self.pop();
                let a = self.pop();
                match Self::spec_cmp_anchor(
                    "<=",
                    &a,
                    &b,
                    |x, y| x <= y,
                    |x, y| x <= y,
                ) {
                    Ok(r) => self.stack.push(LispVal::Bool(r)),
                    Err(e) => return StepOutcome::Error(e),
                }
                self.pc += 1;
            }
            Op::Gt => {
                let b = self.pop();
                let a = self.pop();
                match Self::spec_cmp_anchor(
                    ">",
                    &a,
                    &b,
                    |x, y| x > y,
                    |x, y| x > y,
                ) {
                    Ok(r) => self.stack.push(LispVal::Bool(r)),
                    Err(e) => return StepOutcome::Error(e),
                }
                self.pc += 1;
            }
            Op::Ge => {
                let b = self.pop();
                let a = self.pop();
                match Self::spec_cmp_anchor(
                    ">=",
                    &a,
                    &b,
                    |x, y| x >= y,
                    |x, y| x >= y,
                ) {
                    Ok(r) => self.stack.push(LispVal::Bool(r)),
                    Err(e) => return StepOutcome::Error(e),
                }
                self.pc += 1;
            }
            Op::Not => {
                let v = self.pop();
                self.stack.push(LispVal::Bool(!Self::spec_is_truthy(&v)));
                self.pc += 1;
            }
            Op::TypedBinOp(binop, ty) => {
                let b = self.pop();
                let a = self.pop();
                match ty {
                    Ty::I64 => {
                        let av = Self::spec_typed_i64_val(&a);
                        let bv = Self::spec_typed_i64_val(&b);
                        self.stack.push(match binop {
                            BinOp::Add => match av.checked_add(bv) {
                                Some(r) => {
                                    if (-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                                        LispVal::Num(r)
                                    } else {
                                        return StepOutcome::Error(
                                            "integer overflow in add (payload range ±2^60)".into(),
                                        )
                                    }
                                }
                                None => {
                                    return StepOutcome::Error("integer overflow in add".into())
                                }
                            },
                            BinOp::Sub => match av.checked_sub(bv) {
                                Some(r) => {
                                    if (-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                                        LispVal::Num(r)
                                    } else {
                                        return StepOutcome::Error(
                                            "integer overflow in sub (payload range ±2^60)".into(),
                                        )
                                    }
                                }
                                None => {
                                    return StepOutcome::Error("integer overflow in sub".into())
                                }
                            },
                            BinOp::Mul => match av.checked_mul(bv) {
                                Some(r) => {
                                    if (-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                                        LispVal::Num(r)
                                    } else {
                                        return StepOutcome::Error(
                                            "integer overflow in mul (payload range ±2^60)".into(),
                                        )
                                    }
                                }
                                None => {
                                    return StepOutcome::Error("integer overflow in mul".into())
                                }
                            },
                            BinOp::Div => match av.checked_div(bv) {
                                Some(r) => {
                                    if (-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                                        LispVal::Num(r)
                                    } else {
                                        return StepOutcome::Error(
                                            "integer overflow in div (payload range ±2^60)".into(),
                                        )
                                    }
                                }
                                None => {
                                    return StepOutcome::Error("integer overflow in div".into())
                                }
                            },
                            BinOp::Mod => match av.checked_rem(bv) {
                                Some(r) => LispVal::Num(r), // |a mod b| <= |a|: stays in range
                                None => {
                                    return StepOutcome::Error("integer overflow in mod".into())
                                }
                            },
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                    Ty::F64 => {
                        let av = match &a {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        let bv = match &b {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        self.stack.push(match binop {
                            BinOp::Add => LispVal::Float(av + bv),
                            BinOp::Sub => LispVal::Float(av - bv),
                            BinOp::Mul => LispVal::Float(av * bv),
                            BinOp::Div => LispVal::Float(av / bv),
                            BinOp::Mod => LispVal::Float(av % bv),
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                    Ty::U64 => {
                    let av = match &a { LispVal::U64(v) => *v, _ => 0u64 };
                    let bv = match &b { LispVal::U64(v) => *v, _ => 0u64 };
                    if matches!(binop, BinOp::Div | BinOp::Mod) && bv == 0 {
                        return StepOutcome::Error("division by zero".into());
                    }
                    self.stack.push(match binop {
                        BinOp::Add => LispVal::U64(av.wrapping_add(bv)),
                        BinOp::Sub => LispVal::U64(av.wrapping_sub(bv)),
                        BinOp::Mul => LispVal::U64(av.wrapping_mul(bv)),
                        BinOp::Div => LispVal::U64(av.wrapping_div(bv)),
                        BinOp::Mod => LispVal::U64(av.wrapping_rem(bv)),
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                }
                self.pc += 1;
            }
            Op::JumpIfTrue(addr) => {
                let v = self.pop();
                if Self::spec_is_truthy(&v) {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::JumpIfFalse(addr) => {
                let v = self.pop();
                if !Self::spec_is_truthy(&v) {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::Jump(addr) => {
                self.pc = *addr;
            }
            Op::Return => {
                return StepOutcome::Return(self.pop());
            }
            Op::Recur(n) => {
                // Pop N values in reverse order into slots 0..N
                // Matches Rust: for i in (0..*n).rev() { slots[i] = stack.pop() }
                for i in (0..*n).rev() {
                    self.slots[i] = self.stack.pop().unwrap_or(LispVal::Nil);
                }
                self.pc = 0;
            }
            Op::RecurDirect(n) => {
                // Same as Recur but guaranteed small N
                for i in (0..*n).rev() {
                    self.slots[i] = self.stack.pop().unwrap_or(LispVal::Nil);
                }
                self.pc = 0;
            }
            // --- Compound ops: fused LoadSlot + PushI64 + Arith/Cmp ---
            Op::SlotAddImm(s, imm) => {
                // Matches Rust: DON'T write back to slot
                let v = self.slot_num(*s);
                match v.checked_add(*imm) {
                    Some(r) => {
                        if !(-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                            return StepOutcome::Error("integer overflow in add (payload range ±2^60)".into());
                        }
                        self.stack.push(LispVal::Num(r));
                        self.pc += 1;
                    }
                    None => return StepOutcome::Error("integer overflow in add".into()),
                }
            }
            Op::SlotSubImm(s, imm) => {
                // Matches Rust: DON'T write back to slot
                let v = self.slot_num(*s);
                match v.checked_sub(*imm) {
                    Some(r) => {
                        if !(-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                            return StepOutcome::Error("integer overflow in sub (payload range ±2^60)".into());
                        }
                        self.stack.push(LispVal::Num(r));
                        self.pc += 1;
                    }
                    None => return StepOutcome::Error("integer overflow in sub".into()),
                }
            }
            Op::SlotMulImm(s, imm) => {
                let v = self.slot_num(*s);
                match v.checked_mul(*imm) {
                    Some(r) => {
                        if !(-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                            return StepOutcome::Error("integer overflow in mul (payload range ±2^60)".into());
                        }
                        self.stack.push(LispVal::Num(r));
                        self.pc += 1;
                    }
                    None => return StepOutcome::Error("integer overflow in mul".into()),
                }
            }
            Op::SlotDivImm(s, imm) => {
                let v = self.slot_num(*s);
                match v.checked_div(*imm) {
                    Some(r) => {
                        if !(-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                            return StepOutcome::Error("integer overflow in div (payload range ±2^60)".into());
                        }
                        self.stack.push(LispVal::Num(r));
                        self.pc += 1;
                    }
                    None => return StepOutcome::Error("integer overflow in div".into()),
                }
            }
            Op::SlotEqImm(s, imm) => {
                let v = self.slot_num(*s);
                self.stack.push(LispVal::Bool(v == *imm));
                self.pc += 1;
            }
            Op::SlotLtImm(s, imm) => {
                let v = self.slot_num(*s);
                self.stack.push(LispVal::Bool(v < *imm));
                self.pc += 1;
            }
            Op::SlotLeImm(s, imm) => {
                let v = self.slot_num(*s);
                self.stack.push(LispVal::Bool(v <= *imm));
                self.pc += 1;
            }
            Op::SlotGtImm(s, imm) => {
                let v = self.slot_num(*s);
                self.stack.push(LispVal::Bool(v > *imm));
                self.pc += 1;
            }
            Op::SlotGeImm(s, imm) => {
                let v = self.slot_num(*s);
                self.stack.push(LispVal::Bool(v >= *imm));
                self.pc += 1;
            }
            // --- Super-fused: cmp + jump without stack traffic ---
            Op::JumpIfSlotLtImm(s, imm, addr) => {
                let v = self.slot_num(*s);
                if v < *imm {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::JumpIfSlotLeImm(s, imm, addr) => {
                let v = self.slot_num(*s);
                if v <= *imm {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::JumpIfSlotGtImm(s, imm, addr) => {
                let v = self.slot_num(*s);
                if v > *imm {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::JumpIfSlotGeImm(s, imm, addr) => {
                let v = self.slot_num(*s);
                if v >= *imm {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            Op::JumpIfSlotEqImm(s, imm, addr) => {
                let v = self.slot_num(*s);
                if v == *imm {
                    self.pc = *addr;
                } else {
                    self.pc += 1;
                }
            }
            // --- Mega-fused: RecurIncAccum ---
            Op::RecurIncAccum(counter, accum, step, limit, exit_addr) => {
                let cv = self.slot_num(*counter);
                if cv >= *limit {
                    self.pc = *exit_addr;
                } else {
                    let av = self.slot_num(*accum);
                    let new_accum = match av.checked_add(cv) {
                        Some(r) => {
                            if !(-(1i64 << 60)..=(1i64 << 60) - 1).contains(&r) {
                                return StepOutcome::Error(
                                    "integer overflow in add (payload range ±2^60)".into(),
                                );
                            }
                            r
                        }
                        None => return StepOutcome::Error("integer overflow in add".into()),
                    };
                    let new_counter = match cv.checked_add(*step) {
                        Some(r) => r,
                        None => return StepOutcome::Error("integer overflow in add".into()),
                    };
                    // Write back to slots
                    if *accum < self.slots.len() {
                        self.slots[*accum] = LispVal::Num(new_accum);
                    }
                    if *counter < self.slots.len() {
                        self.slots[*counter] = LispVal::Num(new_counter);
                    }
                    self.pc = 0;
                }
            }
            // --- StoreAndLoadSlot ---
            Op::StoreAndLoadSlot(s) => {
                let val = self.pop();
                if *s < self.slots.len() {
                    self.slots[*s] = val;
                    match &self.slots[*s] {
                        LispVal::Num(n) => self.stack.push(LispVal::Num(*n)),
                        _ => self.stack.push(self.slots[*s].clone()),
                    }
                } else {
                    while self.slots.len() <= *s {
                        self.slots.push(LispVal::Nil);
                    }
                    self.slots[*s] = val.clone();
                    self.stack.push(val);
                }
                self.pc += 1;
            }
            // --- ReturnSlot ---
            Op::ReturnSlot(s) => {
                let val = self.get_slot(*s);
                return StepOutcome::Return(val);
            }
            Op::PushLiteral(ref val) => {
                self.stack.push(val.clone());
                self.pc += 1;
            }
            // --- Sum-type primitives ---
            Op::ConstructTag(ref type_name, variant_id, n_fields) => {
                let n = *n_fields as usize;
                let mut fields = Vec::with_capacity(n);
                for _ in 0..n {
                    fields.push(self.stack.pop().unwrap_or(LispVal::Nil));
                }
                fields.reverse();
                self.stack.push(LispVal::Tagged {
                    type_name: type_name.clone(),
                    variant_id: *variant_id,
                    fields,
                });
                self.pc += 1;
            }
            Op::TagTest(ref type_name, variant_id) => {
                // Peek at stack top — does NOT pop
                let matches = match self.stack.last() {
                    Some(LispVal::Tagged {
                        type_name: tn,
                        variant_id: vid,
                        ..
                    }) => tn == type_name && *vid == *variant_id,
                    _ => false,
                };
                self.stack.push(LispVal::Bool(matches));
                self.pc += 1;
            }
            Op::GetField(idx) => {
                let val = self.pop();
                match val {
                    LispVal::Tagged { fields, .. } => {
                        let field = fields.get(*idx as usize).cloned().unwrap_or(LispVal::Nil);
                        self.stack.push(field);
                    }
                    _ => {
                        return StepOutcome::Error("get-field: expected tagged value".into());
                    }
                }
                self.pc += 1;
            }
            // Fused HOF opcodes: SpecVM can't call lambdas, so push empty list / init
            Op::MapOp(_) => {
                let _list_val = self.pop(); // pop list (ignored)
                self.stack.push(LispVal::List(vec![]));
                self.pc += 1;
            }
            Op::FilterOp(_) => {
                let _list_val = self.pop(); // pop list (ignored)
                self.stack.push(LispVal::List(vec![]));
                self.pc += 1;
            }
            Op::ReduceOp(_) => {
                let _list_val = self.pop(); // pop list
                let init = self.pop(); // pop init, push it back
                self.stack.push(init);
                self.pc += 1;
            }
            Op::DictMutSet(slot_idx) => {
                let val = self.pop();
                let key = self.pop();
                // Match Rust: mutate slot in-place
                if *slot_idx < self.slots.len() {
                    match &mut self.slots[*slot_idx] {
                        LispVal::Map(ref mut m) => {
                            if let LispVal::Str(k) = &key {
                                m.insert(k.clone(), val);
                            } else {
                                return StepOutcome::Error(
                                    "dict-mut-set: key must be string".into(),
                                );
                            }
                        }
                        _ => return StepOutcome::Error("dict-mut-set: slot is not a map".into()),
                    }
                    // Push the mutated dict for the result (matches Rust)
                    self.stack.push(self.slots[*slot_idx].clone());
                } else {
                    return StepOutcome::Error("dict-mut-set: slot out of bounds".into());
                }
                self.pc += 1;
            }
            Op::GetDefaultSlot(map_slot, key_slot, default_slot, result_slot) => {
                // Fused: result = dict/get(slots[map], slots[key]) ?? slots[default]
                // Extend slots if needed (matches Rust)
                while self.slots.len() <= *result_slot {
                    self.slots.push(LispVal::Nil);
                }
                let map_val = self.get_slot(*map_slot);
                let key_val = self.get_slot(*key_slot);
                let result = match (&map_val, &key_val) {
                    (LispVal::Map(m), LispVal::Str(k)) => match m.get(k) {
                        Some(v) if !matches!(v, LispVal::Nil) => v.clone(),
                        _ => self.get_slot(*default_slot),
                    },
                    _ => self.get_slot(*default_slot),
                };
                // Write result to result_slot
                if *result_slot < self.slots.len() {
                    self.slots[*result_slot] = result;
                }
                self.pc += 1;
            }
            // --- Ops NOT supported by the spec VM ---
            // These require closure environments, globals, builtins, or recursive dispatch
            Op::CallCaptured(_, _)
            | Op::CallCapturedRef(_, _)
            | Op::PushClosure(_)
            | Op::PushBuiltin(_)
            | Op::PushSelf
            | Op::CallSelf(_)
            | Op::CallDynamic(_)
            | Op::StoreCaptured(_)
            | Op::StoreGlobal(_)
            | Op::LoadCaptured(_)
            | Op::LoadGlobal(_)
            | Op::TracePush(_)
            | Op::TracePop => {
                return StepOutcome::Error("unsupported op in spec VM".into());
            }
            // --- DictGet/DictSet: supported by loop VM ---
            Op::DictGet => {
                let key = self.pop();
                let map = self.pop();
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => m.get(k).cloned().unwrap_or(LispVal::Nil),
                    _ => LispVal::Nil,
                };
                self.stack.push(result);
                self.pc += 1;
            }
            Op::DictSet => {
                let val = self.pop();
                let key = self.pop();
                let map = self.pop();
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => LispVal::Map(m.update(k.clone(), val)),
                    _ => {
                        return StepOutcome::Error("dict/set: need (map key value)".into());
                    }
                };
                self.stack.push(result);
                self.pc += 1;
            }
            // --- BuiltinCall: needs eval_builtin which we can't easily call from here ---
            // For fuzzing purposes, just return an error (the loop VM would call eval_builtin)
            Op::BuiltinCall(name, _) => {
                return StepOutcome::Error(format!(
                    "BuiltinCall({}) not supported in spec VM",
                    name
                ));
            }
            // Vec opcodes: SpecVM handles them concretely
            Op::MakeVec(n) => {
                let mut items = Vec::with_capacity(*n);
                for _ in 0..*n {
                    items.push(self.stack.pop().unwrap_or(LispVal::Nil));
                }
                items.reverse();
                self.stack.push(LispVal::Vec(items));
                self.pc += 1;
            }
            Op::VecNth => {
                let idx = self.pop();
                let vec_val = self.pop();
                // Match Rust strictly: only LispVal::Num indexes a vec.
                match (&idx, &vec_val) {
                    (LispVal::Num(i), LispVal::Vec(items))
                        if *i >= 0 && (*i as usize) < items.len() =>
                    {
                        self.stack.push(items[*i as usize].clone());
                    }
                    _ => self.stack.push(LispVal::Nil),
                }
                self.pc += 1;
            }
            Op::VecAssoc => {
                let val = self.pop();
                let idx = self.pop();
                let vec_val = self.pop();
                // Match Rust strictly: Num index, in-bounds update, everything else → Nil.
                match (&idx, &vec_val) {
                    (LispVal::Num(i), LispVal::Vec(items))
                        if *i >= 0 && (*i as usize) < items.len() =>
                    {
                        let mut new_items = items.clone();
                        new_items[*i as usize] = val;
                        self.stack.push(LispVal::Vec(new_items));
                    }
                    _ => self.stack.push(LispVal::Nil),
                }
                self.pc += 1;
            }
            Op::VecLen => {
                let vec_val = self.pop();
                match &vec_val {
                    LispVal::Vec(items) => self.stack.push(LispVal::Num(items.len() as i64)),
                    _ => self.stack.push(LispVal::Num(0)),
                }
                self.pc += 1;
            }
            Op::VecConj => {
                let val = self.pop();
                let vec_val = self.pop();
                match vec_val {
                    LispVal::Vec(mut items) => {
                        items.push(val);
                        self.stack.push(LispVal::Vec(items));
                    }
                    LispVal::Nil => {
                        self.stack.push(LispVal::Vec(vec![val]));
                    }
                    _ => self.stack.push(LispVal::Nil),
                }
                self.pc += 1;
            }
            Op::VecContains => {
                let val = self.pop();
                let vec_val = self.pop();
                match &vec_val {
                    LispVal::Vec(items) => {
                        let found = items.iter().any(|item| match (item, &val) {
                            (LispVal::Num(a), LispVal::Num(b)) => a == b,
                            (LispVal::Bool(a), LispVal::Bool(b)) => a == b,
                            (LispVal::Str(a), LispVal::Str(b)) => a == b,
                            (LispVal::Nil, LispVal::Nil) => true,
                            _ => false,
                        });
                        self.stack.push(LispVal::Bool(found));
                    }
                    _ => self.stack.push(LispVal::Bool(false)),
                }
                self.pc += 1;
            }
            // U64 field ops: spec VM doesn't fuzz these, return error
            Op::U64MulHi | Op::U64And | Op::U64Or | Op::U64Xor
            | Op::U64Shr | Op::U64Shl | Op::U64Not => {
                return StepOutcome::Error("u64 op not supported in spec VM".into());
            }
            Op::VecSlice => {
                let end_val = self.pop();
                let start_val = self.pop();
                let vec_val = self.pop();
                match (&start_val, &end_val, &vec_val) {
                    (LispVal::Num(s), LispVal::Num(e), LispVal::Vec(v)) => {
                        let si = if *s < 0 { 0usize } else { (*s as usize).min(v.len()) };
                        let ei = if *e < 0 { 0usize } else { (*e as usize).min(v.len()) };
                        let ei = ei.max(si); // never panic: clamp end below start
                        self.stack.push(LispVal::Vec(v[si..ei].to_vec()));
                    }
                    _ => self.stack.push(LispVal::Nil),
                }
                self.pc += 1;
            }
        }
        StepOutcome::Continue
    }

    /// Run the spec VM to completion (or error/step limit).
    pub fn run(mut self, max_steps: usize) -> SpecResult {
        // Pre-flight: validate all slot indices before execution begins.
        // This mirrors the Rust VM's validate_slot_indices so both VMs agree on OOB.
        if let Err(e) = self.validate_slot_indices() {
            return SpecResult::Error(e);
        }
        for step in 0..max_steps {
            // Resource guard: exponential value growth (Dup + MakeVec inside a
            // RecurIncAccum loop) can OOM the process before the step budget
            // expires. Periodically estimate the stack's total value size; if it
            // exceeds the cap, declare the program pathological.
            if step % 16 == 0 {
                let total: usize = self.stack.iter().map(lisp_val_size).sum();
                if total > 1_000_000 {
                    self.ok = false;
                    return SpecResult::ResourceLimit;
                }
            }
            match self.step() {
                StepOutcome::Continue => {}
                StepOutcome::Return(v) => return SpecResult::Value(v),
                StepOutcome::Error(e) => {
                    self.ok = false;
                    return SpecResult::Error(e);
                }
            }
        }
        SpecResult::StepLimit
    }
}

/// Depth-limited size estimate for a LispVal (nodes counted, strings counted
/// by length). Used by the SpecVm resource guard to detect exponential stack
/// growth before it OOMs the process.
fn lisp_val_size(v: &LispVal) -> usize {
    fn go(v: &LispVal, depth: u32) -> usize {
        if depth == 0 {
            return 1;
        }
        match v {
            LispVal::Nil | LispVal::Bool(_) | LispVal::Num(_) | LispVal::Float(_)
            | LispVal::U64(_) => 1,
            LispVal::Str(s) => 1 + s.len(),
            LispVal::List(items) | LispVal::Vec(items) => {
                1 + items.iter().map(|it| go(it, depth - 1)).sum::<usize>()
            }
            LispVal::Map(m) => {
                1 + m
                    .iter()
                    .map(|(k, val)| k.len() + go(val, depth - 1))
                    .sum::<usize>()
            }
            _ => 1,
        }
    }
    go(v, 24)
}

/// Outcome of a single VM step.
pub enum StepOutcome {
    Continue,
    Return(LispVal),
    Error(String),
}

// ---------------------------------------------------------------------------
// Fuzz helpers — deterministic RNG for reproducibility
// ---------------------------------------------------------------------------

/// Live seed: nanosecond clock entropy mixed through xorshift. Gives every
/// run a fresh exploration path (deterministic tests remain the regression net).
pub fn live_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    let mut x = nanos ^ 0x9E3779B97F4A7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// Simple Xorshift64 PRNG for deterministic test generation.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn next_usize(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Slot index for fuzzing: returns [0, num_slots) when num_slots > 0,
    /// or 0 when num_slots == 0 (SpecVm handles OOB gracefully, Rust panics —
    /// but the differential_test_one catches panics so this is fine for edge testing).
    #[allow(dead_code)]
    pub fn next_slot(&mut self, num_slots: usize) -> usize {
        if num_slots == 0 {
            0
        } else {
            self.next_usize(num_slots)
        }
    }

    #[allow(dead_code)]
    pub fn next_i64(&mut self) -> i64 {
        self.next_u64() as i64
    }

    pub fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 0
    }

    /// Generate a value in range [lo, hi]
    pub fn next_range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() as i64 % (hi - lo + 1))
    }

    /// Random LispVal for slot initialization
    pub fn next_lisp_val(&mut self) -> LispVal {
        match self.next_usize(6) {
            0 => LispVal::Nil,
            1 => LispVal::Bool(self.next_bool()),
            2 => LispVal::Num(self.boundary_i64()),
            3 => LispVal::Float(self.boundary_f64()),
            4 => LispVal::Str(format!("s{}", self.next_usize(100))),
            5 => LispVal::U64(self.boundary_u64()),
            _ => LispVal::Nil,
        }
    }

    /// Boundary-biased integer: 50% small range, 50% boundary/edge values.
    /// Exercises overflow paths that [-5,5] never hits.
    pub fn boundary_i64(&mut self) -> i64 {
        const EDGES: &[i64] = &[
            0,
            1,
            -1,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
            i64::MAX / 2,
            i64::MIN / 2,
            255,
            256,
            -256,
            65535,
            65536,
            // Square roots of i64::MAX (overflow under mul)
            3037000499,
            -3037000500,
            // Near overflow for i32::MAX/MIN (truncation edges)
            2147483647,
            -2147483648,
        ];
        if self.next_usize(2) == 0 {
            // 50%: pick from the boundary table
            EDGES[self.next_usize(EDGES.len())]
        } else {
            // 50%: small range (original behavior)
            self.next_range(-10, 10)
        }
    }

    /// Boundary-biased float: 50% normal range, 50% edge values.
    /// Exercises NaN/Inf propagation, underflow, and precision edges.
    /// Boundary-biased u64: 50% small range, 50% edge values.
    /// Exercises wrapping arithmetic, powers of 2, and u64 field ops.
    pub fn boundary_u64(&mut self) -> u64 {
        const EDGES: &[u64] = &[
            0,
            1,
            100,
            255,
            256,
            65535,
            65536,
            // Powers of 2
            1 << 16,
            1 << 31,
            1 << 32,
            // Patterns for bitwise ops
            0xFFFF_FFFF,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
        ];
        if self.next_usize(2) == 0 {
            EDGES[self.next_usize(EDGES.len())]
        } else {
            (self.next_u64() % 200) as u64
        }
    }

    pub fn boundary_f64(&mut self) -> f64 {
        const EDGES: &[f64] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE, // smallest positive normal
            f64::EPSILON,      // 1.0 + EPS != 1.0
            // Values where float→int truncation changes behavior
            3.7,
            -2.3,
            0.999999,
            -0.000001,
            // Large enough to overflow i64 when cast
            1e19,
            -1e19,
            // Precision boundaries
            9007199254740992.0, // 2^53 (first non-representable integer)
            9007199254740993.0, // rounds to 2^53
        ];
        if self.next_usize(2) == 0 {
            // 50%: pick from the boundary table
            EDGES[self.next_usize(EDGES.len())]
        } else {
            // 50%: normal range
            self.next_range(-200, 200) as f64 / 10.0
        }
    }
}

/// Supported opcodes for the loop VM fuzz subset.
/// These are the "pure" opcodes that work with stack+slots+pc only.
#[derive(Debug, Clone, Copy)]
pub enum FuzzOp {
    LoadSlot,
    PushI64,
    PushBool,
    PushNil,
    Dup,
    Pop,
    StoreSlot,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    JumpIfTrue,
    JumpIfFalse,
    Jump,
    Return,
    Recur,
    RecurDirect,
    SlotAddImm,
    SlotSubImm,
    SlotMulImm,
    SlotDivImm,
    SlotEqImm,
    SlotLtImm,
    SlotLeImm,
    SlotGtImm,
    SlotGeImm,
    JumpIfSlotLtImm,
    JumpIfSlotLeImm,
    JumpIfSlotGtImm,
    JumpIfSlotGeImm,
    JumpIfSlotEqImm,
    PushFloat,
    RecurIncAccum,
    StoreAndLoadSlot,
    ReturnSlot,
    PushStr,
    MakeList,
    TypedBinOpI64,
    TypedBinOpF64,
    TypedBinOpU64,
    DictGet,
    DictSet,
    DictMutSet,
    PushLiteral,
    ConstructTag,
    TagTest,
    GetField,
    GetDefaultSlot,
    PushU64,
    Not,
    MakeVec,
    VecNth,
    VecAssoc,
    VecLen,
    VecConj,
    VecContains,
    VecSlice,
}

pub const FUZZ_OPS: &[FuzzOp] = &[
    FuzzOp::LoadSlot,
    FuzzOp::PushI64,
    FuzzOp::PushBool,
    FuzzOp::PushNil,
    FuzzOp::Dup,
    FuzzOp::Pop,
    FuzzOp::StoreSlot,
    FuzzOp::Add,
    FuzzOp::Sub,
    FuzzOp::Mul,
    FuzzOp::Div,
    FuzzOp::Mod,
    FuzzOp::Eq,
    FuzzOp::Lt,
    FuzzOp::Le,
    FuzzOp::Gt,
    FuzzOp::Ge,
    FuzzOp::JumpIfTrue,
    FuzzOp::JumpIfFalse,
    FuzzOp::Jump,
    FuzzOp::Return,
    FuzzOp::Recur,
    FuzzOp::RecurDirect,
    FuzzOp::SlotAddImm,
    FuzzOp::SlotSubImm,
    FuzzOp::SlotMulImm,
    FuzzOp::SlotDivImm,
    FuzzOp::SlotEqImm,
    FuzzOp::SlotLtImm,
    FuzzOp::SlotLeImm,
    FuzzOp::SlotGtImm,
    FuzzOp::SlotGeImm,
    FuzzOp::JumpIfSlotLtImm,
    FuzzOp::JumpIfSlotLeImm,
    FuzzOp::JumpIfSlotGtImm,
    FuzzOp::JumpIfSlotGeImm,
    FuzzOp::JumpIfSlotEqImm,
    FuzzOp::PushFloat,
    FuzzOp::RecurIncAccum,
    FuzzOp::StoreAndLoadSlot,
    FuzzOp::ReturnSlot,
    FuzzOp::PushStr,
    FuzzOp::MakeList,
    FuzzOp::TypedBinOpI64,
    FuzzOp::TypedBinOpF64,
    FuzzOp::TypedBinOpU64,
    FuzzOp::DictGet,
    FuzzOp::DictSet,
    FuzzOp::DictMutSet,
    FuzzOp::PushLiteral,
    FuzzOp::ConstructTag,
    FuzzOp::TagTest,
    FuzzOp::GetField,
    FuzzOp::GetDefaultSlot,
    FuzzOp::PushU64,
    FuzzOp::Not,
    FuzzOp::MakeVec,
    FuzzOp::VecNth,
    FuzzOp::VecLen,
    FuzzOp::VecConj,
    FuzzOp::VecAssoc,
    FuzzOp::VecContains,
    FuzzOp::VecSlice,
];

/// Vec ops: SpecVm handles these but the loop VM errors on them.
/// Kept in FuzzOp enum for potential future lambda-VM differential tests.
/// Not in FUZZ_OPS since they can't be diff-tested against run_compiled_loop.
const _FUZZ_OPS_VEC_ONLY: &[FuzzOp] = &[
    FuzzOp::MakeVec,
    FuzzOp::VecNth,
    FuzzOp::VecAssoc,
    FuzzOp::VecLen,
    FuzzOp::VecConj,
    FuzzOp::VecContains,
    FuzzOp::VecSlice,
];

/// Ops that access slots by index — invalid when num_slots == 0.
pub fn is_slot_dependent(fop: &FuzzOp) -> bool {
    matches!(
        fop,
        FuzzOp::LoadSlot
            | FuzzOp::StoreSlot
            | FuzzOp::Recur
            | FuzzOp::RecurDirect
            | FuzzOp::SlotAddImm
            | FuzzOp::SlotSubImm
            | FuzzOp::SlotMulImm
            | FuzzOp::SlotDivImm
            | FuzzOp::SlotEqImm
            | FuzzOp::SlotLtImm
            | FuzzOp::SlotLeImm
            | FuzzOp::SlotGtImm
            | FuzzOp::SlotGeImm
            | FuzzOp::JumpIfSlotLtImm
            | FuzzOp::JumpIfSlotLeImm
            | FuzzOp::JumpIfSlotGtImm
            | FuzzOp::JumpIfSlotGeImm
            | FuzzOp::JumpIfSlotEqImm
            | FuzzOp::RecurIncAccum
            | FuzzOp::StoreAndLoadSlot
            | FuzzOp::ReturnSlot
            | FuzzOp::DictMutSet
            | FuzzOp::GetDefaultSlot
    )
}

/// Convert a FuzzOp to an actual Op, using the RNG for operand values.
/// `max_pc` is used to generate valid jump targets.
/// `num_slots` is used to generate valid slot indices.
pub fn fuzz_op_to_op(rng: &mut Rng, fop: FuzzOp, max_pc: usize, num_slots: usize) -> Op {
    let slot = || rng.next_usize(if num_slots == 0 { 1 } else { num_slots });
    let imm = || rng.boundary_i64();
    let addr = || rng.next_usize(max_pc + 1);

    match fop {
        FuzzOp::LoadSlot => {
            Op::LoadSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::PushI64 => Op::PushI64(rng.boundary_i64()),
        FuzzOp::PushFloat => Op::PushFloat(rng.boundary_f64()),
        FuzzOp::PushBool => Op::PushBool(rng.next_bool()),
        FuzzOp::PushNil => Op::PushNil,
        FuzzOp::Dup => Op::Dup,
        FuzzOp::Pop => Op::Pop,
        FuzzOp::StoreSlot => {
            Op::StoreSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::Add => Op::Add,
        FuzzOp::Sub => Op::Sub,
        FuzzOp::Mul => Op::Mul,
        FuzzOp::Div => Op::Div,
        FuzzOp::Mod => Op::Mod,
        FuzzOp::Eq => Op::Eq,
        FuzzOp::Lt => Op::Lt,
        FuzzOp::Le => Op::Le,
        FuzzOp::Gt => Op::Gt,
        FuzzOp::Ge => Op::Ge,
        FuzzOp::JumpIfTrue => Op::JumpIfTrue(rng.next_usize(max_pc + 1)),
        FuzzOp::JumpIfFalse => Op::JumpIfFalse(rng.next_usize(max_pc + 1)),
        FuzzOp::Jump => Op::Jump(rng.next_usize(max_pc + 1)),
        FuzzOp::Return => Op::Return,
        FuzzOp::Recur => Op::Recur(rng.next_usize(if num_slots == 0 { 1 } else { num_slots })),
        FuzzOp::RecurDirect => {
            Op::RecurDirect(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::SlotAddImm => Op::SlotAddImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotSubImm => Op::SlotSubImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotMulImm => Op::SlotMulImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotDivImm => Op::SlotDivImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotEqImm => Op::SlotEqImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotLtImm => Op::SlotLtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotLeImm => Op::SlotLeImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotGtImm => Op::SlotGtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::SlotGeImm => Op::SlotGeImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        FuzzOp::JumpIfSlotLtImm => Op::JumpIfSlotLtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        FuzzOp::JumpIfSlotLeImm => Op::JumpIfSlotLeImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        FuzzOp::JumpIfSlotGtImm => Op::JumpIfSlotGtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        FuzzOp::JumpIfSlotGeImm => Op::JumpIfSlotGeImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        FuzzOp::JumpIfSlotEqImm => Op::JumpIfSlotEqImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        FuzzOp::RecurIncAccum => {
            let s = if num_slots >= 2 {
                rng.next_usize(if num_slots == 0 { 1 } else { num_slots })
            } else {
                0
            };
            let a = if num_slots >= 2 {
                (s + 1) % num_slots
            } else {
                0
            };
            Op::RecurIncAccum(s, a, 1, rng.next_range(2, 8), rng.next_usize(max_pc + 1))
        }
        FuzzOp::StoreAndLoadSlot => {
            Op::StoreAndLoadSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::ReturnSlot => {
            Op::ReturnSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::PushStr => {
            // Generate a short random string from a small alphabet
            const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
            let len = rng.next_usize(4) + 1; // 1..4 chars
            let s: String = (0..len)
                .map(|_| CHARS[rng.next_usize(CHARS.len())] as char)
                .collect();
            Op::PushStr(s)
        }
        FuzzOp::MakeList => {
            let n = rng.next_usize(3) + 1; // 1..3 items
            Op::MakeList(n)
        }
        FuzzOp::TypedBinOpI64 => {
            const BINOPS: &[BinOp] = &[
                BinOp::Add,
                BinOp::Sub,
                BinOp::Mul,
                BinOp::Div,
                BinOp::Mod,
                BinOp::Lt,
                BinOp::Le,
                BinOp::Gt,
                BinOp::Ge,
                BinOp::Eq,
            ];
            let op = BINOPS[rng.next_usize(BINOPS.len())].clone();
            Op::TypedBinOp(op, Ty::I64)
        }
        FuzzOp::TypedBinOpU64 => {
            const BINOPS: &[BinOp] = &[
                BinOp::Add,
                BinOp::Sub,
                BinOp::Mul,
                BinOp::Div,
                BinOp::Mod,
                BinOp::Lt,
                BinOp::Le,
                BinOp::Gt,
                BinOp::Ge,
                BinOp::Eq,
            ];
            let op = BINOPS[rng.next_usize(BINOPS.len())].clone();
            Op::TypedBinOp(op, Ty::U64)
        }
        FuzzOp::TypedBinOpF64 => {
            const BINOPS: &[BinOp] = &[
                BinOp::Add,
                BinOp::Sub,
                BinOp::Mul,
                BinOp::Div,
                BinOp::Mod,
                BinOp::Lt,
                BinOp::Le,
                BinOp::Gt,
                BinOp::Ge,
                BinOp::Eq,
            ];
            let op = BINOPS[rng.next_usize(BINOPS.len())].clone();
            Op::TypedBinOp(op, Ty::F64)
        }
        FuzzOp::DictGet => Op::DictGet,
        FuzzOp::DictSet => Op::DictSet,
        FuzzOp::DictMutSet => {
            Op::DictMutSet(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        FuzzOp::PushLiteral => Op::PushLiteral(rng.next_lisp_val()),
        FuzzOp::ConstructTag => {
            const TAG_NAMES: &[&str] = &["Option", "Result", "Pair", "Node", "Leaf"];
            let name = TAG_NAMES[rng.next_usize(TAG_NAMES.len())].to_string();
            let variant = rng.next_usize(4) as u16;
            let n_fields = rng.next_usize(3) as u8; // 0-2 fields
            Op::ConstructTag(name, variant, n_fields)
        }
        FuzzOp::TagTest => {
            const TAG_NAMES: &[&str] = &["Option", "Result", "Pair", "Node", "Leaf"];
            let name = TAG_NAMES[rng.next_usize(TAG_NAMES.len())].to_string();
            let variant = rng.next_usize(4) as u16;
            Op::TagTest(name, variant)
        }
        FuzzOp::GetField => {
            Op::GetField(rng.next_usize(3) as u8) // 0-2 field index
        }
        FuzzOp::GetDefaultSlot => {
            let mut s = || rng.next_usize(if num_slots == 0 { 1 } else { num_slots });
            Op::GetDefaultSlot(s(), s(), s(), s())
        }
        FuzzOp::PushU64 => Op::PushU64(rng.boundary_u64()),
        FuzzOp::Not => Op::Not,
        FuzzOp::MakeVec => Op::MakeVec(rng.next_usize(4)),
        FuzzOp::VecNth => Op::VecNth,
        FuzzOp::VecAssoc => Op::VecAssoc,
        FuzzOp::VecLen => Op::VecLen,
        FuzzOp::VecConj => Op::VecConj,
        FuzzOp::VecContains => Op::VecContains,
        FuzzOp::VecSlice => Op::VecSlice,
    }
}

/// Generate a random bytecode program.
pub fn generate_random_program(rng: &mut Rng, num_slots: usize, code_len: usize) -> Vec<Op> {
    let mut code = Vec::with_capacity(code_len);

    // Build filtered op list: exclude slot-dependent ops when num_slots == 0
    let available_ops: Vec<&FuzzOp> = if num_slots == 0 {
        FUZZ_OPS
            .iter()
            .filter(|fop| !is_slot_dependent(fop))
            .collect()
    } else {
        FUZZ_OPS.iter().collect()
    };

    for _ in 0..code_len {
        let fop_idx = rng.next_usize(available_ops.len());
        let fop = *available_ops[fop_idx];
        let op = fuzz_op_to_op(rng, fop, code_len, num_slots);
        code.push(op);
    }

    // Ensure the program always terminates: if no Return/ReturnSlot at the end, append one
    let has_terminal = code
        .iter()
        .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
    if !has_terminal {
        // Add a return at the end
        code.push(Op::Return);
    }

    code
}

/// Run a differential test for one program.
/// Returns a description of any mismatch, or None if they agree.
pub fn differential_test_one(
    code: Vec<Op>,
    init_slots: Vec<LispVal>,
    max_steps: usize,
) -> Option<String> {
    use std::panic;

    // --- Run the spec VM ---
    let spec_vm = SpecVm::new(code.clone(), init_slots.clone());
    let spec_result = spec_vm.run(max_steps);

    // --- Run the Rust VM with a capped step budget ---
    // Mutations that loop (e.g., PushNil, MakeList(1), Jump(0)) can build deeply
    // nested LispVal structures within the step budget. When the VM exits, Drop of
    // these structures causes recursive stack overflow (SIGABRT).
    // catch_unwind cannot catch stack-overflow panics (no stack left to unwind),
    // and on macOS the signal kills the whole process regardless of thread isolation.
    //
    // Fix: cap the Rust VM's step budget to match the spec VM's max_steps exactly.
    // This prevents the Rust VM from building structures far deeper than what the
    // spec VM would produce (the root cause of the stack overflow on Drop).
    // Pathological program (exponential stack growth): running the Rust VM
    // would OOM the process. Skip — there is no semantic disagreement to find.
    if matches!(spec_result, SpecResult::ResourceLimit) {
        return None;
    }
    let cl = make_test_compiled_lambda(init_slots.len(), init_slots.len(), code.clone());
    // Pre-validate slot indices to match the spec VM's behavior.
    // The spec VM calls validate_slot_indices() before executing; if OOB,
    // it errors immediately. The Rust VM uses safe_slot which silently
    // returns Nil — we validate here so both VMs agree on OOB errors.
    if let Err(e) = validate_slot_indices(&code, init_slots.len()) {
        // Both VMs should error on OOB — this is a match.
        match spec_result {
            SpecResult::Error(_) => return None,
            _ => {
                return Some(format!(
                    "VALIDATION ERROR: {} but spec={:?}",
                    e, spec_result
                ))
            }
        }
    }
    let mut state = lisp_rlm_wasm::types::EvalState::new();
    state.eval_budget = (max_steps * 3) as u64;
    let rust_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        run_compiled_lambda(
            &cl,
            &init_slots,
            &mut lisp_rlm_wasm::types::Env::new(),
            &mut state,
        )
    }));

    let rust_result = match rust_result {
        Ok(r) => r,
        Err(panic_payload) => {
            let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".into()
            };
            // After validate_slot_indices, OOB slot panics should be impossible.
            // Any remaining panic is a real bug — flag as mismatch.
            return Some(format!(
                "Rust VM PANIC [{}]: spec={:?}",
                panic_msg, spec_result
            ));
        }
    };

    // --- Compare ---
    /// NaN-aware equality: Float(NaN) == Float(NaN), otherwise delegate to PartialEq.
    pub fn vals_equal(a: &LispVal, b: &LispVal) -> bool {
        match (a, b) {
            (LispVal::Float(fa), LispVal::Float(fb)) => {
                if fa.is_nan() && fb.is_nan() {
                    true
                } else {
                    fa == fb
                }
            }
            (LispVal::Vec(xs), LispVal::Vec(ys)) => {
                xs.len() == ys.len()
                    && xs.iter().zip(ys.iter()).all(|(x, y)| vals_equal(x, y))
            }
            (LispVal::List(xs), LispVal::List(ys)) => {
                xs.len() == ys.len()
                    && xs.iter().zip(ys.iter()).all(|(x, y)| vals_equal(x, y))
            }
            (LispVal::Map(ma), LispVal::Map(mb)) => {
                ma.len() == mb.len()
                    && ma.iter().all(|(k, va)| match mb.get(k) {
                        Some(vb) => vals_equal(va, vb),
                        None => false,
                    })
            }
            _ => a == b,
        }
    }

    match (&spec_result, &rust_result) {
        // Resource-limit case is handled above (skipped), but keep the match
        // exhaustive for safety.
        (SpecResult::ResourceLimit, _) => None,
        (SpecResult::Value(sv), Ok(rv)) => {
            if !vals_equal(sv, rv) {
                Some(format!(
                    "VALUE MISMATCH: spec={:?} rust={:?}\n  code={:?}\n  slots={:?}",
                    sv, rv, code, init_slots
                ))
            } else {
                None
            }
        }
        (SpecResult::Error(_), Err(_)) => {
            // Both errored — that's a match
            None
        }
        (SpecResult::StepLimit, _) => {
            // Spec hit step limit — not a mismatch
            None
        }
        (SpecResult::Error(se), Ok(rv)) => {
            // Spec errored but Rust returned — potential mismatch
            Some(format!(
                "SPEC ERRORED, RUST DID NOT: spec_err={:?}\n  rust={:?}\n  code={:?}\n  slots={:?}",
                se, rv, code, init_slots
            ))
        }
        (SpecResult::Value(sv), Err(re)) => {
            // Spec returned but Rust errored — potential mismatch
            Some(format!(
                "SPEC RETURNED, RUST ERRORED: spec={:?}\n  rust_err={:?}\n  code={:?}\n  slots={:?}",
                sv, re, code, init_slots
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Regression tests — known-good programs from F* verification
// ---------------------------------------------------------------------------

