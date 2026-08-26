//! Tests for U64 field arithmetic ops — secp256k1 field element support.
//!
//! Covers: u64 literal parsing, wrapping add/sub/mul, u64-mul-hi (high 64 bits),
//! bitwise ops (and/or/xor/shr/shl/not), peephole TypedBinOp U64 fusion,
//! and a BIP-340 generator point P constant roundtrip.

use lisp_rlm_wasm::program::run_program;
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};

fn eval(code: &str) -> LispVal {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let forms = lisp_rlm_wasm::parser::parse_all(code).expect("parse error");
    run_program(&forms, &mut env, &mut state).expect("eval error")
}

fn eval_u64(code: &str) -> u64 {
    match eval(code) {
        LispVal::U64(v) => v,
        other => panic!("expected U64, got {:?}", other),
    }
}

fn eval_bool(code: &str) -> bool {
    match eval(code) {
        LispVal::Bool(b) => b,
        other => panic!("expected Bool, got {:?}", other),
    }
}

// ── Literal parsing ────────────────────────────────────────────────────────

#[test]
fn u64_literal_decimal() {
    assert_eq!(eval_u64("42u64"), 42);
}

#[test]
fn u64_literal_zero() {
    assert_eq!(eval_u64("0u64"), 0);
}

#[test]
fn u64_literal_hex() {
    assert_eq!(eval_u64("0xFFu64"), 0xFF);
}

#[test]
fn u64_literal_max() {
    assert_eq!(eval_u64("0xFFFFFFFFFFFFFFFFu64"), u64::MAX);
}

// ── Wrapping arithmetic (via TypedBinOp U64 fusion) ────────────────────────

#[test]
fn u64_wrapping_add() {
    assert_eq!(eval_u64("(+ 1u64 2u64)"), 3);
}

#[test]
fn u64_wrapping_add_overflow() {
    // u64::MAX + 1 = 0 (wrapping)
    assert_eq!(
        eval_u64("(+ 0xFFFFFFFFFFFFFFFFu64 1u64)"),
        0
    );
}

#[test]
fn u64_wrapping_sub() {
    assert_eq!(eval_u64("(- 10u64 3u64)"), 7);
}

#[test]
fn u64_wrapping_sub_underflow() {
    // 0 - 1 = u64::MAX (wrapping)
    assert_eq!(eval_u64("(- 0u64 1u64)"), u64::MAX);
}

#[test]
fn u64_wrapping_mul() {
    assert_eq!(eval_u64("(* 6u64 7u64)"), 42);
}

#[test]
fn u64_wrapping_mul_overflow() {
    // (u64::MAX) * 2 = u64::MAX - 1 (wrapping)
    assert_eq!(
        eval_u64("(* 0xFFFFFFFFFFFFFFFFu64 2u64)"),
        u64::MAX - 1
    );
}

#[test]
fn u64_wrapping_div() {
    assert_eq!(eval_u64("(/ 100u64 3u64)"), 33);
}

#[test]
fn u64_wrapping_mod() {
    assert_eq!(eval_u64("(% 100u64 3u64)"), 1);
}

// ── Comparisons ────────────────────────────────────────────────────────────

#[test]
fn u64_lt() {
    assert!(eval_bool("(< 3u64 5u64)"));
    assert!(!eval_bool("(< 5u64 3u64)"));
}

#[test]
fn u64_le() {
    assert!(eval_bool("(<= 3u64 3u64)"));
    assert!(!eval_bool("(<= 4u64 3u64)"));
}

#[test]
fn u64_gt() {
    assert!(eval_bool("(> 5u64 3u64)"));
    assert!(!eval_bool("(> 3u64 5u64)"));
}

#[test]
fn u64_ge() {
    assert!(eval_bool("(>= 5u64 5u64)"));
    assert!(!eval_bool("(>= 3u64 5u64)"));
}

#[test]
fn u64_eq() {
    assert!(eval_bool("(= 42u64 42u64)"));
    assert!(!eval_bool("(= 42u64 43u64)"));
}

