//! Extract mismatching program from differential fuzzer.
//! Run: cargo test --test extract_mismatch -- --nocapture

use lisp_rlm_wasm::bytecode::{make_test_compiled_loop, Op, BinOp, Ty};
use lisp_rlm_wasm::types::LispVal;
use std::collections::BTreeMap;

// Copied minimal RNG from test_differential_fuzz
struct Rng { state: u64 }
impl Rng {
    fn new(seed: u64) -> Self { Self { state: if seed == 0 { 1 } else { seed } } }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        self.state = x; x
    }
    fn next_usize(&mut self, n: usize) -> usize { (self.next_u64() % n as u64) as usize }
    fn next_bool(&mut self) -> bool { self.next_u64() & 1 == 0 }
    fn boundary_i64(&mut self) -> i64 {
        const EDGES: &[i64] = &[
            0, 1, -1, i64::MAX, i64::MIN, i64::MAX - 1, i64::MIN + 1,
            i64::MAX / 2, i64::MIN / 2, 255, 256, -256, 65535, 65536,
            3037000499, -3037000500, 2147483647, -2147483648,
        ];
        if self.next_usize(2) == 0 { EDGES[self.next_usize(EDGES.len())] }
        else { 0 + (self.next_u64() as i64 % (10 - 0 + 1)) }
    }
    fn boundary_f64(&mut self) -> f64 {
        const EDGES: &[f64] = &[
            0.0, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN,
            f64::MAX, f64::MIN, f64::MIN_POSITIVE, f64::EPSILON,
            3.7, -2.3, 0.999999, -0.000001, 1e19, -1e19,
            9007199254740992.0, 9007199254740993.0,
        ];
        if self.next_usize(2) == 0 { EDGES[self.next_usize(EDGES.len())] }
        else { (0 + (self.next_u64() as i64 % (200 - (-200) + 1))) as f64 / 10.0 }
    }
    fn boundary_u64(&mut self) -> u64 {
        const EDGES: &[u64] = &[0, 1, 100, 255, 256, 65535, 65536, 65536, 2147483647, 2147483648,
            0xFFFF_FFFF, 0xAAAA_AAAA_AAAA_AAAA, 0x5555_5555_5555_5555];
        if self.next_usize(2) == 0 { EDGES[self.next_usize(EDGES.len())] }
        else { self.next_u64() % 200 }
    }
    fn next_lisp_val(&mut self) -> LispVal {
        match self.next_usize(6) {
            0 => LispVal::Nil, 1 => LispVal::Bool(self.next_bool()),
            2 => LispVal::Num(self.boundary_i64()), 3 => LispVal::Float(self.boundary_f64()),
            4 => LispVal::Str(format!("s{}", self.next_usize(100))),
            5 => LispVal::U64(self.boundary_u64()),
            _ => LispVal::Nil,
        }
    }
}

fn is_slot_dependent(op: &Op) -> bool {
    matches!(op,
        Op::LoadSlot(_) | Op::StoreSlot(_) | Op::Recur(_) | Op::RecurDirect(_)
        | Op::SlotAddImm(_, _) | Op::SlotSubImm(_, _) | Op::SlotMulImm(_, _)
        | Op::SlotDivImm(_, _) | Op::SlotEqImm(_, _) | Op::SlotLtImm(_, _)
        | Op::SlotLeImm(_, _) | Op::SlotGtImm(_, _) | Op::SlotGeImm(_, _)
        | Op::JumpIfSlotLtImm(_, _, _) | Op::JumpIfSlotLeImm(_, _, _)
        | Op::JumpIfSlotGtImm(_, _, _) | Op::JumpIfSlotGeImm(_, _, _)
        | Op::JumpIfSlotEqImm(_, _, _) | Op::RecurIncAccum(_, _, _, _, _)
        | Op::StoreAndLoadSlot(_) | Op::ReturnSlot(_) | Op::DictMutSet(_)
        | Op::GetDefaultSlot(_, _, _, _)
    )
}

#[test]
// IGNORED scaffold (was Kampouse WIP swept into commit 4274439 by add -A).
// Panics by design until generate_random_program is imported from the
// differential fuzzer. Un-ignore + implement when that lands.
#[ignore = "scaffold: needs generate_random_program from test_differential_fuzz"]
fn extract_mismatch_1278() {
    let mut rng = Rng::new(12345);
    for i in 0..1279 {
        let num_slots = rng.next_usize(4) + 1;
        let code_len = rng.next_usize(15) + 5;
        // ... generate program ...
        // For now just advance the RNG state
        let _ = (num_slots, code_len);
    }
    // After this loop, the RNG state matches program #1278
    // We'd need the full generate_random_program to reproduce
    panic!("This test needs generate_random_program imported");
}
