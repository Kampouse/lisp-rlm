use lisp_rlm::{parser::parse_all, run_program, EvalState, Env};

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
fn test_compose_simple() {
    // Basic: compose with direct numbers
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
((compose list twice) 5)
"#);
    eprintln!("result: {}", r);
}

#[test]
fn test_twice_alone() {
    let r = run(r#"
(define twice (lambda (x) (* 2 x)))
(twice 5)
"#);
    eprintln!("result: {}", r);
    assert_eq!(r, "Num(10)");
}