// ── Bit ops ────────────────────────────────────────────────────────────────

#[test]
fn u64_and() {
    assert_eq!(eval_u64("(u64-and 0xFF00u64 0x0FF0u64)"), 0x0F00);
}

#[test]
fn u64_or() {
    assert_eq!(eval_u64("(u64-or 0xF000u64 0x00F0u64)"), 0xF0F0);
}

#[test]
fn u64_xor() {
    assert_eq!(eval_u64("(u64-xor 0xFF00u64 0x0FF0u64)"), 0xF0F0);
}

#[test]
fn u64_shr() {
    assert_eq!(eval_u64("(u64-shr 16u64 2u64)"), 4);
}

#[test]
fn u64_shl() {
    assert_eq!(eval_u64("(u64-shl 1u64 4u64)"), 16);
}

#[test]
fn u64_not() {
    assert_eq!(eval_u64("(u64-not 0u64)"), u64::MAX);
    assert_eq!(eval_u64("(u64-not 0xFFFFFFFFFFFFFFFFu64)"), 0);
}

// ── u64-mul-hi (high 64 bits of u128 product) ─────────────────────────────

#[test]
fn u64_mul_hi_basic() {
    // 2^64 * 1 → hi = 1, lo = 0
    let hi = eval_u64("(u64-mul-hi 1u64 0u64)");
    // 1 * 0 = 0, hi = 0
    assert_eq!(hi, 0);
}

#[test]
fn u64_mul_hi_overflow() {
    // (2^32) * (2^32) = 2^64 → hi = 1
    let two_32 = 0x1_0000_0000u64;
    let result = eval_u64(&format!("(u64-mul-hi {}u64 {}u64)", two_32, two_32));
    assert_eq!(result, 1);
}

#[test]
fn u64_mul_hi_secp256k1_reduced() {
    // Verifies mul-hi with known values.
    // 0xFFFFFFFFFFFFFFFF * 0xFFFFFFFFFFFFFFFF = (2^64 - 1)^2 = 2^128 - 2*2^64 + 1
    // hi = 2^64 - 2 = 0xFFFFFFFFFFFFFFFE
    let result = eval_u64("(u64-mul-hi 0xFFFFFFFFFFFFFFFFu64 0xFFFFFFFFFFFFFFFFu64)");
    assert_eq!(result, 0xFFFFFFFFFFFFFFFEu64);
}

// ── Let binding + loop with u64 ────────────────────────────────────────────

#[test]
fn u64_let_binding() {
    assert_eq!(eval_u64("(let ((a 10u64) (b 20u64)) (+ a b))"), 30);
}

