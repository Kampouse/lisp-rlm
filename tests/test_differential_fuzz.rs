//! Differential fuzzing test for the lisp-rlm bytecode loop VM.
//!
//! Compares the Rust VM (run_compiled_loop) against an independent spec interpreter
//! that mirrors the F* formal model (verification/semantics/lisp_ir/LispIR.ClosureVM.fst).
//!
//! Architecture:
//!   1. SpecVm — a pure, standalone bytecode interpreter written from the F* spec
//!   2. Rust VM — the actual run_compiled_loop via make_test_compiled_loop/run_compiled_loop_test
//!   3. Random bytecode programs are generated, run through both, and results compared
//!   4. Known-good programs from the F* verification serve as regression tests

mod fuzz_common;
use fuzz_common::*;

#[test]
fn test_regression_direct_add() {
    // PushI64(3), PushI64(4), Add, Return → 7
    let code = vec![Op::PushI64(3), Op::PushI64(4), Op::Add, Op::Return];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Num(7)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(7)));
}

#[test]
fn test_regression_minsky_addition() {
    // Minsky machine addition: slots[0]=3, slots[1]=4 → result should be 7
    // Program:
    //   0: JumpIfSlotEqImm(0, 0, 6)  -- if x == 0, jump to exit (pc=6)
    //   1: SlotSubImm(0, 1)           -- push x-1
    //   2: StoreAndLoadSlot(0)        -- x = x-1, push x-1
    //   3: SlotAddImm(1, 1)           -- push y+1
    //   4: StoreAndLoadSlot(1)        -- y = y+1, push y+1
    //   5: Recur(2)                   -- recur with [x-1, y+1]
    //   6: ReturnSlot(1)              -- return y
    let code = vec![
        Op::JumpIfSlotEqImm(0, 0, 6),
        Op::SlotSubImm(0, 1),
        Op::StoreAndLoadSlot(0),
        Op::SlotAddImm(1, 1),
        Op::StoreAndLoadSlot(1),
        Op::Recur(2),
        Op::ReturnSlot(1),
    ];
    let args = vec![LispVal::Num(3), LispVal::Num(4)];

    let cl = make_test_compiled_lambda(2, 2, code.clone());
    let rust_result = run_lambda_test(&cl, &args);
    let spec_result = SpecVm::new(code, args).run(1000);

    assert_eq!(
        rust_result,
        Ok(LispVal::Num(7)),
        "Rust VM minsky addition failed"
    );
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(7)),
        "Spec VM minsky addition failed"
    );
}

#[test]
fn test_regression_minsky_addition_zero() {
    // Edge case: x=0, y=5 → result should be 5 (no iterations)
    let code = vec![
        Op::JumpIfSlotEqImm(0, 0, 6),
        Op::SlotSubImm(0, 1),
        Op::StoreAndLoadSlot(0),
        Op::SlotAddImm(1, 1),
        Op::StoreAndLoadSlot(1),
        Op::Recur(2),
        Op::ReturnSlot(1),
    ];
    let args = vec![LispVal::Num(0), LispVal::Num(5)];

    let cl = make_test_compiled_lambda(2, 2, code.clone());
    let rust_result = run_lambda_test(&cl, &args);
    let spec_result = SpecVm::new(code, args).run(1000);

    assert_eq!(rust_result, Ok(LispVal::Num(5)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(5)));
}

#[test]
fn test_regression_sum_zero_to_n() {
    // Sum 0..4: RecurIncAccum(counter=0, accum=1, step=1, limit=5, exit=1)
    //   pc=0: RecurIncAccum(0, 1, 1, 5, 1)  -- if slots[0]>=5 jump to 1, else accum+=counter, counter+=1, pc=0
    //   pc=1: PushI64(0)                      -- dummy (not reached via exit path... actually ReturnSlot is better)
    // Wait, let me re-think. RecurIncAccum exits by jumping to exit_addr.
    // We need: pc=0: RecurIncAccum(0, 1, 1, 5, 1), pc=1: ReturnSlot(1)
    // With slots=[0, 0]: counter starts at 0, accum starts at 0
    // Iteration 0: counter=0 < 5, accum = 0+0=0, counter=0+1=1, pc=0
    // Iteration 1: counter=1 < 5, accum = 0+1=1, counter=1+1=2, pc=0
    // Iteration 2: counter=2 < 5, accum = 1+2=3, counter=2+1=3, pc=0
    // Iteration 3: counter=3 < 5, accum = 3+3=6, counter=3+1=4, pc=0
    // Iteration 4: counter=4 < 5, accum = 6+4=10, counter=4+1=5, pc=0
    // Iteration 5: counter=5 >= 5, pc=1
    // pc=1: ReturnSlot(1) → return 10
    let code = vec![Op::RecurIncAccum(0, 1, 1, 5, 1), Op::ReturnSlot(1)];
    let args = vec![LispVal::Num(0), LispVal::Num(0)];

    let cl = make_test_compiled_lambda(2, 2, code.clone());
    let rust_result = run_lambda_test(&cl, &args);
    let spec_result = SpecVm::new(code, args).run(1000);

    assert_eq!(rust_result, Ok(LispVal::Num(10)), "Rust VM sum failed");
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(10)),
        "Spec VM sum failed"
    );
}

#[test]
fn test_regression_arithmetic_sequence() {
    // Test a sequence of arithmetic ops
    // PushI64(10), PushI64(3), Sub, PushI64(2), Mul → stack: [14]
    // Return → 14
    let code = vec![
        Op::PushI64(10),
        Op::PushI64(3),
        Op::Sub,
        Op::PushI64(2),
        Op::Mul,
        Op::Return,
    ];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Num(14)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(14)));
}

#[test]
fn test_regression_comparison_and_branch() {
    // Test conditional: if slots[0] > 0, return slots[0], else return 42
    //   0: LoadSlot(0)
    //   1: PushI64(0)
    //   2: Gt          -- push (slots[0] > 0)
    //   3: JumpIfFalse(6) -- if false, jump to 6
    //   4: LoadSlot(0)
    //   5: Return
    //   6: PushI64(42)
    //   7: Return
    let code = vec![
        Op::LoadSlot(0),
        Op::PushI64(0),
        Op::Gt,
        Op::JumpIfFalse(6),
        Op::LoadSlot(0),
        Op::Return,
        Op::PushI64(42),
        Op::Return,
    ];

    // Test with positive value
    let slots_pos = vec![LispVal::Num(5)];
    let cl = make_test_compiled_loop(slots_pos.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    assert_eq!(rust_result, Ok(LispVal::Num(5)), "positive branch (rust)");

    let spec_result = SpecVm::new(code.clone(), slots_pos).run(100);
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(5)),
        "positive branch (spec)"
    );

    // Test with zero value
    let slots_zero = vec![LispVal::Num(0)];
    let cl = make_test_compiled_loop(slots_zero.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    assert_eq!(rust_result, Ok(LispVal::Num(42)), "zero branch (rust)");

    let spec_result = SpecVm::new(code.clone(), slots_zero).run(100);
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(42)),
        "zero branch (spec)"
    );

    // Test with negative value
    let slots_neg = vec![LispVal::Num(-3)];
    let cl = make_test_compiled_loop(slots_neg.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    assert_eq!(rust_result, Ok(LispVal::Num(42)), "negative branch (rust)");

    let spec_result = SpecVm::new(code.clone(), slots_neg).run(100);
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(42)),
        "negative branch (spec)"
    );
}

#[test]
fn test_regression_division_by_zero() {
    // PushI64(10), PushI64(0), Div → error
    let code = vec![Op::PushI64(10), Op::PushI64(0), Op::Div, Op::Return];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert!(rust_result.is_err(), "Rust should error on div-by-zero");
    assert!(
        matches!(spec_result, SpecResult::Error(_)),
        "Spec should error on div-by-zero"
    );
}

#[test]
fn test_regression_modulo() {
    // 17 mod 5 = 2
    let code = vec![Op::PushI64(17), Op::PushI64(5), Op::Mod, Op::Return];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Num(2)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(2)));
}

#[test]
fn test_regression_slot_immediate_ops() {
    // Test fused slot+immediate ops
    //   0: SlotAddImm(0, 10)  → push slots[0] + 10
    //   1: StoreSlot(0)       → slots[0] = result
    //   2: SlotSubImm(0, 3)   → push slots[0] - 3
    //   3: Return
    // With slots=[5]: push 15, store, push 12, return 12
    let code = vec![
        Op::SlotAddImm(0, 10),
        Op::StoreSlot(0),
        Op::SlotSubImm(0, 3),
        Op::Return,
    ];
    let slots = vec![LispVal::Num(5)];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Num(12)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(12)));
}

#[test]
fn test_regression_slot_sub_imm_no_writeback() {
    // Verify SlotSubImm does NOT write back to slot (matches Rust behavior)
    //   0: SlotSubImm(0, 1)   → push slots[0] - 1 (DON'T write back)
    //   1: LoadSlot(0)        → push slots[0] (should still be original value)
    //   2: Sub                → push (original - 1) - original = -1
    //   3: Return
    // With slots=[5]: push 4, push 5, sub → -1, return -1
    let code = vec![Op::SlotSubImm(0, 1), Op::LoadSlot(0), Op::Sub, Op::Return];
    let slots = vec![LispVal::Num(5)];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(
        rust_result,
        Ok(LispVal::Num(-1)),
        "SlotSubImm should not write back (rust)"
    );
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(-1)),
        "SlotSubImm should not write back (spec)"
    );
}

#[test]
fn test_regression_equality() {
    // Test Eq with same values
    let code = vec![Op::PushI64(42), Op::PushI64(42), Op::Eq, Op::Return];
    let slots = vec![];
    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);
    assert_eq!(rust_result, Ok(LispVal::Bool(true)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Bool(true)));

    // Test Eq with different values
    let code = vec![Op::PushI64(1), Op::PushI64(2), Op::Eq, Op::Return];
    let slots = vec![];
    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);
    assert_eq!(rust_result, Ok(LispVal::Bool(false)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Bool(false)));
}

