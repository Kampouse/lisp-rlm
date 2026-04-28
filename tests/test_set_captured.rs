
use lisp_rlm::{lisp_eval, LispVal, Env, EvalState, parse_all};

fn eval(src: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(src).unwrap();
    let mut result = LispVal::Nil;
    for expr in exprs {
        result = lisp_eval(&expr, env, state).unwrap();
    }
    result
}

#[test]
fn test_set_captured_deep_dive() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    // make-counter: (lambda () (set! count (+ count 1)) count)
    // The inner lambda captures 'count' from the let
    eval(r#"(define (make-counter)
      (let ((count 0))
        (lambda ()
          (set! count (+ count 1))
          count)))"#, &mut env, &mut state);
    
    // Check if make-counter compiled
    let mc = env.get("make-counter").unwrap();
    match &mc {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => eprintln!("make-counter COMPILED: {} ops", cl.code.len()),
                None => eprintln!("make-counter NOT compiled"),
            }
        }
        _ => {}
    }
    
    eval(r#"(define c (make-counter))"#, &mut env, &mut state);
    
    // Check if the counter lambda compiled
    let c = env.get("c").unwrap();
    match &c {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => {
                    eprintln!("counter lambda COMPILED: {} ops", cl.code.len());
                    for (i, op) in cl.code.iter().enumerate() {
                        eprintln!("  {:3}: {:?}", i, op);
                    }
                }
                None => eprintln!("counter lambda NOT compiled (tree-walking)"),
            }
        }
        _ => {}
    }
    
    // Test
    let r1 = eval(r#"(c)"#, &mut env, &mut state);
    eprintln!("c() = {:?}", r1);
    assert_eq!(r1, LispVal::Num(1), "first call = 1");
    
    let r2 = eval(r#"(c)"#, &mut env, &mut state);
    eprintln!("c() = {:?}", r2);
    assert_eq!(r2, LispVal::Num(2), "second call = 2");
    
    let r3 = eval(r#"(c)"#, &mut env, &mut state);
    eprintln!("c() = {:?}", r3);
    assert_eq!(r3, LispVal::Num(3), "third call = 3");
}
