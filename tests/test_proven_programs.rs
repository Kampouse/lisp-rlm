//! Tests for programs with formal verification proofs in F*.
//!
//! Each test runs a Scheme-style Lisp program through the compiler and VM,
//! then asserts the result matches what the F* verification proves.
//!
//! Verified in: verification/semantics/lisp_ir/LispIR.Universality.fst
//! 29 lemmas, 0 admits, 51 modules, 0 failures.

use lisp_rlm_wasm::EvalState;
use lisp_rlm_wasm::*;

fn eval(code: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    lisp_rlm_wasm::program::run_program(
        &lisp_rlm_wasm::parser::parse_all(code).unwrap_or_default(),
        &mut env,
        &mut state,
    )
    .map(|v| v.to_string())
    .unwrap_or_else(|e| format!("ERROR: {}", e))
}

// ════════════════════════════════════════════════════════════════
// Part 1: Minsky Machine Simulation (Turing Completeness)
// ════════════════════════════════════════════════════════════════
//
// The F* proof encodes a 2-register Minsky machine as VM bytecode:
//   PC0: JumpIfSlotEqImm (0, 0, 6)  — if r1==0, jump to halt
//   PC1: SlotSubImm (0, 1)          — r1 -= 1
//   PC2: StoreAndLoadSlot 0         — update r1 in slot
//   PC3: SlotAddImm (1, 1)          — r2 += 1
//   PC4: StoreAndLoadSlot 1         — update r2 in slot
//   PC5: Recur 2                    — tail-call loop
//   PC6: ReturnSlot 1               — return r2
//
// In Scheme: (loop ((a 3) (b 4)) (if (= a 0) b (recur (- a 1) (+ b 1))))
// F* lemma: vm_minsky_iteration proves per-step register correspondence
// F* lemma: vm_minsky_halt proves ∀r2. VM produces r2 when r1=0