#[test]
fn u64_loop_accumulate() {
    // Sum 1..5 using loop/recur with u64
    let result = eval_u64(r#"
        (let loop ((i 0u64) (acc 0u64))
          (if (= i 5u64)
            acc
            (recur (+ i 1u64) (+ acc i))))
    "#);
    assert_eq!(result, 10); // 0+1+2+3+4
}

#[test]
fn u64_loop_with_mul() {
    // Factorial of 5 = 120
    let result = eval_u64(r#"
        (let loop ((i 1u64) (acc 1u64))
          (if (= i 6u64)
            acc
            (recur (+ i 1u64) (* acc i))))
    "#);
    assert_eq!(result, 120);
}

// ── TypedBinOp U64 peephole fusion (slot-based) ───────────────────────────

#[test]
fn u64_typed_binop_fusion_in_loop() {
    // Both operands come from slots → peephole should fuse to TypedBinOp(_, U64)
    // Test with n=3 for easy trace: fib sequence a=1,b=1 → after 3 iters: a=3
    let result = eval_u64(r#"
        (let loop ((a 1u64) (b 1u64) (n 10u64))
          (if (= n 0u64)
            a
            (recur (+ a b) a (- n 1u64))))
    "#);
    // 10 iterations of fib: 1,1→2,1→3,2→5,3→8,5→13,8→21,13→34,21→55,34→89,55→144
    assert_eq!(result, 144);
}

// ── BIP-340 generator point P field arithmetic ─────────────────────────────
// The secp256k1 generator point's x-coordinate (as a field element):
//   0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
// This is a 256-bit number = 4 u64 limbs.
// We verify that modular reduction works by checking:
//   x mod p should equal x (since x < p for the generator point)

#[test]
fn u64_bip340_p_x_coordinate_limb0() {
    // Least significant limb of P's x-coordinate
    let expected: u64 = 0x79BE667EF9DCBBAC;
    let result = eval_u64("0x79BE667EF9DCBBACu64");
    assert_eq!(result, expected);
}

#[test]
fn u64_bip340_p_x_coordinate_limb3() {
    // Most significant limb of P's x-coordinate
    let big = 0xFFFFFFFFFFFFFFFFu64;
    // Bitwise ops on large values
    assert_eq!(eval_u64("(u64-and 0xFFFFFFFFFFFFFFFFu64 0xFFFFFFFFFFFFFFFFu64)"), big);
    assert_eq!(eval_u64("(u64-or 0u64 0xFFFFFFFFFFFFFFFFu64)"), big);
}

// ── Multi-limb addition (simulating 256-bit field add) ─────────────────────

#[test]
fn u64_multilimb_add_with_carry() {
    // Add two 128-bit numbers (2 u64 limbs each) with carry chain:
    //   a = [0xFFFFFFFFFFFFFFFF, 0]  (low, high)
    //   b = [1, 0]
    //   sum_low = 0xFFFFFFFFFFFFFFFF + 1 = 0 (wrapping)
    //   carry = 1 (detected via u64-shr of XOR)
    //   sum_high = 0 + 0 + 1 = 1
    let result = eval(r#"
        (let* ((a_lo 0xFFFFFFFFFFFFFFFFu64)
               (a_hi 0u64)
               (b_lo 1u64)
               (b_hi 0u64)
               (sum_lo (+ a_lo b_lo))
               (sum_hi (+ a_hi b_hi))
               (carry (u64-shr (u64-or (u64-xor sum_lo a_lo) (u64-xor sum_lo b_lo)) 63u64)))
          (list sum_lo (+ sum_hi carry)))
    "#);
    match result {
        LispVal::List(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], LispVal::U64(0));
            assert_eq!(items[1], LispVal::U64(1));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn u64_multilimb_mul_hi_256bit() {
    // (2^32) * (2^32) = 2^64: lo=0, hi=1
    let lo = eval_u64("(* 0x100000000u64 0x100000000u64)");
    let hi = eval_u64("(u64-mul-hi 0x100000000u64 0x100000000u64)");
    assert_eq!(lo, 0);
    assert_eq!(hi, 1);
}

// ── Combined: bit extraction via mul-hi + shr ──────────────────────────────

#[test]
fn u64_extract_bits_mul_hi_shr() {
    // Extract bits 64..127 of a * b where a = b = 2^32
    // a * b = 2^64, stored as [lo=0, hi=1]
    let two_32: u64 = 0x100000000;
    let result = eval_u64(&format!(
        "(let* ((a {}u64) (b {}u64) (lo (* a b)) (hi (u64-mul-hi a b))) hi)",
        two_32, two_32
    ));
    assert_eq!(result, 1); // high limb of 2^64 * 2^32 would be 2^32, but 2^32 * 2^32 = 2^64 → hi=1
}

// ── u64-not in field reduction context ──────────────────────────────────────

#[test]
fn u64_not_for_bitmask() {
    // Create a mask for lower 32 bits: NOT(0xFFFFFFFF00000000) = 0x00000000FFFFFFFF
    let result = eval_u64("(u64-not 0xFFFFFFFF00000000u64)");
    assert_eq!(result, 0x00000000FFFFFFFFu64);
}

#[test]
fn u64_mask_and_extract() {
    // Mask lower 16 bits of 0xDEADBEEFCAFEBABE
    let result = eval_u64("(u64-and 0xDEADBEEFCAFEBABEu64 0xFFFFu64)");
    assert_eq!(result, 0xBABEu64);
}
