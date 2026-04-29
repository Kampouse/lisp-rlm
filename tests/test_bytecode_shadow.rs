//! Direct bytecode compiler test for shadowing fix
use lisp_rlm::*;

#[test]
fn test_simple_shadow() {
    let exprs = parse_all("(lambda (x) (let ((x 42)) x))").unwrap();
    let mut env = Env::new();
    if let LispVal::Lambda { compiled, body, .. } = lisp_rlm::program::run_program(&exprs[0..1], &mut env, &mut EvalState::new()).unwrap() {
        eprintln!("Simple body: {:?}", body);
        match compiled {
            Some(cl) => eprintln!("Simple shadow compiled: {:?}", cl.code),
            None => eprintln!("Simple shadow NOT compiled"),
        }
    }
}

#[test]
fn test_shadow_restore() {
    let exprs = parse_all("(lambda (x) (let ((x 0)) (set! x 99)) x)").unwrap();
    let mut env = Env::new();
    if let LispVal::Lambda { compiled, body, .. } = lisp_rlm::program::run_program(&exprs[0..1], &mut env, &mut EvalState::new()).unwrap() {
        eprintln!("Restore body: {:?}", body);
        match compiled {
            Some(cl) => {
                eprintln!("Restore compiled! code: {:?}", cl.code);
                eprintln!("total_slots: {}", cl.total_slots);
            }
            None => eprintln!("Restore NOT compiled - falls back to CPS"),
        }
    }
    
    // Also test evaluation
    let r = lisp_rlm::program::run_program(&parse_all("(map (lambda (x) (let ((x 0)) (set! x 99)) x) (list 1 2 3))").unwrap(), &mut Env::new(), &mut EvalState::new()).unwrap();
    eprintln!("map result: {}", r);
}
