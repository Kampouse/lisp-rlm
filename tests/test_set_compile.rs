
use lisp_rlm_wasm::{LispVal, Env, EvalState, parse_all};

fn eval(src: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(src).unwrap();
    let mut result = LispVal::Nil;
    for expr in exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr.clone()], env, state).unwrap();
    }
    result
}

#[test]
fn test_what_compiles_with_set_captured() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    // A lambda that does set! on its own param -- should compile
    eval(r#"(define (inc! x) (set! x (+ x 1)) x)"#, &mut env, &mut state);
    let inc = env.get("inc!").unwrap();
    match &inc {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => eprintln!("inc! (set! on param) COMPILED: {} ops", cl.code.len()),
                None => eprintln!("inc! NOT compiled"),
            }
        }
        _ => {}
    }
    
    // A lambda that does set! on a let var -- should compile  
    eval(r#"(define (test-local-set) (let ((x 0)) (set! x 42) x))"#, &mut env, &mut state);
    let tls = env.get("test-local-set").unwrap();
    match &tls {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => eprintln!("test-local-set (set! on let var) COMPILED: {} ops", cl.code.len()),
                None => eprintln!("test-local-set NOT compiled"),
            }
        }
        _ => {}
    }
    
    // A lambda that does set! on a captured var -- should NOT compile
    eval(r#"(define (make-adder base) (lambda (x) (set! base (+ base x)) base))"#, &mut env, &mut state);
    let ma = env.get("make-adder").unwrap();
    match &ma {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => eprintln!("make-adder (outer) COMPILED: {} ops", cl.code.len()),
                None => eprintln!("make-adder NOT compiled"),
            }
        }
        _ => {}
    }
    
    eval(r#"(define adder (make-adder 10))"#, &mut env, &mut state);
    let adder = env.get("adder").unwrap();
    match &adder {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => eprintln!("adder (inner, set! on captured) COMPILED: {} ops", cl.code.len()),
                None => eprintln!("adder (inner, set! on captured) NOT compiled - correct fallback"),
            }
        }
        _ => {}
    }
    
    // But it should still work correctly via tree-walking
    let r1 = eval(r#"(adder 5)"#, &mut env, &mut state);
    eprintln!("adder(5) = {:?} (should be 15)", r1);
    assert_eq!(r1, LispVal::Num(15));
    
    let r2 = eval(r#"(adder 3)"#, &mut env, &mut state);
    eprintln!("adder(3) = {:?} (should be 18)", r2);
    assert_eq!(r2, LispVal::Num(18));
}
