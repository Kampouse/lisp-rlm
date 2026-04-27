//! Minimal shadowing test
use lisp_rlm::EvalState;
use lisp_rlm::*;

fn eval(code: &str) -> String {
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state).unwrap();
    }
    result.to_string()
}

#[test]
fn test_shadow_simple() {
    let r = eval("(map (lambda (x) (let ((x 0)) (set! x 99)) x) (list 1 2 3))");
    eprintln!("RESULT: {}", r);
    assert_eq!(r, "(1 2 3)");
}
