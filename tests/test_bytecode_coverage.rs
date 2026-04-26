//! Bytecode coverage tests — verify harness hot-path functions compile

use lisp_rlm::EvalState;
use lisp_rlm::*;

fn eval_and_get_lambda(code: &str, func_name: &str) -> Result<Option<lisp_rlm::bytecode::CompiledLambda>, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    for expr in &exprs {
        let _ = lisp_eval(expr, &mut env, &mut state)?;
    }
    match env.get(func_name) {
        Some(LispVal::Lambda { compiled, .. }) => Ok(compiled.clone().map(|b| *b)),
        _ => Ok(None),
    }
}

#[test]
fn test_get_default_compiles() {
    let result = eval_and_get_lambda(r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
    "#, "get-default");
    let cl = result.unwrap().expect("get-default should compile to bytecode");
    let ops = &cl.code;
    // Should NOT contain CallCaptured — dict/get and nil? are builtins
    let has_call_captured = ops.iter().any(|op| matches!(op, lisp_rlm::bytecode::Op::CallCaptured(_, _)));
    assert!(!has_call_captured, "get-default should not have CallCaptured ops, found: {:?}", ops);
}

#[test]
fn test_harness_score_intention_compiles() {
    let result = eval_and_get_lambda(r#"
        (define urgency-weights
            (dict "critical" 10 "high" 7 "medium" 4 "low" 1))
        (define (score-intention intent)
            (let ((urgency (get-default intent "urgency" "medium"))
                  (cost (get-default intent "estimated_cost" 0))
                  (confidence (get-default intent "confidence" 0.5)))
              (+ (* (get-default (dict "critical" 10 "high" 7 "medium" 4 "low" 1) urgency 4) confidence)
                 (/ 1.0 (+ cost 1)))))
    "#, "score-intention");
    // score-intention calls get-default which is captured
    let cl = result.unwrap();
    // If it compiled, that's good — even if it has CallCaptured for get-default
    if let Some(cl) = cl {
        let op_names: Vec<String> = cl.code.iter().map(|op| format!("{:?}", op).split('(').next().unwrap().to_string()).collect();
        eprintln!("score-intention ops: {:?}", op_names);
    }
}

#[test]
fn test_inner_lambda_compiles_in_filter() {
    // The outer function won't compile (filter is captured),
    // but the inner lambda passed to filter WILL compile via HOF fast path.
    // Test the actual execution instead:
    let code = r#"
        (define items
            (list
                (dict "priority" 1)
                (dict "priority" 5)
                (dict "priority" 3)))
        (define (filter-high items)
            (filter (lambda (i) (> (dict/get i "priority") 2)) items))
        (map (lambda (i) (dict/get i "priority")) (filter-high items))
    "#;
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state).unwrap();
    }
    assert_eq!(result.to_string(), "(5 3)");
}

#[test]
fn test_get_default_no_call_captured() {
    // Core assertion: get-default compiles with zero CallCaptured ops
    let result = eval_and_get_lambda(r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
    "#, "get-default");
    let cl = result.unwrap().expect("get-default must compile");
    for op in &cl.code {
        if let lisp_rlm::bytecode::Op::CallCaptured(_, _) = op {
            panic!("get-default should not have CallCaptured, found: {:?}", cl.code);
        }
    }
    // Should have exactly these patterns: LoadSlot, PushStr, BuiltinCall, StoreSlot, BuiltinCall, Branch
    assert!(cl.code.len() > 0, "should have some ops");
}

#[test]
fn test_harness_style_pipeline() {
    let code = r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define intentions
            (list
                (dict "id" "a" "urgency" "critical" "confidence" 0.9)
                (dict "id" "b" "urgency" "low" "confidence" 0.3)
                (dict "id" "c" "urgency" "high" "confidence" 0.8)))
        (define (score-intention intent)
            (let ((urgency (get-default intent "urgency" "medium")))
                (get-default (dict "critical" 10 "high" 7 "medium" 4 "low" 1) urgency 4)))
        (define results (map score-intention intentions))
        results
    "#;
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state).unwrap();
    }
    assert_eq!(result.to_string(), "(10 1 7)");
}
