//! Tests for bytecode compiler extensions: let, let*, when, unless, CallCaptured

use lisp_rlm::EvalState;
use lisp_rlm::*;

fn run_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    run_program(code).unwrap_or_else(|e| format!("ERROR: {}", e))
}

fn _eval_val(code: &str) -> LispVal {
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state).unwrap();
    }
    result
}

// --- let in lambda (bytecode fast path) ---

#[test]
fn test_map_lambda_with_let() {
    // let inside a lambda should compile to bytecode
    let result = eval(r#"(map (lambda (x) (let ((y (+ x 1))) (* y 2))) (list 1 2 3))"#);
    assert_eq!(result, "(4 6 8)");
}

#[test]
fn test_map_lambda_with_nested_let() {
    let result = eval(
        r#"(map (lambda (x) (let ((a (+ x 1)) (b (* x 2))) (+ a b))) (list 1 2 3))"#,
    );
    // x=1: a=2, b=2 → 4; x=2: a=3, b=4 → 7; x=3: a=4, b=6 → 10
    assert_eq!(result, "(4 7 10)");
}

#[test]
fn test_filter_lambda_with_let() {
    let result = eval(
        r#"(filter (lambda (x) (let ((d (* x 2))) (> d 5))) (list 1 2 3 4 5))"#,
    );
    // x=1: d=2 no; x=2: d=4 no; x=3: d=6 yes; x=4: d=8 yes; x=5: d=10 yes
    assert_eq!(result, "(3 4 5)");
}

#[test]
fn test_map_lambda_with_let_star() {
    // let*: second binding sees first
    let result = eval(
        r#"(map (lambda (x) (let* ((a (+ x 1)) (b (* a 2))) b)) (list 1 2 3))"#,
    );
    // x=1: a=2, b=4; x=2: a=3, b=6; x=3: a=4, b=8
    assert_eq!(result, "(4 6 8)");
}

// --- when/unless in lambda (bytecode fast path) ---

#[test]
fn test_map_lambda_with_when() {
    let result = eval(
        r#"(map (lambda (x) (if (> x 2) (* x 10) x)) (list 1 2 3 4))"#,
    );
    assert_eq!(result, "(1 2 30 40)");
}

#[test]
fn test_filter_lambda_with_when_body() {
    // when inside a lambda
    let result = eval(
        r#"(map (lambda (x) (when (> x 2) (+ x 100))) (list 1 2 3 4))"#,
    );
    // when is false → nil, when is true → result
    assert_eq!(result, "(nil nil 103 104)");
}

#[test]
fn test_map_lambda_with_unless() {
    let result = eval(
        r#"(map (lambda (x) (unless (> x 2) (+ x 100))) (list 1 2 3 4))"#,
    );
    // unless condition is true (x>2) → nil, false → body
    assert_eq!(result, "(101 102 nil nil)");
}

// --- CallCaptured: calling outer functions from lambda ---

#[test]
fn test_map_lambda_calling_user_function() {
    let result = eval(
        r#"
        (define (double x) (* x 2))
        (map (lambda (x) (double x)) (list 1 2 3))
    "#,
    );
    assert_eq!(result, "(2 4 6)");
}

#[test]
fn test_map_lambda_calling_user_function_complex() {
    let result = eval(
        r#"
        (define (add-tax price) (+ (* price 0.13) price))
        (map (lambda (p) (add-tax p)) (list 10 20 30))
    "#,
    );
    // 10+1.3=11.3, 20+2.6=22.6, 30+3.9=33.9
    // Float arithmetic in bytecode
    assert!(result.contains("11.3"), "expected 11.3 in output, got: {}", result);
}

#[test]
fn test_filter_lambda_calling_user_predicate() {
    let result = eval(
        r#"
        (define (even-and-big? x) (and (= (mod x 2) 0) (> x 3)))
        (filter (lambda (x) (even-and-big? x)) (list 1 2 3 4 5 6))
    "#,
    );
    // 4 and 6 are even and > 3
    assert_eq!(result, "(4 6)");
}

#[test]
fn test_map_lambda_let_and_user_fn_combined() {
    let result = eval(
        r#"
        (define (square x) (* x x))
        (map (lambda (x) (let ((s (square x))) (+ s 1))) (list 1 2 3 4))
    "#,
    );
    // 1+1=2, 4+1=5, 9+1=10, 16+1=17
    assert_eq!(result, "(2 5 10 17)");
}

#[test]
fn test_map_lambda_chained_user_calls() {
    let result = eval(
        r#"
        (define (inc x) (+ x 1))
        (define (dbl x) (* x 2))
        (map (lambda (x) (+ (inc x) (dbl x))) (list 1 2 3))
    "#,
    );
    // x=1: inc=2, dbl=2 → 4; x=2: inc=3, dbl=4 → 7; x=3: inc=4, dbl=6 → 10
    assert_eq!(result, "(4 7 10)");
}

// --- let in loop VM ---

#[test]
fn test_loop_with_let_in_body() {
    let result = eval(
        r#"
        (loop ((i 0) (sum 0))
            (if (>= i 5)
                sum
                (let ((next (+ sum (* i i))))
                    (recur (+ i 1) next))))
    "#,
    );
    // sum of squares 0..4: 0+1+4+9+16 = 30
    assert_eq!(result, "30");
}

// --- Edge cases ---

#[test]
fn test_let_shadowing_in_lambda() {
    // let-bound x should shadow param x inside the body
    let result = eval(
        r#"(map (lambda (x) (let ((x 42)) x)) (list 1 2 3))"#,
    );
    assert_eq!(result, "(42 42 42)");
}

#[test]
fn test_nested_let_in_lambda() {
    let result = eval(
        r#"(map (lambda (x) (let ((a (+ x 1))) (let ((b (* a 2))) b))) (list 1 2 3))"#,
    );
    // x=1: a=2, b=4; x=2: a=3, b=6; x=3: a=4, b=8
    assert_eq!(result, "(4 6 8)");
}

#[test]
fn test_when_with_multiple_body_forms() {
    let result = eval(
        r#"(map (lambda (x) (when (> x 2) (+ x 1) (* x 10))) (list 1 2 3 4))"#,
    );
    // when false → nil; when true, last form value: (* x 10)
    assert_eq!(result, "(nil nil 30 40)");
}
