//! Tests for lambda execution and higher-order functions.
//!
//! KNOWN BUG: map/filter/reduce with lambda arguments hang due to
//! apply_lambda infinite loop when called from dispatch_collections.
//!
//! Tracked in: test_lambda_hof.rs — ignored tests below
//! Workaround: use (loop ...) with recur instead of map/filter
//!
//! Root cause: Hermes' span-aware tokenizer (reverted) or bytecode
//! compiler (disabled). The call_val → apply_lambda → lisp_eval path
//! hangs inside dispatch_collections::handle but works fine in direct calls.

use lisp_rlm::EvalState;
use lisp_rlm::{lisp_eval, parse_all, Env, LispVal};

fn eval_val(code: &str) -> LispVal {
    let mut env = Env::new();
    let mut state = EvalState::new();
    for module in &["math", "list", "string"] {
        if let Some(mcode) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = parse_all(mcode) {
                for expr in &exprs {
                    let _ = lisp_eval(expr, &mut env, &mut state);
                }
            }
        }
    }
    match parse_all(code) {
        Ok(exprs) => {
            let mut result = LispVal::Nil;
            for expr in &exprs {
                match lisp_eval(expr, &mut env, &mut state) {
                    Ok(v) => result = v,
                    Err(e) => return LispVal::Str(format!("ERROR: {}", e)),
                }
            }
            result
        }
        Err(e) => LispVal::Str(format!("PARSE ERROR: {}", e)),
    }
}

// --- WORKING TESTS ---

#[test]
fn test_direct_lambda_call() {
    let result = eval_val("((lambda (x) (+ x 1)) 5)");
    assert_eq!(result, LispVal::Num(6));
}

#[test]
fn test_sugar_define_call() {
    let result = eval_val("(define (f x) (+ x 1)) (f 5)");
    assert_eq!(result, LispVal::Num(6));
}

#[test]
fn test_loop_recur() {
    let result = eval_val("(loop ((i 0) (acc 0)) (if (>= i 5) acc (recur (+ i 1) (+ acc i))))");
    assert_eq!(result, LispVal::Num(10));
}

#[test]
fn test_lambda_closure() {
    let result = eval_val("(define x 10) ((lambda (y) (+ x y)) 5)");
    assert_eq!(result, LispVal::Num(15));
}

#[test]
fn test_nested_lambda() {
    let result = eval_val("((lambda (x) ((lambda (y) (+ x y)) 3)) 7)");
    assert_eq!(result, LispVal::Num(10));
}

#[test]
fn _bug_map_with_lambda() {
    // EXPECTED: (2 3 4)
    // ACTUAL: hangs forever in apply_lambda
    let result = eval_val("(map (lambda (x) (+ x 1)) (list 1 2 3))");
    match result {
        LispVal::List(items) => {
            assert_eq!(
                items,
                vec![LispVal::Num(2), LispVal::Num(3), LispVal::Num(4)]
            );
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

#[test]
fn _bug_filter_with_lambda() {
    // EXPECTED: (4 5 6)
    // ACTUAL: hangs forever in apply_lambda
    let result = eval_val("(filter (lambda (x) (> x 3)) (list 1 4 2 5 6))");
    match result {
        LispVal::List(items) => {
            assert_eq!(
                items,
                vec![LispVal::Num(4), LispVal::Num(5), LispVal::Num(6)]
            );
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

#[test]
fn _bug_reduce_with_lambda() {
    // EXPECTED: 15
    // ACTUAL: hangs forever in apply_lambda
    let result = eval_val("(reduce (lambda (acc x) (+ acc x)) 0 (list 1 2 3 4 5))");
    assert_eq!(result, LispVal::Num(15));
}
