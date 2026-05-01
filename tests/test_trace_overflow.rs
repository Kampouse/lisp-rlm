//! Trace mismatch #80 from test_differential_fuzz_overflow
use lisp_rlm_wasm::bytecode::*;
use lisp_rlm_wasm::LispVal;

// Minimal SpecVm for tracing
#[derive(Debug, Clone)]
enum SpecResult {
    Value(LispVal),
    Error(String),
    StepLimit,
}

struct MiniSpec {
    stack: Vec<LispVal>,
    slots: Vec<LispVal>,
    pc: usize,
    code: Vec<Op>,
}

impl MiniSpec {
    fn new(code: Vec<Op>, slots: Vec<LispVal>) -> Self {
        Self { stack: Vec::new(), slots, pc: 0, code }
    }
    
    fn pop(&mut self) -> LispVal { self.stack.pop().unwrap_or(LispVal::Nil) }
    fn num_val(v: &LispVal) -> i64 {
        match v { LispVal::Num(n) => *n, LispVal::Float(f) => *f as i64, _ => 0 }
    }
    
    fn run(&mut self, max_steps: usize) -> SpecResult {
        for _ in 0..max_steps {
            if self.pc >= self.code.len() { break; }
            let op = self.code[self.pc].clone();
            eprintln!("  pc={} op={:?} stack={:?}", self.pc, op, self.stack);
            match &op {
                Op::SlotAddImm(s, imm) => {
                    let v = Self::num_val(&self.slots[*s]);
                    match v.checked_add(*imm) {
                        Some(r) => { self.stack.push(LispVal::Num(r)); self.pc += 1; }
                        None => return SpecResult::Error("integer overflow in add".into()),
                    }
                }
                Op::PushI64(n) => { self.stack.push(LispVal::Num(*n)); self.pc += 1; }
                Op::Add => {
                    let b = self.pop();
                    let a = self.pop();
                    let av = Self::num_val(&a);
                    let bv = Self::num_val(&b);
                    match av.checked_add(bv) {
                        Some(r) => { self.stack.push(LispVal::Num(r)); self.pc += 1; }
                        None => return SpecResult::Error("integer overflow in add".into()),
                    }
                }
                Op::Sub => {
                    let b = self.pop();
                    let a = self.pop();
                    let av = Self::num_val(&a);
                    let bv = Self::num_val(&b);
                    eprintln!("    Sub: a={:?} b={:?} av={} bv={}", a, b, av, bv);
                    match av.checked_sub(bv) {
                        Some(r) => { self.stack.push(LispVal::Num(r)); self.pc += 1; }
                        None => return SpecResult::Error("integer overflow in sub".into()),
                    }
                }
                Op::Mul => {
                    let b = self.pop();
                    let a = self.pop();
                    let av = Self::num_val(&a);
                    let bv = Self::num_val(&b);
                    match av.checked_mul(bv) {
                        Some(r) => { self.stack.push(LispVal::Num(r)); self.pc += 1; }
                        None => return SpecResult::Error("integer overflow in mul".into()),
                    }
                }
                Op::Return => {
                    return SpecResult::Value(self.pop());
                }
                _ => { self.pc += 1; }
            }
        }
        SpecResult::StepLimit
    }
}

#[test]
fn trace_mismatch_80() {
    let code = vec![
        Op::SlotAddImm(1, -22),
        Op::PushI64(1),
        Op::Add,
        Op::Sub,
        Op::PushI64(-4611686018427387904i64),
        Op::Mul,
        Op::PushI64(-14),
        Op::Return,
    ];
    let init_slots = vec![
        LispVal::Num(1),
        LispVal::Num(-4611686018427387904i64),
        LispVal::Num(256),
    ];

    eprintln!("=== Rust VM ===");
    let cl = make_test_compiled_lambda(3, 3, code.clone());
    let rust_result = run_lambda_test(&cl, &init_slots);
    eprintln!("Rust result: {:?}", rust_result);

    eprintln!("=== MiniSpec VM ===");
    let mut spec = MiniSpec::new(code, init_slots);
    let spec_result = spec.run(10000);
    eprintln!("Spec result: {:?}", spec_result);
}

#[test]
fn test_div_overflow_returns_error() {
    // i64::MIN / -1 overflows
    let code = vec![
        Op::PushI64(i64::MIN),
        Op::PushI64(-1),
        Op::Div,
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code);
    let result = run_lambda_test(&cl, &[]);
    eprintln!("Div overflow result: {:?}", result);
    assert!(result.is_err(), "Expected error for div overflow, got {:?}", result);
    assert!(result.unwrap_err().contains("overflow"));
}
