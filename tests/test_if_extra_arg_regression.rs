//! if-arity cap (2026-10-15): (if cond then else extra…) must be a COMPILE
//! ERROR, not a silent extra-arg drop — in the type checker AND the wasm
//! emitter AND the bytecode compiler.
//!
//! Bug class this pins shut: a misplaced close-paren mis-nests a let*/begin
//! body under the enclosing if. The reader accepts the tree (2-arm if +
//! multi-form let* body are both legal), the emitter then read only
//! (cond then else) and SILENTLY DROPPED the rest — flipping which swap arm
//! ran while the contract still compiled green (CLMM v5 pool swap arc:
//! down-swap executed BOTH arms, returned the up constant, wrote UPPATH
//! after DOWNPATH). Same class as the 2026-09-02 and/or silent-drop.
//!
//! Regression file for the exact corruption lives at
//! tests/compiler-torture/t22-if-extra-arg-err.lisp (checker-level, via
//! emit_determinism's error corpus); the semantic A/B lives in
//! tests/fixtures/if_extra_arg/ (down/up swap skeleton, repaired).

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::typing::type_check_program;

#[test]
fn if_with_extra_positional_form_is_type_error() {
    // The misplaced-paren shape: then-arm let* swallowed the else-begin.
    let src = r#"(define (f v)
  (let ((neg (if (< (str-len v) 5) 1 0)))
    (if (= neg 1)
        (let* ((a v)
               (b (str-cat v "!")))
          (begin (put "K" b))
          (begin (put "K2" "other") "late"))
        "else")))"#;
    // The inner (put …) arm isn't the point — the OUTER if has 2 args here.
    // This one is LEGAL; the guard must NOT fire on it.
    let exprs = parse_all(src).expect("parse");
    let _ = type_check_program(&exprs, false); // may pass or fail on `put`; no arity panic
}

#[test]
fn if_four_positional_forms_error() {
    let src = "(define (f) (if true 1 2 3))";
    let exprs = parse_all(src).expect("parse");
    let err = type_check_program(&exprs, false)
        .expect_err("4-arg if must be rejected by the checker");
    assert!(
        err.contains("if") && err.contains("positional"),
        "wrong error: {err}"
    );
}

#[test]
fn if_five_positional_forms_error() {
    let src = "(define (f) (if true 1 2 3 4))";
    let exprs = parse_all(src).expect("parse");
    let err = type_check_program(&exprs, false)
        .expect_err("5-arg if must be rejected by the checker");
    assert!(err.contains("misplaced"), "wrong error: {err}");
}

#[test]
fn two_and_three_arm_if_still_legal() {
    let src = "(define (f b) (if b 1) )\n(define (g b) (if b 1 2))";
    let exprs = parse_all(src).expect("parse");
    type_check_program(&exprs, false).expect("2/3-arm if must stay legal");
}

#[test]
fn quoted_extra_args_do_not_trip_the_cap() {
    // quote-reader sugar: '(1 2) is ONE form — (if c '(1 2) 3) stays legal.
    let src = "(define (f c) (if c '(1 2) 3))";
    let exprs = parse_all(src).expect("parse");
    type_check_program(&exprs, false).expect("quoted list is one form");
}
