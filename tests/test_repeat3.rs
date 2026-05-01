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
fn test_repeat_define_only() {
    // Just define repeat, no call
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
42
"#);
    eprintln!("result: {}", r);
}

#[test]
fn test_repeat_call_simple() {
    // (repeat twice) — should return a lambda
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
(repeat twice)
"#);
    eprintln!("result: {}", &r[..r.len().min(100)]);
}
