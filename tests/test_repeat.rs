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
fn test_repeat_repeat_step1() {
    // Step 1: just define repeat, no nested call
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
repeat
"#);
    eprintln!("repeat: {}", r);
}

#[test]
fn test_repeat_repeat_step2() {
    // Step 2: (repeat twice) — returns a lambda
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
(repeat twice)
"#);
    eprintln!("repeat(twice): {}", r);
}

#[test]
fn test_repeat_repeat_step3() {
    // Step 3: ((repeat twice) 5) — should be 20
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
((repeat twice) 5)
"#);
    eprintln!("repeat(twice)(5): {}", r);
}

#[test]
fn test_repeat_repeat_step4() {
    // Step 4: (repeat (repeat twice)) — returns a lambda
    let r = run(r#"
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(define twice (lambda (x) (* 2 x)))
(define repeat (lambda (f) (compose f f)))
(repeat (repeat twice))
"#);
    eprintln!("repeat(repeat(twice)): {}", r);
}
