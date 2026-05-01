//! Tests for lambda execution and higher-order functions.
//!
//! Fixed: map/filter/reduce with lambda arguments were hanging due to
//! duplicate PushClosure opcode in bytecode compiler (line ~1280).
//! The double push corrupted the stack for CallDynamic, causing wrong
//! argument binding in computed function calls.

use lisp_rlm_wasm::EvalState;
use lisp_rlm_wasm::{parse_all, Env, LispVal};

fn eval_val(code: &str) -> LispVal {
    let mut env = Env::new();
    let mut state = EvalState::new();
    for module in &["math", "list", "string"] {
        if let Some(mcode) = lisp_rlm_wasm::get_stdlib_code(module) {
            if let Ok(exprs) = parse_all(mcode) {
                let _ = lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state);
            }
        }
    }
    match parse_all(code) {
        Ok(exprs) => {
            match lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state) {
                Ok(v) => v,
                Err(e) => LispVal::Str(format!("ERROR: {}", e)),
            }
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
fn test_map_with_lambda() {
    // FIXED: was hanging due to duplicate PushClosure opcode
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
fn test_filter_with_lambda() {
    // FIXED: was hanging due to duplicate PushClosure opcode
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
fn test_reduce_with_lambda() {
    // FIXED: was hanging due to duplicate PushClosure opcode
    let result = eval_val("(reduce (lambda (acc x) (+ acc x)) 0 (list 1 2 3 4 5))");
    assert_eq!(result, LispVal::Num(15));
}
