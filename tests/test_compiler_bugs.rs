
use lisp_rlm_wasm::{LispVal, Env, EvalState, parse_all};

fn eval(src: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(src).unwrap();
    lisp_rlm_wasm::program::run_program(&exprs, env, state).unwrap_or_else(|e| panic!("eval error: {}", e))
}

fn eval_per_form(src: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(src).unwrap();
    let mut result = LispVal::Nil;
    for expr in exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr], env, state)
            .unwrap_or_else(|e| panic!("eval error: {}", e));
    }
    result
}

#[test]
fn test_find_best_three_items_compiled() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    // Load harness per-form (large file, avoid batch compilation issues)
    eval_per_form(include_str!("../runtime/harness.lisp"), &mut env, &mut state);

    eval(r#"(define items (list (dict "id" "low" "score" 0.3) (dict "id" "high" "score" 0.9) (dict "id" "mid" "score" 0.6)))"#, &mut env, &mut state);

    let best_id = eval(r#"(get-default (find-best items) "id" "?")"#, &mut env, &mut state);
    assert_eq!(best_id, LispVal::Str("high".into()), "find-best should pick highest score, got {:?}", best_id);
}

#[test]
fn test_pick_best_three_items_compiled() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval_per_form(include_str!("../runtime/harness.lisp"), &mut env, &mut state);

    eval(r#"(define items (list (dict "id" "low" "score" 0.3) (dict "id" "high" "score" 0.9) (dict "id" "mid" "score" 0.6)))"#, &mut env, &mut state);

    let best_id = eval(r#"(get-default (pick-best items (car items)) "id" "?")"#, &mut env, &mut state);
    assert_eq!(best_id, LispVal::Str("high".into()), "pick-best should pick highest score, got {:?}", best_id);
}

#[test]
fn test_recursive_gt_on_dict_get() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval_per_form(include_str!("../runtime/harness.lisp"), &mut env, &mut state);

    eval(r#"(define (my-find-best lst)
      (if (nil? lst) nil
        (if (nil? (cdr lst)) (car lst)
          (let ((head (car lst))
                (tail-best (my-find-best (cdr lst))))
            (let ((hs (dict/get head "score"))
                  (ts (dict/get tail-best "score")))
              (if (> (if (nil? hs) 0 hs) (if (nil? ts) 0 ts))
                head
                tail-best))))))"#, &mut env, &mut state);

    eval(r#"(define data (list (dict "id" "a" "score" 0.3) (dict "id" "b" "score" 0.9) (dict "id" "c" "score" 0.6)))"#, &mut env, &mut state);

    let best_id = eval(r#"(get-default (my-find-best data) "id" "?")"#, &mut env, &mut state);
    assert_eq!(best_id, LispVal::Str("b".into()), "my-find-best should pick b (score 0.9), got {:?}", best_id);
}

#[test]
fn test_set_on_captured_var_compiled() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval(r#"(define (make-counter)
      (let ((count 0))
        (lambda ()
          (set! count (+ count 1))
          count)))"#, &mut env, &mut state);

    eval(r#"(define c (make-counter))"#, &mut env, &mut state);

    let r1 = eval(r#"(c)"#, &mut env, &mut state);
    assert_eq!(r1, LispVal::Num(1), "first call should return 1, got {:?}", r1);

    let r2 = eval(r#"(c)"#, &mut env, &mut state);
    assert_eq!(r2, LispVal::Num(2), "second call should return 2, got {:?}", r2);
}
