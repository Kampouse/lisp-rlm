use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

#[test]
fn test_closure_set_direct() {
    let r = eval_str("(begin (define x 0) ((lambda () (set! x 42))) x)").unwrap();
    assert_eq!(
        r,
        LispVal::Num(42),
        "direct lambda set! should mutate outer env"
    );
}

#[test]
fn test_closure_set_named() {
    let r = eval_str("(begin (define x 0) (define f (lambda () (set! x 42))) (f) x)").unwrap();
    assert_eq!(
        r,
        LispVal::Num(42),
        "named lambda set! should mutate outer env"
    );
}

#[test]
fn test_closure_set_for_each() {
    let r = eval_str("(begin (define total 0) (for-each (lambda (x) (set! total (+ total x))) (list 1 2 3)) total)").unwrap();
    assert_eq!(r, LispVal::Num(6), "for-each set! should accumulate");
}

#[test]
fn test_closure_set_manual_3_calls() {
    let r = eval_str("(begin (define total 0) (define add (lambda (x) (set! total (+ total x)))) (add 1) (add 2) (add 3) total)").unwrap();
    assert_eq!(r, LispVal::Num(6), "manual 3 calls should accumulate");
}

#[test]
fn test_closure_compose() {
    // compose must still work — closed_env's f/g should take precedence
    let r = eval_str("(begin (define compose (lambda (f g) (lambda (x) (f (g x))))) (define inc (lambda (n) (+ n 1))) (define double (lambda (n) (* n 2))) (define inc-then-double (compose double inc)) (inc-then-double 5))").unwrap();
    assert_eq!(
        r,
        LispVal::Num(12),
        "compose double(inc(5)) = double(6) = 12"
    );
}
