
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
fn test_pick_best_deep_dive() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    eval(include_str!("../runtime/harness.lisp"), &mut env, &mut state);
    
    // Check if pick-best compiled
    let pb = env.get("pick-best").unwrap();
    match &pb {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => {
                    eprintln!("pick-best COMPILED: {} ops", cl.code.len());
                    for (i, op) in cl.code.iter().enumerate() {
                        eprintln!("  {:3}: {:?}", i, op);
                    }
                }
                None => eprintln!("pick-best NOT compiled"),
            }
        }
        _ => {}
    }
    
    // Test pick-best with 3 items
    eval(r#"(define items (list (dict "id" "low" "score" 0.3) (dict "id" "high" "score" 0.9) (dict "id" "mid" "score" 0.6)))"#, &mut env, &mut state);
    
    let best = eval(r#"(pick-best items (car items))"#, &mut env, &mut state);
    eprintln!("pick-best result: {:?}", best);
    
    let best_id = eval(r#"(get-default (pick-best items (car items)) "id" "?")"#, &mut env, &mut state);
    eprintln!("best id: {:?}", best_id);
    assert_eq!(best_id, LispVal::Str("high".into()));
}
