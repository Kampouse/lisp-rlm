
use lisp_rlm::{LispVal, Env, EvalState, parse_all};

fn eval(src: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(src).unwrap();
    let mut result = LispVal::Nil;
    for expr in exprs {
        result = lisp_rlm::program::run_program(&[expr.clone()], env, state).unwrap();
    }
    result
}

#[test]
fn test_find_best_compiled_status() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    // Load harness
    eval(include_str!("../runtime/harness.lisp"), &mut env, &mut state);
    
    // Check if find-best has compiled bytecode
    let fb = env.get("find-best").unwrap();
    match &fb {
        LispVal::Lambda { compiled, .. } => {
            match compiled {
                Some(cl) => {
                    eprintln!("find-best COMPILED: {} ops", cl.code.len());
                    for (i, op) in cl.code.iter().enumerate() {
                        eprintln!("  {:3}: {:?}", i, op);
                    }
                    // Now test with actual values
                }
                None => eprintln!("find-best NOT compiled (tree-walking)"),
            }
        }
        _ => eprintln!("find-best is not a lambda"),
    }
    
    // Test with actual data
    eval(r#"(define items (list (dict "id" "low" "score" 0.3) (dict "id" "high" "score" 0.9) (dict "id" "mid" "score" 0.6)))"#, &mut env, &mut state);
    
    let best = eval(r#"(find-best items)"#, &mut env, &mut state);
    eprintln!("find-best result: {:?}", best);
    
    let best_id = eval(r#"(get-default (find-best items) "id" "?")"#, &mut env, &mut state);
    eprintln!("best id: {:?}", best_id);
    assert_eq!(best_id, LispVal::Str("high".into()));
}
