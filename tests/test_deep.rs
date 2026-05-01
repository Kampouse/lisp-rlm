use lisp_rlm_wasm::{parser::parse_all, run_program, EvalState, Env};

fn run(code: &str) -> String {
    let forms = parse_all(code).expect("parse");
    let mut env = Env::new();
    let mut state = EvalState::new();
    match run_program(&forms, &mut env, &mut state) {
        Ok(v) => format!("{:?}", v),
        Err(e) => format!("ERROR: {}", e),
    }
}

#[test]
fn test_deep_call() {
    // Test deeply nested computed calls without repeat/compose
    let r = run(r#"
(define f (lambda (x) (* x 2)))
(define g (lambda (x) (+ x 1)))
(((lambda (h) (lambda (x) (h (h x)))) f) 5)
"#);
    eprintln!("result: {}", r);
}
