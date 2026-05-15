use lisp_rlm_wasm::{parse_all, Env, EvalState, run_program};

fn eval(code: &str) -> String {
    let exprs: Vec<_> = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    match run_program(&exprs, &mut env, &mut state) {
        Ok(val) => val.to_string(),
        Err(e) => format!("ERROR: {}", e),
    }
}

#[test]
fn test_named_let_countdown() {
    let result = eval("(let countdown ((n 10)) (if (= n 0) 0 (countdown (- n 1))))");
    println!("named let countdown result: {}", result);
    assert_eq!(result, "0");
}
