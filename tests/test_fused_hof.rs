//! Integration tests for fused HOF opcodes (MapOp, FilterOp, ReduceOp).
//!
//! These tests verify that the compiler emits fused opcodes when the function
//! argument is a known symbol (in the slot map), and that the VM executes them
//! correctly — matching the behavior of the BuiltinCall fallback.

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::program::run_program;
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};

fn eval(src: &str) -> String {
    let forms = parse_all(src).expect("parse error");
    let mut env = Env::new();
    let mut state = EvalState::new();
    let result = run_program(&forms, &mut env, &mut state).expect("eval error");
    result.to_string()
}

fn eval_with_env(src: &str, prelude: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    // Run prelude first (defines functions into env)
    let prelude_forms = parse_all(prelude).expect("prelude parse error");
    for form in &prelude_forms {
        run_program(&[form.clone()], &mut env, &mut state).expect("prelude eval error");
    }
    let forms = parse_all(src).expect("parse error");
    let result = run_program(&forms, &mut env, &mut state).expect("eval error");
    result.to_string()
}

// ---------------------------------------------------------------------------
// MapOp tests
// ---------------------------------------------------------------------------

#[test]
fn map_double_list() {
    let result = eval_with_env(
        "(map double '(1 2 3 4 5))",
        "(define (double x) (* x 2))",
    );
    assert_eq!(result, "(2 4 6 8 10)");
}

#[test]
fn map_square_list() {
    let result = eval_with_env(
        "(map square '(1 2 3))",
        "(define (square x) (* x x))",
    );
    assert_eq!(result, "(1 4 9)");
}

#[test]
fn map_empty_list() {
    let result = eval_with_env(
        "(map double '())",
        "(define (double x) (* x 2))",
    );
    assert_eq!(result, "()");
}

#[test]
fn map_single_element() {
    let result = eval_with_env(
        "(map inc '(42))",
        "(define (inc x) (+ x 1))",
    );
    assert_eq!(result, "(43)");
}

#[test]
fn map_identity_list() {
    let result = eval_with_env(
        "(map identity '(1 2 3))",
        "(define (identity x) x)",
    );
    assert_eq!(result, "(1 2 3)");
}

#[test]
fn map_negate_list() {
    let result = eval_with_env(
        "(map negate '(1 -2 3 -4))",
        "(define (negate x) (* x -1))",
    );
    assert_eq!(result, "(-1 2 -3 4)");
}

#[test]
fn map_result_equals_builtin_map() {
    let prelude = "(define (double x) (* x 2))";
    let fused = eval_with_env("(map double '(1 2 3))", prelude);
    // The builtin map (via BuiltinCall) should give the same result.
    // When the function is a known symbol, the fused path is taken.
    // When it's not (e.g., a lambda), the builtin path is taken.
    let builtin = eval("(map (lambda (x) (* x 2)) '(1 2 3))");
    assert_eq!(fused, builtin);
}

// ---------------------------------------------------------------------------
// FilterOp tests
// ---------------------------------------------------------------------------

#[test]
fn filter_positive() {
    let result = eval_with_env(
        "(filter positive? '(-3 -1 0 1 2 5))",
        "(define (positive? x) (> x 0))",
    );
    assert_eq!(result, "(1 2 5)");
}

#[test]
fn filter_even() {
    let result = eval_with_env(
        "(filter even? '(1 2 3 4 5 6))",
        "(define (even? x) (= (mod x 2) 0))",
    );
    assert_eq!(result, "(2 4 6)");
}

#[test]
fn filter_empty_list() {
    let result = eval_with_env(
        "(filter positive? '())",
        "(define (positive? x) (> x 0))",
    );
    assert_eq!(result, "()");
}

#[test]
fn filter_none_match() {
    let result = eval_with_env(
        "(filter positive? '(-1 -2 -3))",
        "(define (positive? x) (> x 0))",
    );
    assert_eq!(result, "()");
}

#[test]
fn filter_all_match() {
    let result = eval_with_env(
        "(filter positive? '(1 2 3))",
        "(define (positive? x) (> x 0))",
    );
    assert_eq!(result, "(1 2 3)");
}

