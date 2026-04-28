
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
fn test_find_best_many_items() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    eval(include_str!("../runtime/harness.lisp"), &mut env, &mut state);
    
    // 10 items, best is in the middle
    eval(r#"(define items (list 
        (dict "id" "a" "score" 0.1)
        (dict "id" "b" "score" 0.2)
        (dict "id" "c" "score" 0.3)
        (dict "id" "d" "score" 0.95)
        (dict "id" "e" "score" 0.5)
        (dict "id" "f" "score" 0.6)
        (dict "id" "g" "score" 0.7)
        (dict "id" "h" "score" 0.8)
        (dict "id" "i" "score" 0.4)
        (dict "id" "j" "score" 0.15)))"#, &mut env, &mut state);
    
    let best_id = eval(r#"(get-default (find-best items) "id" "?")"#, &mut env, &mut state);
    eprintln!("find-best 10 items: {:?}", best_id);
    assert_eq!(best_id, LispVal::Str("d".into()), "should pick d (0.95)");
    
    let best_id2 = eval(r#"(get-default (pick-best items (car items)) "id" "?")"#, &mut env, &mut state);
    eprintln!("pick-best 10 items: {:?}", best_id2);
    assert_eq!(best_id2, LispVal::Str("d".into()), "should pick d (0.95)");
}

#[test]
fn test_find_best_scores_near_zero() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    eval(include_str!("../runtime/harness.lisp"), &mut env, &mut state);
    
    // Scores all below 1.0 — the old num_val() truncation bug would fail here
    eval(r#"(define items (list 
        (dict "id" "a" "score" 0.1)
        (dict "id" "b" "score" 0.05)
        (dict "id" "c" "score" 0.08)))"#, &mut env, &mut state);
    
    let best_id = eval(r#"(get-default (find-best items) "id" "?")"#, &mut env, &mut state);
    eprintln!("find-best near-zero: {:?}", best_id);
    assert_eq!(best_id, LispVal::Str("a".into()), "should pick a (0.1)");
}

#[test] 
fn test_set_on_global_via_harness() {
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    eval(include_str!("../runtime/harness.lisp"), &mut env, &mut state);
    
    // budget-spend does set! on *budget* global
    eval(r#"(budget-spend 50)"#, &mut env, &mut state);
    let used = eval(r#"(if (nil? (dict/get *budget* "used")) 0 (dict/get *budget* "used"))"#, &mut env, &mut state);
    eprintln!("budget used: {:?}", used);
    assert_eq!(used, LispVal::Num(50));
    
    eval(r#"(budget-spend 30)"#, &mut env, &mut state);
    let used2 = eval(r#"(if (nil? (dict/get *budget* "used")) 0 (dict/get *budget* "used"))"#, &mut env, &mut state);
    eprintln!("budget used after 2nd: {:?}", used2);
    assert_eq!(used2, LispVal::Num(80));
}
