//! Tests for the pure type checker.

use lisp_rlm::{parse_all, lisp_eval, Env, EvalState};

fn eval_source(src: &str) -> Result<String, String> {
    let exprs = parse_all(src).map_err(|e| e.to_string())?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = String::new();
    for expr in &exprs {
        let val = lisp_eval(expr, &mut env, &mut state)?;
        result = val.to_string();
    }
    Ok(result)
}

#[test]
fn pure_simple_arithmetic() {
    let r = eval_source(r#"
        (pure (define (add x y) :: int -> int -> int
          (+ x y)))
        (add 3 4)
    "#);
    assert_eq!(r.unwrap(), "7");
}

#[test]
fn pure_type_mismatch() {
    let r = eval_source(r#"
        (pure (define (add x y) :: int -> int -> int
          (str-concat x y)))
    "#);
    assert!(r.is_err(), "expected type error, got {:?}", r);
    assert!(r.unwrap_err().contains("type"), "error should mention type");
}

#[test]
fn pure_wrong_arity() {
    let r = eval_source(r#"
        (pure (define (f x y) :: int -> int -> int
          (+ x y)))
    "#);
    // This should succeed — 2 params match the arrow type
    assert!(r.is_ok());
}

#[test]
fn pure_polymorphic_identity() {
    let r = eval_source(r#"
        (pure (define (id x) x))
        (id 42)
    "#);
    // Inference with no annotation should work
    assert!(r.is_ok(), "polymorphic identity should type-check: {:?}", r);
}

#[test]
fn pure_list_operations() {
    let r = eval_source(r#"
        (pure (define (my-sum lst)
          (reduce + 0 lst)))
        (my-sum (list 1 2 3))
    "#);
    assert!(r.is_ok(), "list ops should type-check: {:?}", r);
}

#[test]
fn pure_map_hof() {
    let r = eval_source(r#"
        (pure (define (double-all lst)
          (map (lambda (x) (* x 2)) lst)))
        (double-all (list 1 2 3))
    "#);
    assert!(r.is_ok(), "map HOF should type-check: {:?}", r);
}

#[test]
fn pure_rejects_mutation() {
    let r = eval_source(r#"
        (pure (define (bad x)
          (set! x 5)))
    "#);
    // set! is not a pure form — should fail
    // Currently it'll fail because set! isn't in the type checker's known forms
    // It'll try to infer it as application of "set!" which isn't in env
    assert!(r.is_err(), "pure should reject set! {:?}", r);
}

#[test]
fn pure_nested_lambda() {
    let r = eval_source(r#"
        (pure (define (adder n)
          (lambda (x) (+ n x))))
        ((adder 5) 3)
    "#);
    assert!(r.is_ok(), "nested lambda should type-check: {:?}", r);
}

#[test]
fn pure_cond_inference() {
    let r = eval_source(r#"
        (pure (define (abs x)
          (if (< x 0) (- 0 x) x)))
        (abs -5)
    "#);
    assert!(r.is_ok(), "cond/if should type-check: {:?}", r);
}

#[test]
fn pure_let_binding() {
    let r = eval_source(r#"
        (pure (define (distance x y)
          (let ((dx (- x 0))
                (dy (- y 0)))
            (+ (* dx dx) (* dy dy)))))
        (distance 3 4)
    "#);
    assert!(r.is_ok(), "let bindings should type-check: {:?}", r);
}

#[test]
fn pure_type_error_in_body() {
    let r = eval_source(r#"
        (pure (define (broken x) :: int -> int
          (+ x "hello")))
    "#);
    assert!(r.is_err(), "adding int + str should fail: {:?}", r);
}
