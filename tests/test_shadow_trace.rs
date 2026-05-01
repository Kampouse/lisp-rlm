use lisp_rlm_wasm::EvalState;
use lisp_rlm_wasm::*;

fn eval(code: &str) -> String {
    let exprs = parse_all(code).unwrap();
    eprintln!("Parsed: {:?}", exprs[0]);
    let mut env = Env::new();
    let mut state = EvalState::new();
    let r = lisp_rlm_wasm::program::run_program(&exprs[..1], &mut env, &mut state);
    format!("{:?}", r)
}

#[test]
fn test_trace() {
    let r3 = eval("(let ((x 1)) (let ((x 2)) x) x)");
    eprintln!("result = {}", r3);
    assert_eq!(r3, "Ok(Num(1))");
}