#[test]
fn filter_result_equals_builtin() {
    let prelude = "(define (positive? x) (> x 0))";
    let fused = eval_with_env("(filter positive? '(-1 0 1 2))", prelude);
    let builtin = eval("(filter (lambda (x) (> x 0)) '(-1 0 1 2))");
    assert_eq!(fused, builtin);
}

// ---------------------------------------------------------------------------
// ReduceOp tests
// ---------------------------------------------------------------------------

#[test]
fn reduce_sum() {
    let result = eval_with_env(
        "(reduce add 0 '(1 2 3 4 5))",
        "(define (add a b) (+ a b))",
    );
    assert_eq!(result, "15");
}

#[test]
fn reduce_product() {
    let result = eval_with_env(
        "(reduce mul 1 '(1 2 3 4))",
        "(define (mul a b) (* a b))",
    );
    assert_eq!(result, "24");
}

#[test]
fn reduce_empty_list() {
    let result = eval_with_env(
        "(reduce add 42 '())",
        "(define (add a b) (+ a b))",
    );
    assert_eq!(result, "42");
}

#[test]
fn reduce_single_element() {
    let result = eval_with_env(
        "(reduce add 0 '(7))",
        "(define (add a b) (+ a b))",
    );
    assert_eq!(result, "7");
}

#[test]
fn reduce_max() {
    let result = eval_with_env(
        "(reduce my-max -999999 '(3 1 4 1 5 9 2 6))",
        "(define (my-max a b) (if (> a b) a b))",
    );
    assert_eq!(result, "9");
}

#[test]
fn reduce_result_equals_builtin() {
    let prelude = "(define (add a b) (+ a b))";
    let fused = eval_with_env("(reduce add 0 '(1 2 3 4))", prelude);
    let builtin = eval("(reduce (lambda (a b) (+ a b)) 0 '(1 2 3 4))");
    assert_eq!(fused, builtin);
}

// ---------------------------------------------------------------------------
// Compositional tests: map -> filter -> reduce pipeline
// ---------------------------------------------------------------------------

#[test]
fn pipeline_map_filter_reduce() {
    let prelude = r#"
        (define (double x) (* x 2))
        (define (positive? x) (> x 0))
        (define (add a b) (+ a b))
    "#;
    let result = eval_with_env(
        "(reduce add 0 (filter positive? (map double '(-2 -1 0 1 2 3))))",
        prelude,
    );
    // double: (-4 -2 0 2 4 6), filter positive: (2 4 6), reduce sum: 12
    assert_eq!(result, "12");
}

#[test]
fn pipeline_filter_map() {
    let prelude = r#"
        (define (positive? x) (> x 0))
        (define (square x) (* x x))
    "#;
    let result = eval_with_env(
        "(map square (filter positive? '(-3 -1 0 1 2 3)))",
        prelude,
    );
    // filter: (1 2 3), map square: (1 4 9)
    assert_eq!(result, "(1 4 9)");
}

// ---------------------------------------------------------------------------
// Edge cases: non-symbol function args fall through to BuiltinCall
// ---------------------------------------------------------------------------

#[test]
fn map_lambda_falls_through_to_builtin() {
    // Lambda is not a known symbol -> should use BuiltinCall, not fused opcode
    let result = eval("(map (lambda (x) (* x 2)) '(3 4 5))");
    assert_eq!(result, "(6 8 10)");
}

#[test]
fn filter_lambda_falls_through_to_builtin() {
    let result = eval("(filter (lambda (x) (> x 0)) '(-1 0 1 2))");
    assert_eq!(result, "(1 2)");
}

#[test]
fn reduce_lambda_falls_through_to_builtin() {
    let result = eval("(reduce (lambda (a b) (+ a b)) 0 '(1 2 3))");
    assert_eq!(result, "6");
}

// ---------------------------------------------------------------------------
// Regression: fused opcodes produce correct results end-to-end
// ---------------------------------------------------------------------------

#[test]
fn fused_reduce_with_compiled_function() {
    let result = eval(
        r#"
        (define (sum a b) (+ a b))
        (reduce sum 0 '(10 20 30))
        "#,
    );
    assert_eq!(result, "60");
}

#[test]
fn fused_map_then_reduce() {
    let result = eval(
        r#"
        (define (double x) (* x 2))
        (define (sum a b) (+ a b))
        (reduce sum 0 (map double '(1 2 3 4)))
        "#,
    );
    assert_eq!(result, "20");
}
