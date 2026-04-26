//! Tests for define-time compilation + dict builtins + cached CallCaptured

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

// --- Define-time compilation: lambdas are pre-compiled ---

#[test]
fn test_map_compiled_lambda() {
    let result = eval(r#"(map (lambda (x) (+ x 1)) (list 10 20 30))"#);
    assert_eq!(result, "(11 21 31)");
}

#[test]
fn test_filter_compiled_lambda() {
    let result = eval(r#"(filter (lambda (x) (> x 2)) (list 1 2 3 4 5))"#);
    assert_eq!(result, "(3 4 5)");
}

#[test]
fn test_reduce_compiled_lambda() {
    let result = eval(r#"(reduce (lambda (acc x) (+ acc x)) 0 (list 1 2 3 4 5))"#);
    assert_eq!(result, "15");
}

#[test]
fn test_nested_compiled_calls() {
    let result = eval(r#"
        (define (add1 x) (+ x 1))
        (define (double x) (* x 2))
        (map (lambda (x) (double (add1 x))) (list 1 2 3))
    "#);
    assert_eq!(result, "(4 6 8)");
}

#[test]
fn test_compiled_lambda_with_let_and_call() {
    let result = eval(r#"
        (define (wrap x) (list x x))
        (map (lambda (x) (let ((doubled (* x 2))) (wrap doubled))) (list 1 2 3))
    "#);
    assert_eq!(result, "((2 2) (4 4) (6 6))");
}

#[test]
fn test_find_compiled_lambda() {
    let result = eval(r#"(find (lambda (x) (= x 3)) (list 1 2 3 4 5))"#);
    assert_eq!(result, "3");
}

#[test]
fn test_some_compiled_lambda() {
    let result = eval(r#"(some (lambda (x) (> x 10)) (list 1 5 15 2))"#);
    assert_eq!(result, "true");
}

#[test]
fn test_every_compiled_lambda() {
    let result = eval(r#"(every (lambda (x) (> x 0)) (list 1 2 3))"#);
    assert_eq!(result, "true");
}

#[test]
fn test_partition_compiled_lambda() {
    let result = eval(r#"(partition (lambda (x) (> x 3)) (list 1 2 3 4 5))"#);
    assert_eq!(result, "((4 5) (1 2 3))");
}

#[test]
fn test_fold_left_compiled_lambda() {
    let result = eval(r#"(fold-left (lambda (acc x) (- acc x)) 100 (list 10 20 30))"#);
    assert_eq!(result, "40");
}

#[test]
fn test_fold_right_compiled_lambda() {
    let result = eval(r#"(fold-right (lambda (x acc) (- x acc)) 0 (list 1 2 3))"#);
    assert_eq!(result, "2");
}

// --- Dict builtins in bytecode ---

#[test]
fn test_dict_get_in_lambda() {
    let result = eval(r#"
        (define d (dict "a" 1 "b" 2 "c" 3))
        (map (lambda (k) (dict/get d k)) (list "a" "b" "c"))
    "#);
    assert_eq!(result, "(1 2 3)");
}

#[test]
fn test_dict_get_missing_key() {
    let result = eval(r#"
        (define d (dict "a" 1))
        (map (lambda (k) (dict/get d k)) (list "a" "z"))
    "#);
    assert_eq!(result, "(1 nil)");
}

#[test]
fn test_dict_set_basic() {
    let result = eval(r#"
        (define d (dict "x" 10))
        (dict/set d "y" 20)
    "#);
    let normalized = result.replace('"', "'");
    assert!(normalized.contains("'x'") && normalized.contains("10"));
    assert!(normalized.contains("'y'") && normalized.contains("20"));
}

#[test]
fn test_dict_has_in_lambda() {
    let result = eval(r#"
        (define d (dict "a" 1 "b" 2))
        (list (dict/has? d "a") (dict/has? d "z"))
    "#);
    assert_eq!(result, "(true false)");
}

#[test]
fn test_dict_keys_basic() {
    let result = eval(r#"
        (define d (dict "x" 1 "y" 2))
        (sort (dict/keys d) string<?)
    "#);
    let normalized = result.replace('"', "'");
    assert_eq!(normalized, "('x' 'y')");
}

// --- Harness-style integration ---

#[test]
fn test_harness_style_filter_with_dict() {
    let result = eval(r#"
        (define intentions
            (list
                (dict "id" "a" "priority" 3)
                (dict "id" "b" "priority" 1)
                (dict "id" "c" "priority" 5)))
        (map (lambda (i) (dict/get i "id"))
            (filter (lambda (i) (> (dict/get i "priority") 2)) intentions))
    "#);
    let normalized = result.replace('"', "'");
    assert_eq!(normalized, "('a' 'c')");
}

#[test]
fn test_multi_level_compiled_dispatch() {
    let result = eval(r#"
        (define (process x)
            (let ((y (* x 2)))
                (+ y 1)))
        (define (batch items)
            (map process items))
        (batch (list 1 2 3 4 5))
    "#);
    assert_eq!(result, "(3 5 7 9 11)");
}
