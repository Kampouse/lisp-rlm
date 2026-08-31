//! Behavioral tests for the interpreter ports of wasm_emit/call_core.rs
//! intrinsics (surface_parity, T6 drift class). Constants machine-verified
//! with python3 — see GAPS.md policy on hand-math.

use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    lisp_rlm_wasm::program::run_program(&parse_all(code).unwrap(), &mut env, &mut state)
        .map(|v| v.to_string())
        .unwrap_or_else(|e| format!("ERROR: {}", e))
}

#[test]
fn wrap_arith_matches_wasm() {
    // i64 wrapping (wasm fold_binop_wrapping: I64Add/Sub/Mul)
    assert_eq!(eval_str("(wrap-add 9223372036854775807 1)"), "-9223372036854775808");
    assert_eq!(eval_str("(wrap-sub -9223372036854775808 1)"), "9223372036854775807");
    assert_eq!(eval_str("(wrap-mul 4611686018427387904 2)"), "-9223372036854775808");
    // variadic left fold + 0-arg identity
    assert_eq!(eval_str("(wrap-add 1 2 3 4)"), "10");
    assert_eq!(eval_str("(wrap-add)"), "0");
    assert_eq!(eval_str("(wrap-mul)"), "1");
    // non-numeric → hard error, never silent bit-cast
    assert!(eval_str("(wrap-add 1 \"x\")").starts_with("ERROR"));
}

#[test]
fn muldiv_matches_wasm() {
    // unsigned 128-bit intermediate, truncating division (emit_muldiv)
    assert_eq!(eval_str("(muldiv 100 3 7)"), "42");
    assert_eq!(eval_str("(muldiv 3037000499 3037000499 3037000499)"), "3037000499");
    // c == 0 → canonical division-by-zero
    assert!(eval_str("(muldiv 1 2 0)").contains("division by zero"));
    // (-1 -1 1): u64::MAX² >> 64 = u64::MAX-1 >= 1 → overflow trap in wasm
    assert!(eval_str("(muldiv -1 -1 1)").contains("overflow"));
    assert!(eval_str("(muldiv 1 2)").starts_with("ERROR"));
}

#[test]
fn isqrt_matches_wasm() {
    assert_eq!(eval_str("(isqrt 17)"), "4");
    assert_eq!(eval_str("(isqrt 16)"), "4");
    assert_eq!(eval_str("(isqrt 0)"), "0");
    // wasm compares I64LtU: -1 reads as u64::MAX → isqrt = 2^32-1
    assert_eq!(eval_str("(isqrt -1)"), "4294967295");
    assert!(eval_str("(isqrt 1 2)").starts_with("ERROR"));
}

#[test]
fn bitops_match_wasm() {
    assert_eq!(eval_str("(band 12 10)"), "8");
    assert_eq!(eval_str("(bor 12 10)"), "14");
    assert_eq!(eval_str("(bnot 0)"), "-1");
    // shifts mask the count mod 64 (I64Shl/I64ShrU)
    assert_eq!(eval_str("(shl 1 4)"), "16");
    assert_eq!(eval_str("(shl 1 64)"), "1");
    assert_eq!(eval_str("(shr 16 2)"), "4");
    // shr is LOGICAL (I64ShrU): -1 >> 60 = 15
    assert_eq!(eval_str("(shr -1 60)"), "15");
    // shl must stay in the 61-bit payload range (emit_tag_num_checked traps)
    assert_eq!(eval_str("(shl 1 59)"), "576460752303423488");
    assert!(eval_str("(shl 1 60)").contains("out of range"));
    assert!(eval_str("(band 1)").starts_with("ERROR"));
}

#[test]
fn near_kv_get_compiles_and_reads() {
    // was rejected by the lisp-run compile gate (GAPS t18/t19) though the
    // runtime twin existed — composite key read matching near/kv writes.
    // NOTE: eval_str mints a fresh EvalState per call, so write+read must
    // share ONE program (begin) — separate calls would read an empty store.
    assert_eq!(
        eval_str(r#"(begin (near/kv 99 "acc" "slot") (near/kv-get "acc" "slot"))"#),
        "99"
    );
    // miss → Num(0) default (both surfaces; fresh state guarantees the miss)
    assert_eq!(eval_str("(near/kv-get \"no\" \"such\")"), "0");
}