#[test]
fn test_regression_empty_stack_pop_coercion() {
    // Anchor-aligned (2026-08-26): pop-from-empty yields Nil, and generic
    // arith on non-numeric operands HARD-ERRORS (wasm trap semantics —
    // GAPS.md: wasm is the semantic anchor). Old lenient 0-coercion retired.
    //   0: Add  → pop Nil, pop Nil → type error: add expects numbers
    //   1: Return (unreached)
    let code = vec![Op::Add, Op::Return];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert!(matches!(rust_result, Err(ref e) if e.contains("type error: add")));
    assert!(matches!(
        spec_result,
        SpecResult::Error(ref e) if e.contains("type error: add")
    ));
}

#[test]
fn test_regression_float_coercion() {
    // Float propagation: when either operand is Float, result is Float
    //   0: PushFloat(3.7)
    //   1: PushI64(2)
    //   2: Add → 3.7 + 2.0 = 5.7 (Float propagation)
    //   3: Return
    let code = vec![Op::PushFloat(3.7), Op::PushI64(2), Op::Add, Op::Return];
    let slots = vec![];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Float(5.7)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Float(5.7)));
}

#[test]
fn test_regression_nested_recur() {
    // Fibonacci-like: compute fib(5) using recur
    // slots[0] = n (counter), slots[1] = a, slots[2] = b
    // if n == 0, return a; else recur(n-1, b, a+b)
    //   0: JumpIfSlotEqImm(0, 0, 7)  -- if n==0, exit
    //   1: SlotSubImm(0, 1)          -- push n-1
    //   2: StoreAndLoadSlot(0)       -- n = n-1
    //   3: LoadSlot(2)               -- push b (new a)
    //   4: LoadSlot(1)               -- push a
    //   5: LoadSlot(2)               -- push b
    //   6: Add                       -- push a+b (new b)
    //   7: Recur(3)                  -- recur(n-1, b, a+b)
    // Wait, that's wrong. Recur pops 3 values. Let me re-think.
    // Stack before Recur: [n-1, b, a+b]
    // Recur(3) pops 3 in reverse: slots[2]=a+b, slots[1]=b, slots[0]=n-1
    // Then jumps to pc=0.
    // Actually wait, I need more slots. Let me use a simpler approach:
    //   0: JumpIfSlotEqImm(0, 0, 8)
    //   1: SlotSubImm(0, 1)
    //   2: StoreAndLoadSlot(0)        -- stack: [n-1]
    //   3: LoadSlot(1)                -- stack: [n-1, a]  (this is new b)
    //   4: LoadSlot(2)                -- stack: [n-1, a, b]
    //   5: Add                        -- stack: [n-1, a+b] (new b)
    //   6: LoadSlot(1)                -- stack: [n-1, a+b, a] (new a = old b)
    //   Wait this is getting confusing. Let me use a clean approach.
    //
    // Recur(3) pops: slots[2] = top, slots[1] = second, slots[0] = third
    // We want: slots[0] = n-1, slots[1] = b, slots[2] = a+b
    // So we need to push in order: a+b, b, n-1
    //   0: JumpIfSlotEqImm(0, 0, 8)  -- exit at pc=8
    //   1: LoadSlot(1)                -- push a
    //   2: LoadSlot(2)                -- push b
    //   3: Add                        -- push a+b
    //   4: LoadSlot(2)                -- push b (for new a)
    //   5: SlotSubImm(0, 1)          -- push n-1
    //   6: Recur(3)                   -- slots[2]=n-1, slots[1]=b, slots[0]=a+b
    // Wait, Recur(3): for i in (0..3).rev() { slots[i] = stack.pop() }
    //   slots[2] = pop() = n-1
    //   slots[1] = pop() = b
    //   slots[0] = pop() = a+b
    // So slots = [a+b, b, n-1]. That's wrong. We want [n-1, b, a+b].
    // We need to push in reverse order: n-1, b, a+b
    // Then pop order: slots[2]=a+b, slots[1]=b, slots[0]=n-1. Perfect!

    // Let me redo:
    //   0: JumpIfSlotEqImm(0, 0, 8)  -- if n==0, jump to return
    //   1: SlotSubImm(0, 1)          -- push n-1
    //   2: LoadSlot(2)                -- push b
    //   3: LoadSlot(1)                -- push a
    //   4: Add                        -- push a+b
    //   5: Recur(3)                   -- pop: slots[2]=a+b, slots[1]=b, slots[0]=n-1
    //   Wait, that gives us n-1 on top. We push n-1 first, then b, then a, then a+b.
    //   Stack: [n-1, b, a, a+b]
    //   Recur(3): slots[2]=a+b, slots[1]=a, slots[0]=b
    //   That's wrong too. We want [n-1, b, a+b].
    //
    //   Correct push order: n-1, b, a+b
    //   1: SlotSubImm(0, 1)          -- push n-1         stack: [n-1]
    //   2: LoadSlot(2)                -- push b           stack: [n-1, b]
    //   3: LoadSlot(1)                -- push a           stack: [n-1, b, a]
    //   4: LoadSlot(2)                -- push b           stack: [n-1, b, a, b]
    //   5: Add                        -- push a+b         stack: [n-1, b, a+b]
    //   6: Recur(3)                   -- slots[2]=a+b, slots[1]=b, slots[0]=n-1 ✓
    //   7: (unreachable, Recur jumps to 0)
    //   8: ReturnSlot(1)              -- return b (which is fib(n))

    let code = vec![
        Op::JumpIfSlotEqImm(0, 0, 8), // 0: if n==0, exit
        Op::SlotSubImm(0, 1),         // 1: push n-1
        Op::LoadSlot(2),              // 2: push b
        Op::LoadSlot(1),              // 3: push a
        Op::LoadSlot(2),              // 4: push b
        Op::Add,                      // 5: push a+b
        Op::Recur(3),                 // 6: recur(n-1, b, a+b)
        Op::PushNil,                  // 7: (padding, not reached)
        Op::ReturnSlot(1),            // 8: return b
    ];
    // fib(6) = 8: slots = [n=6, a=0, b=1]
    // Iterations: n=6,a=0,b=1 → n=5,a=1,b=1 → n=4,a=1,b=2 → n=3,a=2,b=3 → n=2,a=3,b=5 → n=1,a=5,b=8 → n=0 → return 8
    let args = vec![LispVal::Num(6), LispVal::Num(0), LispVal::Num(1)];

    let cl = make_test_compiled_lambda(3, 3, code.clone());
    let rust_result = run_lambda_test(&cl, &args);
    let spec_result = SpecVm::new(code, args).run(1000);

    assert_eq!(rust_result, Ok(LispVal::Num(8)), "Rust fib(6) failed");
    assert_eq!(
        spec_result,
        SpecResult::Value(LispVal::Num(8)),
        "Spec fib(6) failed"
    );
}

#[test]
fn test_regression_store_and_load_slot() {
    // StoreAndLoadSlot: pop, store to slot, push slot value back
    //   0: PushI64(42)
    //   1: StoreAndLoadSlot(0)  → slots[0]=42, push 42
    //   2: LoadSlot(0)          → push 42
    //   3: Add                  → 42+42=84
    //   4: Return
    let code = vec![
        Op::PushI64(42),
        Op::StoreAndLoadSlot(0),
        Op::LoadSlot(0),
        Op::Add,
        Op::Return,
    ];
    let slots = vec![LispVal::Nil];

    let cl = make_test_compiled_loop(slots.clone(), code.clone(), vec![]);
    let rust_result = run_compiled_loop_test(&cl);
    let spec_result = SpecVm::new(code, slots).run(100);

    assert_eq!(rust_result, Ok(LispVal::Num(84)));
    assert_eq!(spec_result, SpecResult::Value(LispVal::Num(84)));
}

#[test]
fn test_regression_jump_if_slot_ops() {
    // Test JumpIfSlotLtImm and JumpIfSlotGeImm
    //   0: JumpIfSlotLtImm(0, 5, 3)  -- if slots[0] < 5, jump to 3
    //   1: PushI64(100)               -- not taken path
    //   2: Return
    //   3: PushI64(200)               -- taken path
    //   4: Return
    let code = vec![
        Op::JumpIfSlotLtImm(0, 5, 3),
        Op::PushI64(100),
        Op::Return,
        Op::PushI64(200),
        Op::Return,
    ];

    // slots[0] = 3 < 5 → jump to 3 → return 200
    let slots_taken = vec![LispVal::Num(3)];
    let cl = make_test_compiled_loop(slots_taken.clone(), code.clone(), vec![]);
    assert_eq!(run_compiled_loop_test(&cl), Ok(LispVal::Num(200)));
    assert_eq!(
        SpecVm::new(code.clone(), slots_taken).run(100),
        SpecResult::Value(LispVal::Num(200))
    );

    // slots[0] = 7 >= 5 → fall through → return 100
    let slots_fall = vec![LispVal::Num(7)];
    let cl = make_test_compiled_loop(slots_fall.clone(), code.clone(), vec![]);
    assert_eq!(run_compiled_loop_test(&cl), Ok(LispVal::Num(100)));
    assert_eq!(
        SpecVm::new(code.clone(), slots_fall).run(100),
        SpecResult::Value(LispVal::Num(100))
    );
}

