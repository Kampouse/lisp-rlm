use lisp_rlm_wasm::{parser::parse_all, run_program, Env, EvalState, types::LispVal};

fn run(code: &str) -> String {
    let forms = parse_all(code).expect("parse");
    let mut env = Env::new();
    let mut state = EvalState::new();
    match run_program(&forms, &mut env, &mut state) {
        Ok(v) => {
            // Avoid stack overflow from Debug formatting deeply nested lambdas
            let s = match &v {
                LispVal::Lambda { .. } => "Lambda(...)".to_string(),
                other => format!("{:?}", other),
            };
            s
        }
        Err(e) => format!("ERROR: {}", e),
    }
}

#[test]
fn test_repeat_repeat_minimal() {
    // Minimal: just (repeat (repeat twice)) with no call
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
(repeat (repeat twice))
"#);
    eprintln!("result: {}", &r[..r.len().min(200)]);
}