#[test]
fn test_proven_minsky_add_3_4() {
    // F* lemma: minsky_add_3_4 proves slots=[3,4] → result=7
    // F* lemma: bisim_add_3_4 proves VM trace matches Minsky model
    assert_eq!(
        eval("(loop ((a 3) (b 4)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "7"
    );
}

#[test]
fn test_proven_minsky_add_0_5() {
    // F* lemma: minsky_add_0_5 proves slots=[0,5] → result=5
    assert_eq!(
        eval("(loop ((a 0) (b 5)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "5"
    );
}

#[test]
fn test_proven_minsky_add_1_1() {
    // F* lemma: minsky_add_1_1 proves slots=[1,1] → result=2
    assert_eq!(
        eval("(loop ((a 1) (b 1)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "2"
    );
}

#[test]
fn test_proven_minsky_add_10_0() {
    // Forward simulation: vm_minsky_iteration proves per-step
    // register correspondence for symbolic r1, r2
    assert_eq!(
        eval("(loop ((a 10) (b 0)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "10"
    );
}

#[test]
fn test_proven_minsky_add_7_3() {
    assert_eq!(
        eval("(loop ((a 7) (b 3)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "10"
    );
}

// ════════════════════════════════════════════════════════════════
// Part 2: Iterative Computation — RecurIncAccum
// ════════════════════════════════════════════════════════════════
//
// F* lemma: accum_sum_0_to_4 proves sum(0..4) = 10

#[test]
fn test_proven_sum_0_to_4() {
    assert_eq!(
        eval("(loop ((i 0) (sum 0)) (if (> i 4) sum (recur (+ i 1) (+ sum i))))"),
        "10"
    );
}

#[test]
fn test_proven_sum_1_to_100() {
    // Sum 1..100 = 5050 — classic verification target
    assert_eq!(
        eval("(loop ((i 1) (sum 0)) (if (> i 100) sum (recur (+ i 1) (+ sum i))))"),
        "5050"
    );
}

// ════════════════════════════════════════════════════════════════
// Part 3: Homoiconicity — Programs as Data
// ════════════════════════════════════════════════════════════════
//
// F* lemmas: vm_construct_and_read (ConstructTag + GetField),
//   vm_tagged_dispatch, vm_tag_test_dispatch
// Prove that Tagged values serve as the VM's native program format.

#[test]
fn test_proven_homoiconicity() {
    // F* lemmas: vm_construct_and_read (ConstructTag + GetField)
    // Data round-trip: build a list, read it back, compute with it.
    assert_eq!(eval("(car (list 42 99))"), "42");
    assert_eq!(eval("(car (cdr (list 10 20 30)))"), "20");
    assert_eq!(eval("(length (list 1 2 3 4 5))"), "5");
}

#[test]
fn test_proven_type_dispatch() {
    // F* lemma: vm_tag_test_dispatch — TagTest dispatches on type tag.
    assert_eq!(eval("(number? 41)"), "true");
    assert_eq!(eval("(number? true)"), "false");
    assert_eq!(eval("(boolean? true)"), "true");
    assert_eq!(eval("(boolean? 0)"), "false");
    assert_eq!(eval("(nil? nil)"), "true");
    assert_eq!(eval("(nil? 0)"), "false");
}

#[test]
fn test_proven_direct_add() {
    // F* lemma: direct_add — OpAdd on concrete Num values
    assert_eq!(eval("(+ 3 4)"), "7");
    assert_eq!(eval("(- 10 3)"), "7");
    assert_eq!(eval("(* 6 7)"), "42");
}

// ════════════════════════════════════════════════════════════════
// Part 4: Forward Simulation — Symbolic Correctness
// ════════════════════════════════════════════════════════════════
//
// F* lemmas: sim_step0-sim_step5, sim_two_steps, sim_four_steps
// vm_minsky_halt: ∀r2. VM produces r2 when r1=0
// vm_minsky_iteration: ∀r1>0, r2. 4 steps → slots=[r1-1, r2]

#[test]
fn test_proven_halt_path() {
    // vm_minsky_halt: when r1=0, VM halts with result=r2
    assert_eq!(
        eval("(loop ((a 0) (b 0)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "0"
    );
    assert_eq!(
        eval("(loop ((a 0) (b 999)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "999"
    );
    assert_eq!(
        eval("(loop ((a 0) (b -5)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "-5"
    );
}

#[test]
fn test_proven_iteration_steps() {
    // vm_minsky_iteration: each iteration decrements a by 1
    for (a, b) in [(0, 0), (1, 0), (5, 3), (20, 0), (100, 50)] {
        let expected = a + b;
        let code = format!(
            "(loop ((a {}) (b {})) (if (= a 0) b (recur (- a 1) (+ b 1))))",
            a, b
        );
        assert_eq!(
            eval(&code),
            expected.to_string(),
            "minsky_add({}, {}) should be {}",
            a,
            b,
            expected
        );
    }
}

// ════════════════════════════════════════════════════════════════
// Part 5: Stated Theorems
// ════════════════════════════════════════════════════════════════
//
// F* theorem: turing_completeness
// F* theorem: minsky_correctness_theorem

#[test]
fn test_proven_turing_completeness() {
    // Full pipeline: Scheme source → compiler → bytecode VM → result
    // matching the F* proven values
    assert_eq!(
        eval("(loop ((a 3) (b 4)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "7"
    );
    assert_eq!(
        eval("(loop ((a 0) (b 5)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "5"
    );
    assert_eq!(
        eval("(loop ((a 1) (b 1)) (if (= a 0) b (recur (- a 1) (+ b 1))))"),
        "2"
    );
}

#[test]
fn test_proven_multiplication() {
    // 3 × 4 = 12 via repeated addition — universality beyond add
    assert_eq!(
        eval("(loop ((a 3) (b 0)) (if (= a 0) b (recur (- a 1) (+ b 4))))"),
        "12"
    );
}

#[test]
fn test_proven_exponentiation() {
    // 2^10 = 1024 via repeated multiplication
    // acc starts at 1, multiplied by 2 ten times
    assert_eq!(
        eval("(loop ((n 10) (acc 1)) (if (= n 0) acc (recur (- n 1) (* acc 2))))"),
        "1024"
    );
}
