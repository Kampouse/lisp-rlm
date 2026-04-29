//! Debug test for shadowing fix
use lisp_rlm::EvalState;
use lisp_rlm::*;

fn run_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    run_program(code).unwrap_or_else(|e| format!("ERROR: {}", e))
}

#[test]
fn test_shadow_debug() {
    // Simple case: nested let shadow, body returns inner value
    let r = eval("(let ((x 1)) (let ((x 2)) x))");
    println!("(let ((x 1)) (let ((x 2)) x)) = {}", r);
    assert_eq!(r, "2");

    // Key case: shadow then use outer
    let r2 = eval("(let ((x 1)) (let ((x 2)) x) x)");
    println!("(let ((x 1)) (let ((x 2)) x) x) = {}", r2);
    assert_eq!(r2, "1");  // This is the bug: should be 1 after restore

    // set! shadow case
    let r3 = eval("(map (lambda (x) (let ((x 0)) (set! x 99)) x) (list 1 2 3))");
    println!("map shadow set! = {}", r3);
    assert_eq!(r3, "(1 2 3)");  // After restore, x should be param value
}
