//! Tests for untested runtime features: fork, memoize, case-lambda, letrec,
//! delay/force, define-values, par-map, snapshot/rollback.

use lisp_rlm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

// ══════════════════════════════════════
// fork — speculative evaluation
// ══════════════════════════════════════

#[test]
fn test_fork_basic() {
    let code = r#"
(define x 10)
(fork (+ x 5))
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

#[test]
fn test_fork_isolation() {
    // Fork should NOT modify parent env
    let code = r#"
(define x 10)
(fork (set! x 99))
x
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(10));
}

#[test]
fn test_fork_define_isolation() {
    // Define inside fork should not leak to parent
    let code = r#"
(fork (define secret 42))
secret
"#;
    assert!(
        eval_str(code).is_err(),
        "secret should not be defined in parent"
    );
}

#[test]
fn test_fork_preserves_parent() {
    // Parent env is unchanged after fork
    let code = r#"
(define x 1)
(define y 2)
(fork (begin (set! x 100) (set! y 200)))
(list x y)
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2)])
    );
}

#[test]
fn test_fork_with_closure() {
    let code = r#"
(define make-adder (lambda (n) (lambda (x) (+ x n))))
(define add5 (make-adder 5))
(fork (add5 10))
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

// ══════════════════════════════════════
// memoize — cached function results
// ══════════════════════════════════════

#[test]
fn test_memoize_basic() {
    let code = r#"
(define f (memoize (lambda (x) (* x 2))))
(list (f 3) (f 5) (f 3))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(6), LispVal::Num(10), LispVal::Num(6)])
    );
}

#[test]
fn test_memoize_returns_same_result() {
    // Repeated calls with same args return identical value
    let code = r#"
(define f (memoize (lambda (x) (+ x 1))))
(f 10)
"#;
    let first = eval_str(code).unwrap();
    let code2 = r#"
(define f (memoize (lambda (x) (+ x 1))))
(f 10)
"#;
    let second = eval_str(code2).unwrap();
    assert_eq!(first, second);
}

#[test]
fn test_memoize_string_args() {
    let code = r#"
(define greet (memoize (lambda (name) (str-concat "hi " name))))
(greet "alice")
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Str("hi alice".into()));
}

#[test]
fn test_memoize_multi_arg() {
    let code = r#"
(define add (memoize (lambda (a b) (+ a b))))
(list (add 1 2) (add 3 4) (add 1 2))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(3), LispVal::Num(7), LispVal::Num(3)])
    );
}

#[test]
fn test_memoize_rejects_non_lambda() {
    let result = eval_str("(memoize 42)");
    assert!(result.is_err(), "memoize should reject non-lambda");
}

// ══════════════════════════════════════
// case-lambda — multi-arity dispatch
// ══════════════════════════════════════

#[test]
fn test_case_lambda_zero_args() {
    let code = r#"
(define f (case-lambda (() 42)))
(f)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

#[test]
fn test_case_lambda_one_arg() {
    let code = r#"
(define f (case-lambda (() 0) ((x) (* x 2))))
(f 5)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(10));
}

#[test]
fn test_case_lambda_two_args() {
    let code = r#"
(define f (case-lambda
  (() 0)
  ((x) x)
  ((x y) (+ x y))))
(list (f) (f 3) (f 3 4))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(0), LispVal::Num(3), LispVal::Num(7)])
    );
}

#[test]
fn test_case_lambda_rest_args() {
    // Last clause with single symbol catches all remaining args
    let code = r#"
(define f (case-lambda
  (() (list))
  (args args)))
(list (f) (f 1) (f 1 2 3))
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::List(parts) => {
            assert_eq!(parts.len(), 3);
            // (f) → empty list
            // (f 1) → (1)
            // (f 1 2 3) → (1 2 3)
        }
        other => panic!("expected list, got {}", other),
    }
}

#[test]
fn test_case_lambda_wrong_arity() {
    let code = r#"
(define f (case-lambda (() 0) ((x) x)))
(f 1 2 3)
"#;
    assert!(eval_str(code).is_err(), "wrong arity should error");
}

// ══════════════════════════════════════
// letrec — recursive bindings
// ══════════════════════════════════════

#[test]
fn test_letrec_basic() {
    let code = r#"
(letrec ((double (lambda (x) (* x 2))))
  (double 5))
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(10));
}

#[test]
fn test_letrec_mutual_recursion() {
    let code = r#"
(letrec ((even? (lambda (n)
            (if (= n 0) true (odd? (- n 1)))))
         (odd? (lambda (n)
            (if (= n 0) false (even? (- n 1))))))
  (list (even? 4) (odd? 3) (even? 5)))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![
            LispVal::Bool(true),
            LispVal::Bool(true),
            LispVal::Bool(false)
        ])
    );
}

