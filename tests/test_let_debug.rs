use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parser::parse_all(code)?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

#[test]
fn test_let_top_level() {
    let r = eval_str("(let ((x 10)) x)");
    match &r {
        Ok(v) => println!("OK: {:?}", v),
        Err(e) => {
            println!("ERR: {}", e);
            // Try wrapping in begin
            let r2 = eval_str("(begin (let ((x 10)) x))");
            match r2 {
                Ok(v) => println!("begin wrap OK: {:?}", v),
                Err(e2) => println!("begin wrap ERR: {}", e2),
            }
        }
    }
    assert!(r.is_ok(), "let at top level should work: {:?}", r);
}

#[test]
fn test_let_as_only_expr_in_lambda() {
    let r = eval_str("((lambda () (let ((x 10)) x)))");
    match &r {
        Ok(v) => println!("lambda-wrap OK: {:?}", v),
        Err(e) => println!("lambda-wrap ERR: {}", e),
    }
    assert!(r.is_ok());
}