#[test]
fn test_regression_truthiness() {
    // Test is_truthy: only Nil and Bool(false) are falsy
    //   0: PushBool(true), JumpIfTrue(2), PushI64(1), Return, PushI64(2), Return
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code.clone());
    assert_eq!(run_lambda_test(&cl, &[]), Ok(LispVal::Num(2))); // true → jump to 3 → return 2

    let code = vec![
        Op::PushBool(false),
        Op::JumpIfTrue(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code.clone());
    assert_eq!(run_lambda_test(&cl, &[]), Ok(LispVal::Num(1))); // false → fall through → return 1

    let code = vec![
        Op::PushNil,
        Op::JumpIfTrue(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code.clone());
    assert_eq!(run_lambda_test(&cl, &[]), Ok(LispVal::Num(1))); // nil → fall through → return 1

    // JumpIfFalse version
    let code = vec![
        Op::PushNil,
        Op::JumpIfFalse(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code.clone());
    assert_eq!(run_lambda_test(&cl, &[]), Ok(LispVal::Num(2))); // nil → jump to 3 → return 2
}

// ---------------------------------------------------------------------------
// Differential fuzz tests — random bytecode programs
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_short_programs() {
    // Generate many short programs and compare spec vs Rust
    // Use a larger stack because MakeList/DictSet can create deep LispVal trees
    // whose recursive Drop overflows the default 2MB test thread stack.
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(42); // deterministic seed
            let mut mismatches = 0;
            let total = 200;

            for i in 0..total {
                let num_slots = rng.next_usize(4); // 0-3 slots
                let code_len = rng.next_usize(8) + 2; // 2-9 instructions
                let code = generate_random_program(&mut rng, num_slots, code_len);

                // Generate random initial slot values
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }
                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 1000) {
                    mismatches += 1;
                    fuzz_common::report_mismatch("", i, &desc, &code, &init_slots, 1000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches between spec VM and Rust VM in {} programs",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

#[test]
fn test_differential_fuzz_medium_programs() {
    // Medium-length programs with loops (more likely to exercise Recur/RecurIncAccum)
    // Use 8MB stack to avoid stack overflow from deep LispVal Drop (MakeList/DictSet)
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(12345);
            let mut mismatches = 0;
            let total = 8_000;

            for i in 0..total {
                eprintln!("MED {}", i);
                let num_slots = rng.next_usize(4) + 1; // 1-4 slots (at least 1 for loops)
                let code_len = rng.next_usize(15) + 5; // 5-19 instructions
                let code = generate_random_program(&mut rng, num_slots, code_len);

                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
                    mismatches += 1;
                    eprintln!("MISMATCH #{}: {}", i, desc);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches between spec VM and Rust VM in {} programs",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

#[test]
fn test_differential_fuzz_slot_imm_ops() {
    // Focused fuzz on slot+immediate ops (SlotAddImm, SlotSubImm, etc.)
    let mut rng = Rng::new(999);
    let mut mismatches = 0;
    let total = 3000;

    // Only use slot-immediate ops
    let slot_imm_ops = &[
        FuzzOp::LoadSlot,
        FuzzOp::StoreSlot,
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
        FuzzOp::StoreAndLoadSlot,
        FuzzOp::ReturnSlot,
        FuzzOp::Recur,
        FuzzOp::RecurIncAccum,
        FuzzOp::Return,
        FuzzOp::Add,
        FuzzOp::Sub,
        FuzzOp::PushI64,
    ];

    for i in 0..total {
        let num_slots = rng.next_usize(3) + 2; // 2-4 slots
        let code_len = rng.next_usize(10) + 3;

        let mut code = Vec::with_capacity(code_len);
        for _ in 0..code_len {
            let fop_idx = rng.next_usize(slot_imm_ops.len());
            let fop = slot_imm_ops[fop_idx];
            code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
        }

        // Ensure termination
        let has_terminal = code
            .iter()
            .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
        if !has_terminal {
            code.push(Op::Return);
        }

        let mut init_slots = Vec::with_capacity(num_slots);
        for _ in 0..num_slots {
            init_slots.push(LispVal::Num(rng.next_range(-10, 10)));
        }

        if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
            mismatches += 1;
            eprintln!("SLOT_IMM MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("SLOT_IMM", i, &desc, &code, &init_slots, 5000);
        }
    }

    assert_eq!(
        mismatches, 0,
        "Found {} mismatches in slot-imm fuzz ({} programs)",
        mismatches, total
    );
}

#[test]
fn test_differential_fuzz_edge_cases() {
    // Focused fuzz on edge cases: empty stack, non-numeric values, etc.
    // Use 8MB stack to avoid stack overflow from deep LispVal Drop (MakeList/DictSet)
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(7777);
            let mut mismatches = 0;
            let total = 2000;

            for i in 0..total {
                let num_slots = rng.next_usize(2); // 0-1 slots
                let code_len = rng.next_usize(6) + 1;

                let code = generate_random_program(&mut rng, num_slots, code_len);

                // Use non-numeric slot values for edge case testing
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 1000) {
                    mismatches += 1;
                    eprintln!("EDGE MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("EDGE", i, &desc, &code, &init_slots, 1000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in edge case fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

#[test]
fn test_differential_fuzz_recur_patterns() {
    // Focused fuzz on Recur patterns (the most complex control flow)
    // Use 8MB stack to avoid stack overflow from deep LispVal Drop (MakeList/DictSet)
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(314159);
            let mut mismatches = 0;
            let total = 200;

            for i in 0..total {
                let num_slots = rng.next_usize(3) + 1; // 1-3 slots
                let code_len = rng.next_usize(8) + 3;

                let mut code = Vec::with_capacity(code_len);

                // Generate programs that are likely to use Recur or RecurIncAccum
                for j in 0..code_len {
                    let fop = if j == 0 {
                        // First instruction: either a jump-if or RecurIncAccum
                        if rng.next_bool() {
                            FuzzOp::RecurIncAccum
                        } else {
                            FuzzOp::JumpIfSlotEqImm
                        }
                    } else if j == code_len - 1 {
                        // Last instruction: Return or ReturnSlot
                        if rng.next_bool() {
                            FuzzOp::ReturnSlot
                        } else {
                            FuzzOp::Return
                        }
                    } else {
                        // Middle: any op
                        FUZZ_OPS[rng.next_usize(FUZZ_OPS.len())]
                    };
                    code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
                }

                // Use small numeric slot values for bounded loops
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(LispVal::Num(rng.next_range(0, 5)));
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 10000) {
                    mismatches += 1;
                    eprintln!("RECUR MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("RECUR", i, &desc, &code, &init_slots, 10000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in recur fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Step-level differential test — compare single-step execution
// ---------------------------------------------------------------------------

#[test]
fn test_step_level_differential() {
    /// Extract the state from the Rust VM after running one step.
    /// We do this by constructing a program that stops after one step.
    /// Since run_compiled_loop runs to completion, we compare full execution
    /// of single-step programs instead.
    ///
    /// For each opcode, we create a program that:
    ///   1. Sets up initial state via init_slots
    ///   2. Executes the opcode under test
    ///   3. Captures the result (stack top + slot values) via StoreSlot+Return
    ///
    /// Then compare spec_step vs rust_step for that opcode.
    let test_cases: Vec<(&str, Vec<Op>, Vec<LispVal>, Vec<LispVal>)> = vec![
        // (name, code, init_slots, expected_final_slots)
        // LoadSlot
        (
            "LoadSlot",
            vec![Op::LoadSlot(0), Op::Return],
            vec![LispVal::Num(42)],
            vec![LispVal::Num(42)],
        ),
        // StoreSlot
        (
            "StoreSlot",
            vec![Op::PushI64(99), Op::StoreSlot(0), Op::ReturnSlot(0)],
            vec![LispVal::Num(0)],
            vec![LispVal::Num(99)],
        ),
        // Add
        (
            "Add",
            vec![Op::PushI64(3), Op::PushI64(4), Op::Add, Op::Return],
            vec![],
            vec![],
        ),
        // Sub
        (
            "Sub",
            vec![Op::PushI64(10), Op::PushI64(3), Op::Sub, Op::Return],
            vec![],
            vec![],
        ),
        // Mul
        (
            "Mul",
            vec![Op::PushI64(6), Op::PushI64(7), Op::Mul, Op::Return],
            vec![],
            vec![],
        ),
        // Div
        (
            "Div",
            vec![Op::PushI64(20), Op::PushI64(4), Op::Div, Op::Return],
            vec![],
            vec![],
        ),
        // Mod
        (
            "Mod",
            vec![Op::PushI64(17), Op::PushI64(5), Op::Mod, Op::Return],
            vec![],
            vec![],
        ),
        // Eq true
        (
            "Eq_true",
            vec![Op::PushI64(5), Op::PushI64(5), Op::Eq, Op::Return],
            vec![],
            vec![],
        ),
        // Eq false
        (
            "Eq_false",
            vec![Op::PushI64(5), Op::PushI64(6), Op::Eq, Op::Return],
            vec![],
            vec![],
        ),
        // Lt true
        (
            "Lt_true",
            vec![Op::PushI64(3), Op::PushI64(5), Op::Lt, Op::Return],
            vec![],
            vec![],
        ),
        // Gt true
        (
            "Gt_true",
            vec![Op::PushI64(5), Op::PushI64(3), Op::Gt, Op::Return],
            vec![],
            vec![],
        ),
        // Dup
        (
            "Dup",
            vec![Op::PushI64(7), Op::Dup, Op::Add, Op::Return],
            vec![],
            vec![],
        ),
        // Pop
        (
            "Pop",
            vec![Op::PushI64(1), Op::PushI64(2), Op::Pop, Op::Return],
            vec![],
            vec![],
        ),
        // JumpIfTrue taken
        (
            "JumpIfTrue_taken",
            vec![
                Op::PushI64(1),
                Op::JumpIfTrue(3),
                Op::PushI64(0),
                Op::Return,
                Op::PushI64(99),
                Op::Return,
            ],
            vec![],
            vec![],
        ),
        // JumpIfFalse taken
        (
            "JumpIfFalse_taken",
            vec![
                Op::PushNil,
                Op::JumpIfFalse(3),
                Op::PushI64(0),
                Op::Return,
                Op::PushI64(99),
                Op::Return,
            ],
            vec![],
            vec![],
        ),
        // SlotAddImm (no writeback)
        (
            "SlotAddImm",
            vec![Op::SlotAddImm(0, 10), Op::Return],
            vec![LispVal::Num(5)],
            vec![LispVal::Num(5)],
        ), // slot should NOT change
        // SlotSubImm (no writeback)
        (
            "SlotSubImm",
            vec![Op::SlotSubImm(0, 3), Op::Return],
            vec![LispVal::Num(10)],
            vec![LispVal::Num(10)],
        ), // slot should NOT change
        // StoreAndLoadSlot
        (
            "StoreAndLoadSlot",
            vec![Op::PushI64(77), Op::StoreAndLoadSlot(0), Op::Return],
            vec![LispVal::Nil],
            vec![LispVal::Num(77)],
        ),
        // MakeList
        (
            "MakeList",
            vec![
                Op::PushI64(1),
                Op::PushI64(2),
                Op::PushI64(3),
                Op::MakeList(3),
                Op::Return,
            ],
            vec![],
            vec![],
        ),
    ];

    for (name, code, init_slots, _expected_slots) in &test_cases {
        let cl = make_test_compiled_lambda(init_slots.len(), init_slots.len(), code.clone());
        let rust_result = run_lambda_test(&cl, init_slots);
        let spec_result = SpecVm::new(code.clone(), init_slots.clone()).run(100);

        match (&rust_result, &spec_result) {
            (Ok(rv), SpecResult::Value(sv)) => {
                assert_eq!(
                    rv, sv,
                    "Step-level mismatch for '{}': rust={:?} spec={:?}",
                    name, rv, sv
                );
            }
            (Err(re), SpecResult::Error(_)) => {
                // Both errored — ok
            }
            _ => {
                panic!(
                    "Step-level outcome mismatch for '{}': rust={:?} spec={:?}",
                    name, rust_result, spec_result
                );
            }
        }
    }
}

#[test]
fn test_lambda_basic_push_return() {
    let code = vec![Op::PushI64(42), Op::Return];
    let cl = make_test_compiled_lambda(0, 0, code);
    assert_eq!(run_lambda_test(&cl, &[]), Ok(LispVal::Num(42)));
}

#[test]
fn test_lambda_pushbool() {
    // PushBool + Return — just return the bool
    let code = vec![Op::PushBool(true), Op::Return];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("pushbool result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Bool(true)));
}

#[test]
fn test_lambda_jump_basic() {
    // Jump(2) should skip PushI64(1) and land on PushI64(2)
    let code = vec![Op::Jump(2), Op::PushI64(1), Op::PushI64(2), Op::Return];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("jump result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(2)));
}

#[test]
fn test_lambda_jumpiftrue_addr1() {
    // JumpIfTrue(1) — skip nothing, land on PushI64(1)
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(1),
        Op::PushI64(99),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("addr1 result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(99)));
}

#[test]
fn test_lambda_jumpiftrue_addr2() {
    // JumpIfTrue(2) — skip PushI64(1), land on PushI64(99)
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(2),
        Op::PushI64(1),
        Op::PushI64(99),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("addr2 result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(99)));
}

#[test]
fn test_lambda_jumpiftrue_addr3() {
    // JumpIfTrue(3) — skip 2 instrs, land on PushI64(99)
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(3),
        Op::PushI64(1),
        Op::PushI64(2),
        Op::PushI64(99),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("addr3 result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(99)));
}

#[test]
fn test_lambda_truthiness_exact() {
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    eprintln!("code len: {}", code.len());
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("exact result: {:?}", r);
}

#[test]
fn test_lambda_two_returns() {
    let code = vec![Op::PushI64(42), Op::Return, Op::PushI64(99), Op::Return];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("two returns result: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(42)));
}

#[test]
fn test_lambda_jump_past_return() {
    // Jump over a Return instruction
    let code = vec![Op::Jump(2), Op::Return, Op::PushI64(42), Op::Return];
    let cl = make_test_compiled_lambda(0, 0, code);
    let r = run_lambda_test(&cl, &[]);
    eprintln!("jump past return: {:?}", r);
    assert_eq!(r, Ok(LispVal::Num(42)));
}

#[test]
fn test_lambda_jit_minimal() {
    // Minimal JumpIfTrue(3) that should return 2
    let code = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(3),
        Op::PushI64(1),
        Op::Return, // pc=3 in original test
        Op::PushI64(2),
        Op::Return,
    ];
    let cl = make_test_compiled_lambda(0, 0, code.clone());
    let r = run_lambda_test(&cl, &[]);
    eprintln!("jit minimal: {:?}", r);

    // Try with addr=4 instead
    let code2 = vec![
        Op::PushBool(true),
        Op::JumpIfTrue(4),
        Op::PushI64(1),
        Op::Return,
        Op::PushI64(2),
        Op::Return,
    ];
    let cl2 = make_test_compiled_lambda(0, 0, code2);
    let r2 = run_lambda_test(&cl2, &[]);
    eprintln!("jit addr4: {:?}", r2);
}

// ---------------------------------------------------------------------------
// Targeted fuzz: tagged values (ConstructTag + TagTest + GetField)
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_tagged_values() {
    // Fuzz sequences that build tagged values, test them, and extract fields
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(271828);
            let mut mismatches = 0;
            let total = 500;

            for i in 0..total {
                let num_slots = rng.next_usize(3) + 1; // 1-3 slots
                let code_len = rng.next_usize(10) + 4; // 4-13 instructions

                let mut code = Vec::with_capacity(code_len);
                for j in 0..code_len {
                    let fop = if j == code_len - 1 {
                        if rng.next_bool() {
                            FuzzOp::ReturnSlot
                        } else {
                            FuzzOp::Return
                        }
                    } else {
                        // Weighted: 40% tag ops, 60% any op
                        let r = rng.next_usize(10);
                        if r < 3 {
                            FuzzOp::ConstructTag
                        } else if r < 5 {
                            FuzzOp::TagTest
                        } else if r < 7 {
                            FuzzOp::GetField
                        } else {
                            FUZZ_OPS[rng.next_usize(FUZZ_OPS.len())]
                        }
                    };
                    code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
                }

                let has_terminal = code
                    .iter()
                    .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
                if !has_terminal {
                    code.push(Op::Return);
                }

                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 10000) {
                    mismatches += 1;
                    eprintln!("TAG MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("TAG", i, &desc, &code, &init_slots, 10000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in tagged-value fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Targeted fuzz: dict operations (DictGet + DictSet + DictMutSet + GetDefaultSlot)
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_dict_ops() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(31415);
            let mut mismatches = 0;
            let total = 500;

            for i in 0..total {
                let num_slots = rng.next_usize(4) + 2; // 2-5 slots (dict ops need slots)
                let code_len = rng.next_usize(10) + 4;

                let mut code = Vec::with_capacity(code_len);
                for j in 0..code_len {
                    let fop = if j == code_len - 1 {
                        if rng.next_bool() {
                            FuzzOp::ReturnSlot
                        } else {
                            FuzzOp::Return
                        }
                    } else {
                        // 50% dict ops, 50% any op
                        let r = rng.next_usize(8);
                        match r {
                            0 => FuzzOp::DictGet,
                            1 => FuzzOp::DictSet,
                            2 => FuzzOp::DictMutSet,
                            3 => FuzzOp::GetDefaultSlot,
                            _ => FUZZ_OPS[rng.next_usize(FUZZ_OPS.len())],
                        }
                    };
                    code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
                }

                let has_terminal = code
                    .iter()
                    .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
                if !has_terminal {
                    code.push(Op::Return);
                }

                // Initialize some slots with maps for dict ops to work with
                let mut init_slots = Vec::with_capacity(num_slots);
                for s in 0..num_slots {
                    if s == 0 || rng.next_bool() {
                        // First slot always a map, others random
                        let mut map = im::HashMap::new();
                        let n_entries = rng.next_usize(3);
                        for _ in 0..n_entries {
                            let key_len = rng.next_usize(3) + 1;
                            let key: String = (0..key_len)
                                .map(|_| (b'a' + rng.next_usize(3) as u8) as char)
                                .collect();
                            map.insert(key, rng.next_lisp_val());
                        }
                        init_slots.push(LispVal::Map(map));
                    } else {
                        init_slots.push(rng.next_lisp_val());
                    }
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 10000) {
                    mismatches += 1;
                    eprintln!("DICT MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("DICT", i, &desc, &code, &init_slots, 10000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in dict fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Targeted fuzz: float edge cases (NaN, Inf, precision, overflow on cast)
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_float_edges() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(16180);
            let mut mismatches = 0;
            let total = 1500;

            for i in 0..total {
                let code_len = rng.next_usize(8) + 3;
                let num_slots = rng.next_usize(3); // 0-2 slots

                let mut code = Vec::with_capacity(code_len);
                for j in 0..code_len {
                    let fop = if j == code_len - 1 {
                        FuzzOp::Return
                    } else {
                        // Heavy float weighting
                        let r = rng.next_usize(8);
                        match r {
                            0 => FuzzOp::PushFloat,
                            1 => FuzzOp::TypedBinOpF64,
                            2 => FuzzOp::Add, // untyped with float promotion
                            3 => FuzzOp::Sub,
                            4 => FuzzOp::Mul,
                            5 => FuzzOp::Div,
                            6 => FuzzOp::TypedBinOpI64, // float→0 coercion
                            _ => FUZZ_OPS[rng.next_usize(FUZZ_OPS.len())],
                        }
                    };
                    code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
                }

                let has_terminal = code
                    .iter()
                    .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
                if !has_terminal {
                    code.push(Op::Return);
                }

                // Init slots with boundary floats
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    if rng.next_bool() {
                        init_slots.push(LispVal::Float(rng.boundary_f64()));
                    } else {
                        init_slots.push(LispVal::Num(rng.boundary_i64()));
                    }
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 10000) {
                    mismatches += 1;
                    eprintln!("FLOAT MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("FLOAT", i, &desc, &code, &init_slots, 10000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in float edge fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Targeted fuzz: overflow and boundary arithmetic
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_overflow() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(42);
            let mut mismatches = 0;
            let total = 1500;

            for i in 0..total {
                let num_slots = rng.next_usize(3) + 1;
                let code_len = rng.next_usize(6) + 3;

                let mut code = Vec::with_capacity(code_len);
                for j in 0..code_len {
                    let fop = if j == code_len - 1 {
                        FuzzOp::Return
                    } else {
                        // Heavy arithmetic weighting
                        let r = rng.next_usize(6);
                        match r {
                            0 => FuzzOp::PushI64,
                            1 => FuzzOp::Add,
                            2 => FuzzOp::Sub,
                            3 => FuzzOp::Mul,
                            4 => FuzzOp::SlotAddImm,
                            _ => FuzzOp::SlotMulImm,
                        }
                    };
                    code.push(fuzz_op_to_op(&mut rng, fop, code_len, num_slots));
                }

                // Init slots with boundary values
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(LispVal::Num(rng.boundary_i64()));
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 10000) {
                    mismatches += 1;
                    eprintln!("OVERFLOW MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("OVERFLOW", i, &desc, &code, &init_slots, 10000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in overflow fuzz ({} programs)",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Multi-seed fuzz: run short programs with many different seeds
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_multi_seed() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let seeds: &[u64] = &[1, 42, 137, 314, 666, 1337, 3141];
            let mut mismatches = 0;
            let total_per_seed = 50;

            for &seed in seeds {
                let mut rng = Rng::new(seed);
                for i in 0..total_per_seed {
                    let num_slots = rng.next_usize(4);
                    let code_len = rng.next_usize(8) + 2;
                    let code = generate_random_program(&mut rng, num_slots, code_len);

                    let mut init_slots = Vec::with_capacity(num_slots);
                    for _ in 0..num_slots {
                        init_slots.push(rng.next_lisp_val());
                    }

                    if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
                        mismatches += 1;
                        eprintln!("SEED {} MISMATCH #{}: {}", seed, i, desc);
                    }
                }
            }

            assert_eq!(
                mismatches,
                0,
                "Found {} mismatches across {} seeds × {} programs",
                mismatches,
                seeds.len(),
                total_per_seed
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Mutation fuzzing: take known-good programs, mutate 1-2 ops, run differential
// ---------------------------------------------------------------------------

fn corpus_programs() -> Vec<(Vec<Op>, Vec<LispVal>)> {
    // Known-good (code, slots) pairs from regression tests.
    // These all pass on both VMs — mutations near them find bugs.
    vec![
        // Direct add: 3 + 4 = 7
        (
            vec![Op::PushI64(3), Op::PushI64(4), Op::Add, Op::Return],
            vec![],
        ),
        // Minsky addition: slots=[3,4], loop until x==0, y+=1
        (
            vec![
                Op::JumpIfSlotEqImm(0, 0, 6),
                Op::SlotSubImm(0, 1),
                Op::StoreAndLoadSlot(0),
                Op::LoadSlot(1),
                Op::Add,
                Op::StoreSlot(1),
                Op::ReturnSlot(1),
            ],
            vec![LispVal::Num(3), LispVal::Num(4)],
        ),
        // Sum 0..4 via RecurIncAccum
        (
            vec![Op::RecurIncAccum(0, 1, 1, 5, 1), Op::ReturnSlot(1)],
            vec![LispVal::Num(0), LispVal::Num(0)],
        ),
        // Fibonacci: slots=[6,0,1], loop
        (
            vec![
                Op::JumpIfSlotEqImm(0, 0, 8),
                Op::SlotSubImm(0, 1),
                Op::LoadSlot(2),
                Op::LoadSlot(1),
                Op::LoadSlot(2),
                Op::Add,
                Op::Recur(3),
                Op::PushNil,
                Op::ReturnSlot(1),
            ],
            vec![LispVal::Num(6), LispVal::Num(0), LispVal::Num(1)],
        ),
        // SlotSubImm no-writeback: slots=[5], push 4, push 5, sub → -1
        (
            vec![Op::SlotSubImm(0, 1), Op::LoadSlot(0), Op::Sub, Op::Return],
            vec![LispVal::Num(5)],
        ),
        // Float propagation: 3.7 + 2 = 5.7
        (
            vec![Op::PushFloat(3.7), Op::PushI64(2), Op::Add, Op::Return],
            vec![],
        ),
        // Absolute value via SlotGtImm: if x >= 5 → 100, else → 200
        (
            vec![
                Op::SlotGtImm(0, 4),
                Op::JumpIfTrue(4),
                Op::PushI64(200),
                Op::Return,
                Op::PushI64(100),
                Op::Return,
            ],
            vec![LispVal::Num(3)],
        ),
        // Truthiness: push bool, branch
        (
            vec![
                Op::PushBool(true),
                Op::JumpIfTrue(4),
                Op::PushI64(1),
                Op::Return,
                Op::PushI64(2),
                Op::Return,
            ],
            vec![],
        ),
        // Dup + Add: 7*2 = 14
        (vec![Op::PushI64(7), Op::Dup, Op::Add, Op::Return], vec![]),
        // MakeList + Eq: [1,2,3] == [1,2,3]
        (
            vec![
                Op::PushI64(1),
                Op::PushI64(2),
                Op::PushI64(3),
                Op::MakeList(3),
                Op::PushI64(1),
                Op::PushI64(2),
                Op::PushI64(3),
                Op::MakeList(3),
                Op::Eq,
                Op::Return,
            ],
            vec![],
        ),
    ]
}

fn mutate_op(rng: &mut Rng, op: &Op, num_slots: usize, max_pc: usize) -> Op {
    use FuzzOp::*;
    // 50% chance: replace entirely with a random op
    if rng.next_usize(2) == 0 {
        let idx = rng.next_usize(FUZZ_OPS.len());
        return fuzz_op_to_op(rng, FUZZ_OPS[idx].clone(), max_pc, num_slots);
    }
    // Otherwise, tweak a parameter of the existing op
    match op.clone() {
        Op::PushI64(n) => Op::PushI64(n.wrapping_add(rng.boundary_i64())),
        Op::PushFloat(f) => Op::PushFloat(f + rng.boundary_f64()),
        Op::LoadSlot(_) => Op::LoadSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots })),
        Op::StoreSlot(_) => {
            Op::StoreSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        Op::ReturnSlot(_) => {
            Op::ReturnSlot(rng.next_usize(if num_slots == 0 { 1 } else { num_slots }))
        }
        Op::Jump(_) => Op::Jump(rng.next_usize(max_pc + 1)),
        Op::JumpIfTrue(_) => Op::JumpIfTrue(rng.next_usize(max_pc + 1)),
        Op::JumpIfFalse(_) => Op::JumpIfFalse(rng.next_usize(max_pc + 1)),
        Op::SlotAddImm(s, imm) => Op::SlotAddImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            imm.wrapping_add(rng.boundary_i64()),
        ),
        Op::SlotSubImm(s, imm) => Op::SlotSubImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            imm.wrapping_add(rng.boundary_i64()),
        ),
        Op::SlotMulImm(s, imm) => Op::SlotMulImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            imm.wrapping_add(rng.boundary_i64()),
        ),
        Op::SlotDivImm(s, imm) => Op::SlotDivImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        Op::SlotEqImm(s, imm) => Op::SlotEqImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        Op::SlotLtImm(s, imm) => Op::SlotLtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        Op::SlotGtImm(s, imm) => Op::SlotGtImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
        ),
        Op::JumpIfSlotEqImm(_, _, _) => Op::JumpIfSlotEqImm(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.boundary_i64(),
            rng.next_usize(max_pc + 1),
        ),
        Op::RecurIncAccum(_, _, step, _, _) => Op::RecurIncAccum(
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            rng.next_usize(if num_slots == 0 { 1 } else { num_slots }),
            step,
            rng.next_range(1, 10),
            rng.next_usize(max_pc + 1),
        ),
        // For everything else, just replace with random
        _ => {
            let idx = rng.next_usize(FUZZ_OPS.len());
            fuzz_op_to_op(rng, FUZZ_OPS[idx].clone(), max_pc, num_slots)
        }
    }
}

#[test]
fn test_differential_fuzz_mutation() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let corpus = corpus_programs();
            let mut rng = Rng::new(271828); // e
            let mut mismatches = 0;
            let mutations_per_program = 50;

            for (base_code, base_slots) in &corpus {
                let num_slots = base_slots.len();
                for m in 0..mutations_per_program {
                    // Clone and mutate 1-2 ops
                    let mut code = base_code.clone();
                    let max_pc = code.len();
                    let num_mutations = rng.next_usize(2) + 1; // 1 or 2
                    for _ in 0..num_mutations {
                        let idx = rng.next_usize(code.len());
                        code[idx] = mutate_op(&mut rng, &code[idx], num_slots, max_pc);
                    }

                    // Ensure termination
                    let has_terminal = code
                        .iter()
                        .any(|op| matches!(op, Op::Return | Op::ReturnSlot(_)));
                    if !has_terminal {
                        code.push(Op::Return);
                    }

                    if let Some(desc) = differential_test_one(code.clone(), base_slots.clone(), 5000) {
                        mismatches += 1;
                        eprintln!("MUTATION MISMATCH #{m}: {desc}"); fuzz_common::report_mismatch("MUTATION", m, &desc, &code, &base_slots, 5000);
                    }
                }
            }

            assert_eq!(
                mismatches,
                0,
                "Found {} mismatches in mutation fuzzing across {} programs × {} mutations",
                mismatches,
                corpus.len(),
                mutations_per_program
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Cross-type coercion stress: hit every type coercion edge case
// ---------------------------------------------------------------------------

#[test]
fn test_differential_fuzz_type_coercion() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(16180); // golden ratio
            let mut mismatches = 0;
            let total = 2000;

            for i in 0..total {
                // Generate a coercion-heavy program: push mixed types, do arithmetic/comparison
                let num_slots = rng.next_usize(3) + 1; // 1-3 slots
                let code_len = rng.next_usize(10) + 4; // 4-13 ops
                let _max_pc = code_len + 1;

                let mut code = Vec::with_capacity(code_len);

                // Phase 1: Push 2-4 mixed-type values onto the stack
                let num_pushes = rng.next_usize(3) + 2; // 2-4
                for _ in 0..num_pushes {
                    // Heavily biased toward float/nil/bool to stress coercion
                    match rng.next_usize(8) {
                        0 => code.push(Op::PushI64(rng.boundary_i64())),
                        1 => code.push(Op::PushFloat(rng.boundary_f64())),
                        2 => code.push(Op::PushNil),
                        3 => code.push(Op::PushBool(rng.next_usize(2) == 0)),
                        4 => code.push(Op::PushNil), // double-weight nil
                        5 => code.push(Op::PushFloat(f64::NAN)), // double-weight NaN
                        6 => code.push(Op::PushI64(0)), // zero
                        _ => code.push(Op::PushFloat(0.0)), // 0.0
                    }
                }

                // Phase 2: Apply 1-3 operations (arithmetic, comparison, typed ops)
                let num_ops = rng.next_usize(3) + 1; // 1-3
                for _ in 0..num_ops {
                    match rng.next_usize(10) {
                        0 => code.push(Op::Add),
                        1 => code.push(Op::Sub),
                        2 => code.push(Op::Mul),
                        3 => code.push(Op::Div),
                        4 => code.push(Op::Mod),
                        5 => code.push(Op::Eq),
                        6 => code.push(Op::Lt),
                        7 => code.push(Op::Gt),
                        // TypedBinOp with random type — hits the float→0 coercion
                        8 => {
                            let binops = [
                                BinOp::Add,
                                BinOp::Sub,
                                BinOp::Mul,
                                BinOp::Div,
                                BinOp::Lt,
                                BinOp::Eq,
                            ];
                            let op = binops[rng.next_usize(binops.len())].clone();
                            let ty = if rng.next_usize(2) == 0 {
                                Ty::I64
                            } else {
                                Ty::F64
                            };
                            code.push(Op::TypedBinOp(op, ty));
                        }
                        _ => code.push(Op::Dup),
                    }
                }

                // Phase 3: maybe a slot store/load to test slot coercion
                if rng.next_usize(3) == 0 && num_slots > 0 {
                    let s = rng.next_usize(num_slots);
                    code.push(Op::StoreSlot(s));
                    code.push(Op::LoadSlot(s));
                }

                // Terminate
                code.push(Op::Return);

                // Random initial slots — also type-mixed
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    match rng.next_usize(5) {
                        0 => init_slots.push(LispVal::Num(rng.boundary_i64())),
                        1 => init_slots.push(LispVal::Float(rng.boundary_f64())),
                        2 => init_slots.push(LispVal::Nil),
                        3 => init_slots.push(LispVal::Bool(rng.next_usize(2) == 0)),
                        _ => init_slots.push(LispVal::Str("key".into())),
                    }
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 1000) {
                    mismatches += 1;
                    eprintln!("COERCION MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("COERCION", i, &desc, &code, &init_slots, 1000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in {} coercion-stress programs",
                mismatches, total
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Test: Long programs (50-200 ops)
// ---------------------------------------------------------------------------
/// Generates longer programs to exercise deeper code paths and more
/// complex interactions between opcodes. Uses the standard random generator
/// but with significantly more instructions per program.
#[test]
fn test_differential_fuzz_long_programs() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let seeds = [42, 137, 2718];
            let mut mismatches = 0;
            let programs_per_seed = 30;

            for &seed in &seeds {
                let mut rng = Rng::new(seed);
                for i in 0..programs_per_seed {
                    let num_slots = rng.next_usize(6) + 1; // 1-6 slots
                    let code_len = rng.next_usize(151) + 50; // 50-200 instructions
                    let code = generate_random_program(&mut rng, num_slots, code_len);

                    let mut init_slots = Vec::with_capacity(num_slots);
                    for _ in 0..num_slots {
                        init_slots.push(rng.next_lisp_val());
                    }

                    // Longer programs need more steps
                    if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
                        mismatches += 1;
                        eprintln!("LONG PROG MISMATCH seed={} prog={}: {}", seed, i, desc);
                    }
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in long-program tests",
                mismatches
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Test: Recur loop stress
// ---------------------------------------------------------------------------
/// Stress-tests the Recur and RecurIncAccum paths by generating programs
/// that heavily use recursion with various slot configurations and
/// boundary-value loop counters.
#[test]
fn test_differential_fuzz_recur_stress() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(99991);
            let total = 300;
            let mut mismatches = 0;

            for i in 0..total {
                // Generate a recur-heavy program:
                // Pattern: check counter, maybe recur, otherwise return
                let num_slots = rng.next_usize(4) + 2; // 2-5 slots
                let counter_slot = rng.next_usize(num_slots);
                let accum_slot = loop {
                    let s = rng.next_usize(num_slots);
                    if s != counter_slot {
                        break s;
                    }
                };

                // Pick a limit from boundary values to test edge cases
                // wrapping_abs to avoid panic on i64::MIN
                let limit: i64 = rng.boundary_i64().wrapping_abs().min(20);

                // Return address is after the recur block
                // 0: JumpIfSlotEqImm(counter, limit, ret_addr)
                // 1: SlotSubImm(counter, 1)       -- push counter-1
                // 2: LoadSlot(accum)              -- push accum
                // 3: LoadSlot(counter)             -- push counter
                // 4: Add                           -- push accum+counter
                // 5: StoreSlot(accum)              -- accum = accum+counter
                // 6: RecurIncAccum(counter, accum, 1, 1, 0) -- counter+=1, if >= limit goto ret
                // 7: Jump(0)                       -- loop back
                // 8: ReturnSlot(accum)
                let ret_addr = 8;
                let code = vec![
                    Op::JumpIfSlotEqImm(counter_slot, limit, ret_addr), // 0
                    Op::SlotSubImm(counter_slot, 1),                    // 1
                    Op::LoadSlot(accum_slot),                           // 2
                    Op::LoadSlot(counter_slot),                         // 3
                    Op::Add,                                            // 4
                    Op::StoreSlot(accum_slot),                          // 5
                    Op::RecurIncAccum(counter_slot, accum_slot, 1, limit.max(1) + 1, ret_addr), // 6
                    Op::Jump(0),                                        // 7
                    Op::ReturnSlot(accum_slot),                         // 8
                ];

                // Init: counter starts at 0, accum at 0, other slots random
                let mut init_slots = vec![LispVal::Num(0); num_slots];
                init_slots[counter_slot] = LispVal::Num(0);
                init_slots[accum_slot] = LispVal::Num(0);
                for s in 0..num_slots {
                    if s != counter_slot && s != accum_slot {
                        init_slots[s] = rng.next_lisp_val();
                    }
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
                    mismatches += 1;
                    eprintln!("RECUR STRESS MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("RECUR STRESS", i, &desc, &code, &init_slots, 5000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in recur-stress tests",
                mismatches
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Test: Shrinking on mismatch
// ---------------------------------------------------------------------------
/// When a mismatch is found, this helper tries to shrink the program by
/// removing one instruction at a time and checking if the mismatch persists.
/// Returns the smallest reproducer found.
fn shrink_program(
    code: &[Op],
    init_slots: &[LispVal],
    max_steps: usize,
) -> Option<(Vec<Op>, String)> {
    let original_mismatch = differential_test_one(code.to_vec(), init_slots.to_vec(), max_steps);
    if original_mismatch.is_none() {
        return None; // No mismatch to shrink
    }
    let desc = original_mismatch.unwrap();

    let mut best = code.to_vec();
    let mut best_desc = desc;

    // Try removing one instruction at a time
    loop {
        let mut improved = false;
        for skip in 0..best.len() {
            // Don't remove the only return
            if matches!(best[skip], Op::Return | Op::ReturnSlot(_))
                && best
                    .iter()
                    .filter(|o| matches!(o, Op::Return | Op::ReturnSlot(_)))
                    .count()
                    <= 1
            {
                continue;
            }

            let mut candidate: Vec<Op> = best
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, op)| op.clone())
                .collect();

            if candidate.is_empty() {
                continue;
            }

            // Ensure there's still a return
            let has_ret = candidate
                .iter()
                .any(|o| matches!(o, Op::Return | Op::ReturnSlot(_)));
            if !has_ret {
                candidate.push(Op::Return);
            }

            if let Some(d) =
                differential_test_one(candidate.clone(), init_slots.to_vec(), max_steps)
            {
                // Still mismatches with fewer instructions — keep shrinking
                if candidate.len() < best.len() {
                    best = candidate;
                    best_desc = d;
                    improved = true;
                    break; // restart from smaller program
                }
            }
        }
        if !improved {
            break;
        }
    }

    Some((best, best_desc))
}

#[test]
fn test_differential_fuzz_shrink() {
    // This test doesn't find bugs directly — it verifies the shrinker works
    // by taking a few random programs and confirming they DON'T mismatch,
    // then testing that the shrink helper returns None for passing programs.
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut rng = Rng::new(77777);
            for _ in 0..20 {
                let num_slots = rng.next_usize(3);
                let code_len = rng.next_usize(10) + 5;
                let code = generate_random_program(&mut rng, num_slots, code_len);
                let init_slots: Vec<LispVal> =
                    (0..num_slots).map(|_| rng.next_lisp_val()).collect();

                // These should all pass — shrink should return None
                let result = shrink_program(&code, &init_slots, 1000);
                assert!(
                    result.is_none(),
                    "Shrinker found mismatch in random program — that's a real bug: {:?}",
                    result
                );
            }
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Test: Stack depth pressure
// ---------------------------------------------------------------------------
/// Generates programs that push many values without popping to test
/// stack growth behavior. Then pops them off and does arithmetic to
/// ensure deep-stack values are preserved correctly.
#[test]
fn test_differential_fuzz_stack_depth() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let seeds = [111, 222, 333];
            let mut mismatches = 0;
            let programs_per_seed = 50;

            for &seed in &seeds {
                let mut rng = Rng::new(seed);

                for i in 0..programs_per_seed {
                    let num_slots = rng.next_usize(3) + 1; // 1-3 slots
                    let push_count = rng.next_usize(40) + 10; // 10-49 pushes
                    let mut code = Vec::with_capacity(push_count * 2 + 20);

                    // Phase 1: Push many values (mixed types)
                    for _ in 0..push_count {
                        match rng.next_usize(5) {
                            0 => code.push(Op::PushI64(rng.boundary_i64())),
                            1 => code.push(Op::PushFloat(rng.boundary_f64())),
                            2 => code.push(Op::PushNil),
                            3 => code.push(Op::PushBool(rng.next_usize(2) == 0)),
                            _ => {
                                // Push a slot value (tests slot+stack interaction at depth)
                                code.push(Op::LoadSlot(rng.next_usize(num_slots)));
                            }
                        }
                    }

                    // Phase 2: Pop some and do arithmetic on the deep values
                    let ops_after = rng.next_usize(10) + 5; // 5-14 ops
                    for _ in 0..ops_after {
                        match rng.next_usize(6) {
                            0 => code.push(Op::Add),
                            1 => code.push(Op::Sub),
                            2 => code.push(Op::Mul),
                            3 => code.push(Op::Eq),
                            4 => code.push(Op::Dup),
                            _ => code.push(Op::Pop),
                        }
                    }

                    code.push(Op::Return);

                    let init_slots: Vec<LispVal> =
                        (0..num_slots).map(|_| rng.next_lisp_val()).collect();

                    if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 5000) {
                        mismatches += 1;
                        eprintln!("STACK DEPTH MISMATCH seed={} prog={}: {}", seed, i, desc);
                    }
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in stack-depth tests",
                mismatches
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Test: Dangerous sequences (stack underflow + compound effects)
// ---------------------------------------------------------------------------
/// Tests sequences where stack underflow is expected:
/// - Pop on empty stack, then StoreSlot (Nil written)
/// - Dup on empty stack (no-op)
/// - MakeList(n) with fewer than n items (Nil-filled)
/// - Binary ops on empty/1-element stack (Nil operands)
/// - Chained underflow: Pop, Pop, Pop, Add, StoreSlot, ReturnSlot
/// - ReturnSlot on empty stack after Pop
#[test]
fn test_differential_fuzz_dangerous_sequences() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut mismatches = 0;

            // Helper: run a single program and count mismatches
            let mut check = |code: Vec<Op>, slots: Vec<LispVal>, label: &str| {
                if let Some(desc) = differential_test_one(code, slots, 1000) {
                    mismatches += 1;
                    eprintln!("DANGEROUS MISMATCH [{}]: {}", label, desc);
                }
            };

            // --- Pop on empty stack, then StoreSlot ---
            // Pop does nothing (empty), StoreSlot pops Nil from empty stack
            check(
                vec![Op::Pop, Op::StoreSlot(0), Op::ReturnSlot(0)],
                vec![LispVal::Num(99)],
                "pop_empty_then_storeslot",
            );

            // --- Multiple pops on empty ---
            check(
                vec![Op::Pop, Op::Pop, Op::Pop, Op::Return],
                vec![],
                "triple_pop_empty",
            );

            // --- Dup on empty stack ---
            // Dup does nothing, Return returns Nil (empty stack)
            check(vec![Op::Dup, Op::Return], vec![], "dup_empty");

            // --- Dup on empty, then Add (two Nils) ---
            check(
                vec![Op::Dup, Op::Add, Op::Return],
                vec![],
                "dup_empty_then_add",
            );

            // --- MakeList with underflow ---
            // MakeList(3) but nothing on stack → [Nil, Nil, Nil]
            check(
                vec![Op::MakeList(3), Op::Return],
                vec![],
                "makelist_underflow_3",
            );

            // MakeList(2) with only 1 item → [Nil, 42]
            check(
                vec![Op::PushI64(42), Op::MakeList(2), Op::Return],
                vec![],
                "makelist_underflow_2_of_1",
            );

            // MakeList(5) with 2 items → [Nil, Nil, Nil, 7, 3]
            check(
                vec![Op::PushI64(3), Op::PushI64(7), Op::MakeList(5), Op::Return],
                vec![],
                "makelist_underflow_5_of_2",
            );

            // --- Binary ops on empty stack ---
            check(vec![Op::Add, Op::Return], vec![], "add_empty");
            check(vec![Op::Sub, Op::Return], vec![], "sub_empty");
            check(vec![Op::Mul, Op::Return], vec![], "mul_empty");
            check(vec![Op::Div, Op::Return], vec![], "div_empty");
            check(vec![Op::Mod, Op::Return], vec![], "mod_empty");
            check(vec![Op::Eq, Op::Return], vec![], "eq_empty");
            check(vec![Op::Lt, Op::Return], vec![], "lt_empty");

            // --- Binary ops with 1 element (partial underflow) ---
            // Add with only b=42, a=Nil (underflow)
            check(
                vec![Op::PushI64(42), Op::Add, Op::Return],
                vec![],
                "add_one_element",
            );
            check(
                vec![Op::PushI64(42), Op::Sub, Op::Return],
                vec![],
                "sub_one_element",
            );

            // --- Pop then return slot (slot was never written) ---
            // PushI64(5), Pop removes it, StoreSlot(0) writes Nil, ReturnSlot(0) → Nil
            check(
                vec![Op::PushI64(5), Op::Pop, Op::StoreSlot(0), Op::ReturnSlot(0)],
                vec![LispVal::Num(100)],
                "pop_then_storeslot_nil",
            );

            // --- Chained underflow ---
            // Pop, Pop, Pop, Add (both Nil), StoreSlot(0), ReturnSlot(0)
            check(
                vec![
                    Op::Pop,
                    Op::Pop,
                    Op::Pop,
                    Op::Add,
                    Op::StoreSlot(0),
                    Op::ReturnSlot(0),
                ],
                vec![LispVal::Num(0)],
                "chained_underflow_add_nil",
            );

            // --- Pop then Dup (both on empty) ---
            check(
                vec![Op::Pop, Op::Dup, Op::Return],
                vec![],
                "pop_then_dup_empty",
            );

            // --- MakeList(0) - edge case ---
            check(vec![Op::MakeList(0), Op::Return], vec![], "makelist_zero");

            // --- StoreSlot on empty → writes Nil, then LoadSlot reads it back ---
            check(
                vec![Op::StoreSlot(0), Op::LoadSlot(0), Op::Return],
                vec![LispVal::Num(42)],
                "storeslot_empty_writes_nil",
            );

            // --- StoreAndLoadSlot on empty stack ---
            // Pushes old slot value (42), stores Nil from empty stack
            check(
                vec![Op::StoreAndLoadSlot(0), Op::Return],
                vec![LispVal::Num(42)],
                "storeandloadslot_empty",
            );

            // --- Random dangerous programs (structured) ---
            let mut rng = Rng::new(31415);
            let dangerous_ops: &[Op] = &[
                Op::Pop, // underflow-safe
                Op::Dup, // no-op on empty
                Op::Add, // Nil+Nil on empty
                Op::Sub,
                Op::Mul,
                Op::Eq,
                Op::PushNil, // explicitly Nil
                Op::Return,
            ];

            for i in 0..200 {
                let num_slots = rng.next_usize(3) + 1;
                let code_len = rng.next_usize(12) + 4;

                // Generate programs heavily weighted toward underflow-inducing ops
                let mut code = Vec::with_capacity(code_len);
                for _ in 0..code_len {
                    if rng.next_usize(3) == 0 {
                        // 33%: push something (to test partial underflow)
                        match rng.next_usize(4) {
                            0 => code.push(Op::PushI64(rng.boundary_i64())),
                            1 => code.push(Op::PushFloat(rng.boundary_f64())),
                            2 => code.push(Op::PushNil),
                            _ => code.push(Op::LoadSlot(rng.next_usize(num_slots))),
                        }
                    } else {
                        // 67%: dangerous op (likely underflow)
                        let idx = rng.next_usize(dangerous_ops.len());
                        let op = dangerous_ops[idx].clone();
                        match op {
                            Op::Return => {
                                // sometimes use ReturnSlot instead
                                if rng.next_usize(2) == 0 {
                                    code.push(Op::ReturnSlot(rng.next_usize(num_slots)));
                                } else {
                                    code.push(Op::Return);
                                }
                            }
                            _ => code.push(op),
                        }
                    }
                }

                // Ensure at least one Return
                let has_ret = code
                    .iter()
                    .any(|o| matches!(o, Op::Return | Op::ReturnSlot(_)));
                if !has_ret {
                    code.push(Op::Return);
                }

                let init_slots: Vec<LispVal> =
                    (0..num_slots).map(|_| rng.next_lisp_val()).collect();

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 1000) {
                    mismatches += 1;
                    eprintln!("DANGEROUS RANDOM MISMATCH #{i}: {desc}"); fuzz_common::report_mismatch("DANGEROUS RANDOM", i, &desc, &code, &init_slots, 1000);
                }
            }

            assert_eq!(
                mismatches, 0,
                "Found {} mismatches in dangerous-sequence tests",
                mismatches
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Fused HOF opcode: SpecVM placeholder consistency check
// ---------------------------------------------------------------------------
// MapOp/FilterOp/ReduceOp are NOT supported in run_compiled_loop (the loop VM
// rejects them). They only work in run_compiled_lambda (full VM with closures).
// The SpecVM has placeholder semantics for these opcodes (push empty/identity).
// This test verifies that the SpecVM placeholder behavior is consistent.
#[test]
fn test_specvm_fused_hof_placeholders() {
    // MapOp: SpecVM pops list, pushes empty list
    let code = vec![
        Op::PushI64(1),
        Op::PushI64(2),
        Op::MakeList(2),
        Op::MapOp(0),
        Op::Return,
    ];
    let spec = SpecVm::new(code, vec![LispVal::Num(0)]).run(1000);
    assert!(
        matches!(spec, SpecResult::Value(LispVal::List(ref l)) if l.is_empty()),
        "MapOp placeholder should return empty list, got {:?}",
        spec
    );

    // FilterOp: SpecVM pops list, pushes empty list
    let code2 = vec![
        Op::PushI64(1),
        Op::PushI64(2),
        Op::MakeList(2),
        Op::FilterOp(0),
        Op::Return,
    ];
    let spec2 = SpecVm::new(code2, vec![LispVal::Num(0)]).run(1000);
    assert!(
        matches!(spec2, SpecResult::Value(LispVal::List(ref l)) if l.is_empty()),
        "FilterOp placeholder should return empty list, got {:?}",
        spec2
    );

    // ReduceOp: SpecVM pops list + init, pushes init
    let code3 = vec![
        Op::PushI64(42), // init
        Op::PushI64(1),
        Op::PushI64(2),
        Op::MakeList(2), // list
        Op::ReduceOp(0),
        Op::Return,
    ];
    let spec3 = SpecVm::new(code3, vec![LispVal::Num(0)]).run(1000);
    assert_eq!(
        spec3,
        SpecResult::Value(LispVal::Num(42)),
        "ReduceOp placeholder should return init value, got {:?}",
        spec3
    );

    // MapOp on empty list: should still return empty list
    let code4 = vec![Op::MakeList(0), Op::MapOp(0), Op::Return];
    let spec4 = SpecVm::new(code4, vec![LispVal::Num(0)]).run(1000);
    assert!(
        matches!(spec4, SpecResult::Value(LispVal::List(ref l)) if l.is_empty()),
        "MapOp empty list should return empty list, got {:?}",
        spec4
    );

    // ReduceOp on empty list: should return init
    let code5 = vec![
        Op::PushI64(99),
        Op::MakeList(0),
        Op::ReduceOp(0),
        Op::Return,
    ];
    let spec5 = SpecVm::new(code5, vec![LispVal::Num(0)]).run(1000);
    assert_eq!(
        spec5,
        SpecResult::Value(LispVal::Num(99)),
        "ReduceOp empty list should return init, got {:?}",
        spec5
    );
}

#[test]
fn test_differential_fuzz_live_torture() {
    // Live-seeded torture: every `cargo test` run explores a FRESH random slice
    // of program space (nanosecond entropy). On failure, prints the seed so the
    // mismatch is reproducible: Rng::new(<printed seed>).
    // Deterministic tests above remain the regression net for CI stability.
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let seed = live_seed();
            let mut rng = Rng::new(seed);
            let mut mismatches = 0;
            let total = 400;

            for i in 0..total {
                // Wider shape distribution than the fixed tests: 0-5 slots,
                // 2-40 ops — deeper programs than edge_cases, more slots than medium.
                let num_slots = rng.next_usize(6);
                let code_len = rng.next_usize(38) + 2;
                let code = generate_random_program(&mut rng, num_slots, code_len);

                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }

                if let Some(desc) = differential_test_one(code.clone(), init_slots.clone(), 2000) {
                    mismatches += 1;
                    eprintln!(
                        "LIVE MISMATCH #{} (seed {}): {}",
                        i, seed, desc
                    );
                }
            }

            assert_eq!(
                mismatches, 0,
                "Live torture found {} mismatches (seed {} — rerun with Rng::new({}) to reproduce)",
                mismatches, seed, seed
            );
        })
        .unwrap();
    child.join().unwrap();
}

// ---------------------------------------------------------------------------
// Shrinker tests
// ---------------------------------------------------------------------------

#[test]
fn test_shrinker_repatch_maps_jump_targets() {
    let code = vec![
        Op::PushI64(1),          // 0 — deleted
        Op::PushI64(2),          // 1 — deleted
        Op::PushI64(3),          // 2 — deleted
        Op::JumpIfTrue(5),       // 3 → jumps to old 5; after deletion → 2
        Op::PushNil,             // 4 — deleted
        Op::Jump(7),             // 5 → old 7 == len (halt) → new len 4
        Op::PushI64(9),          // 6
        Op::Return,              // 7
    ];
    let keep = [false, false, false, true, false, true, true, true];
    let out = fuzz_common::repatch_kept(&code, &keep).expect("repatch ok");
    assert_eq!(out.len(), 4);
    // kept layout: JumpIfTrue@0, Jump@1, PushI64(9)@2, Return@3
    assert_eq!(format!("{:?}", out[0]), format!("{:?}", Op::JumpIfTrue(1)));
    assert_eq!(format!("{:?}", out[1]), format!("{:?}", Op::Jump(3)));
    assert_eq!(format!("{:?}", out[2]), format!("{:?}", Op::PushI64(9)));
    assert_eq!(format!("{:?}", out[3]), format!("{:?}", Op::Return));

    // Dangling target (into a deleted run) must be rejected.
    let bad_keep = [true, false, true, false];
    let code2 = vec![Op::Jump(1), Op::PushNil, Op::PushNil, Op::Return];
    assert!(fuzz_common::repatch_kept(&code2, &bad_keep).is_none());
}

#[test]
fn test_shrinker_ddmin_strips_junk_preserves_error() {
    use lisp_rlm_wasm::types::LispVal;
    // Gate: the SPEC VM errors (SlotDivImm ÷0) — synthetic, deterministic,
    // independent of the Rust VM.
    // Gate must be SPECIFIC: a degenerate empty/short program also "errors"
    // (pc out of bounds), which ddmin would happily shrink to. Pin the
    // div-by-zero error message.
    let gate = |code: &[Op], _slots: &[LispVal]| -> bool {
        let vm = fuzz_common::SpecVm::new(code.to_vec(), vec![LispVal::Nil; 1]);
        matches!(vm.run(1000), fuzz_common::SpecResult::Error(ref e) if e.contains("div"))
    };
    let junk = || Op::PushI64(0xdead);
    let code = vec![
        junk(), junk(), junk(), junk(),
        Op::SlotDivImm(0, 0), // the erroring op (÷0)
        junk(), junk(), junk(), junk(),
        junk(), junk(), junk(), junk(),
        Op::Return,
    ];
    let (out, _) = fuzz_common::shrink_with(&code, &[LispVal::Nil; 1], &gate);
    assert!(out.len() < code.len() / 2, "should strip most junk, got {:?}", out);
    assert!(
        out.iter().any(|op| format!("{:?}", op) == format!("{:?}", Op::SlotDivImm(0, 0))),
        "must keep the erroring op"
    );
    assert!(gate(&out, &[]), "shrunk program still errors");
}

#[test]
fn test_shrinker_never_accepting_returns_input() {
    let code = vec![Op::PushI64(1), Op::PushI64(2), Op::Add, Op::Return];
    let never = |_: &[Op], _: &[lisp_rlm_wasm::types::LispVal]| false;
    let (out, slots) = fuzz_common::shrink_with(&code, &[], &never);
    let (a, b) = (format!("{:?}", out), format!("{:?}", code));
    assert_eq!(a, b, "no acceptable reduction → input unchanged");
    assert!(slots.is_empty());
}

#[test]
fn test_lockstep_first_divergence_synthetic() {
    use lisp_rlm_wasm::bytecode::VmStepEv;
    let ev = |pc: usize, top: Option<&str>| VmStepEv {
        pc,
        op: format!("Op{pc}"),
        stack_len: 1,
        top: top.map(|s| s.to_string()),
        slots: vec!["Num(1)".into()],
    };
    // identical streams → None
    let a = vec![ev(0, Some("Num(1)")), ev(1, Some("Num(2)")), ev(2, None)];
    let b = a.clone();
    assert!(fuzz_common::first_divergence(&a, &b).is_none());

    // top-of-stack differs at step 1 → Some(1)
    let c = vec![ev(0, Some("Num(1)")), ev(1, Some("Num(9)")), ev(2, None)];
    let (k, sa, sb) = fuzz_common::first_divergence(&a, &c).expect("diverges");
    assert_eq!(k, 1);
    assert_eq!(sa.top.as_deref(), Some("Num(2)"));
    assert_eq!(sb.top.as_deref(), Some("Num(9)"));

    // pc differs at step 2
    let d = vec![ev(0, Some("Num(1)")), ev(1, Some("Num(2)")), ev(7, None)];
    let (k2, _, _) = fuzz_common::first_divergence(&a, &d).expect("diverges");
    assert_eq!(k2, 2);

    // prefix agrees, lengths differ → None (caller handles via err strings)
    let e = vec![ev(0, Some("Num(1)"))];
    assert!(fuzz_common::first_divergence(&a, &e).is_none());
}

#[test]
fn test_lockstep_clean_programs_agree_op_by_op() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut benign = 0usize;
            let mut rng = Rng::new(777);
            for i in 0..200 {
                let num_slots = 1 + rng.next_usize(3);
                let code_len = 6 + rng.next_usize(20);
                let code = generate_random_program(&mut rng, num_slots, code_len);
                let init_slots: Vec<lisp_rlm_wasm::types::LispVal> =
                    (0..num_slots).map(|_| rng.next_lisp_val()).collect();
                // Green tree: lockstep must find no op-level divergence
                // (barring programs that error/limit-out early — those return
                // divergence=None with equal prefixes too).
                if let Some(diag) = fuzz_common::lockstep_first_divergence(code, init_slots, 1000) {
                    if std::env::var("LOCKSTEP_DEBUG").is_ok() {
                        eprintln!("--- program #{i} ---\n{diag}");
                    }
                    benign += 1; // count, decide semantics after inspection
                }
            }
            benign
        })
        .unwrap();
    let benign = child.join().unwrap();
    assert_eq!(benign, 0, "op-level divergence on a green tree = real bug (run with LOCKSTEP_DEBUG=1 for diagnostics)");
}