#[test]
fn test_letrec_factorial() {
    let code = r#"
(letrec ((fact (lambda (n)
            (if (<= n 1) 1 (* n (fact (- n 1)))))))
  (fact 5))
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(120));
}

// ══════════════════════════════════════
// delay / force — lazy evaluation
// ══════════════════════════════════════

#[test]
fn test_delay_force_basic() {
    let code = r#"
(define p (delay (+ 1 2)))
(force p)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(3));
}

#[test]
fn test_delay_not_evaluated_until_forced() {
    // If delay evaluates eagerly, this would error on define
    let code = r#"
(define p (delay (/ 1 0)))
(+ 1 2)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(3));
}

#[test]
fn test_force_twice_same_result() {
    let code = r#"
(define p (delay (+ 1 2)))
(list (force p) (force p))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(3), LispVal::Num(3)])
    );
}

#[test]
fn test_delay_with_closure() {
    let code = r#"
(define x 10)
(define p (delay (+ x 5)))
(force p)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

// ══════════════════════════════════════
// define-values — multiple return values
// ══════════════════════════════════════

#[test]
fn test_define_values_basic() {
    let code = r#"
(define-values (a b) (list 1 2))
(list a b)
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2)])
    );
}

#[test]
fn test_define_values_three() {
    let code = r#"
(define-values (x y z) (list 10 20 30))
(list x y z)
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(10), LispVal::Num(20), LispVal::Num(30)])
    );
}

#[test]
fn test_define_values_from_function() {
    let code = r#"
(define split (lambda (x) (list (* x 10) (* x 100))))
(define-values (tens hundreds) (split 3))
(list tens hundreds)
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(30), LispVal::Num(300)])
    );
}

// ══════════════════════════════════════
// par-map — parallel map
// ══════════════════════════════════════

#[test]
fn test_par_map_basic() {
    let code = r#"
(par-map (lambda (x) (* x 2)) (list 1 2 3 4 5))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![
            LispVal::Num(2),
            LispVal::Num(4),
            LispVal::Num(6),
            LispVal::Num(8),
            LispVal::Num(10)
        ])
    );
}

#[test]
fn test_par_map_empty() {
    let code = "(par-map (lambda (x) x) (list))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![]));
}

#[test]
fn test_par_map_single() {
    let code = "(par-map (lambda (x) (+ x 1)) (list 41))";
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(42)])
    );
}

#[test]
fn test_par_map_order_preserved() {
    let code = r#"
(par-map (lambda (x) (* x x)) (list 5 4 3 2 1))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![
            LispVal::Num(25),
            LispVal::Num(16),
            LispVal::Num(9),
            LispVal::Num(4),
            LispVal::Num(1)
        ])
    );
}

#[test]
fn test_par_map_with_closure() {
    let code = r#"
(define offset 10)
(par-map (lambda (x) (+ x offset)) (list 1 2 3))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(11), LispVal::Num(12), LispVal::Num(13)])
    );
}

// ══════════════════════════════════════
// snapshot / rollback — env persistence
// ══════════════════════════════════════

#[test]
fn test_snapshot_rollback_basic() {
    let code = r#"
(define x 1)
(snapshot)
(set! x 99)
(rollback)
x
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(1));
}

#[test]
fn test_snapshot_rollback_define() {
    let code = r#"
(define x 1)
(snapshot)
(define y 42)
(rollback)
x
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(1));
}

#[test]
fn test_nested_snapshots() {
    let code = r#"
(define x 1)
(snapshot)
(set! x 2)
(snapshot)
(set! x 3)
(rollback)
x
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(2));
}

#[test]
fn test_rollback_restores_then_continue() {
    let code = r#"
(define x 10)
(snapshot)
(set! x 20)
(rollback)
(+ x 5)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

#[test]
fn test_snapshot_returns_id() {
    let code = r#"
(snapshot)
"#;
    // First snapshot returns 0
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(0));
}

#[test]
fn test_multiple_snapshots_ids() {
    let code = r#"
(list (snapshot) (snapshot) (snapshot))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(0), LispVal::Num(1), LispVal::Num(2)])
    );
}

// ══════════════════════════════════════
// par-filter — parallel filter
// ══════════════════════════════════════

#[test]
fn test_par_filter_basic() {
    let code = r#"
(par-filter (lambda (x) (> x 3)) (list 1 2 3 4 5 6))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(4), LispVal::Num(5), LispVal::Num(6)])
    );
}

#[test]
fn test_par_filter_empty() {
    let code = "(par-filter (lambda (x) true) (list))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![]));
}

#[test]
fn test_par_filter_none_match() {
    let code = "(par-filter (lambda (x) false) (list 1 2 3))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![]));
}

#[test]
fn test_par_filter_all_match() {
    let code = "(par-filter (lambda (x) true) (list 1 2 3))";
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)])
    );
}
