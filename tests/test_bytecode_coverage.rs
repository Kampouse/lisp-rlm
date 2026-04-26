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

fn eval_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

// --- Compilation tests ---

#[test]
fn test_get_default_compiles() {
    let result = eval_and_get_lambda(r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
    "#, "get-default");
    let cl = result.unwrap().expect("get-default should compile to bytecode");
    let has_call_captured = cl.code.iter().any(|op| matches!(op, lisp_rlm::bytecode::Op::CallCaptured(_, _)));
    assert!(!has_call_captured, "get-default should not have CallCaptured ops");
}

#[test]
fn test_get_default_no_call_captured() {
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
    assert!(!cl.code.is_empty(), "should have some ops");
}

#[test]
fn test_harness_score_intention_compiles() {
    let result = eval_and_get_lambda(r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (score-intention intent)
            (let ((urgency (get-default intent "urgency" "medium"))
                  (cost (get-default intent "estimated_cost" 0))
                  (confidence (get-default intent "confidence" 0.5)))
              (+ (* (get-default (dict "critical" 10 "high" 7 "medium" 4 "low" 1) urgency 4) confidence)
                 (/ 1.0 (+ cost 1)))))
    "#, "score-intention");
    if let Some(cl) = result.unwrap() {
        let op_names: Vec<String> = cl.code.iter().map(|op| format!("{:?}", op).split('(').next().unwrap().to_string()).collect();
        eprintln!("score-intention ops: {:?}", op_names);
    }
}

#[test]
fn test_inner_lambda_compiles_in_filter() {
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
    let result = eval_program(code).unwrap();
    assert_eq!(result, "(5 3)");
}

// --- Functional tests ---

#[test]
fn test_urgency_compiles_and_works() {
    let code = r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (urgency intent)
            (let ((deadline (get-default intent "deadline" nil))
                  (last (get-default intent "last-acted" nil))
                  (t0 (now)))
                (cond
                    ((and deadline (> t0 deadline)) 1.0)
                    ((and deadline (< (- deadline t0) 3600000)) 0.9)
                    ((and last (> (elapsed last) 3600000)) 0.7)
                    (t 0.3))))
        (list (urgency (dict "deadline" 1)) (urgency (dict)))
    "#;
    let result = eval_program(code).unwrap();
    // deadline=1 is in the past → 1.0; no deadline → 0.3
    assert_eq!(result, "(1 0.3)");
}

#[test]
fn test_cost_efficiency_compiles_and_works() {
    let code = r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (cost-efficiency intent)
            (let ((cost (get-default intent "cost" 1)))
                (cond
                    ((= cost 0) 1.0)
                    ((< cost 10) 0.9)
                    ((< cost 100) 0.6)
                    (t 0.3))))
        (list (cost-efficiency (dict "cost" 0))
              (cost-efficiency (dict "cost" 5))
              (cost-efficiency (dict "cost" 50))
              (cost-efficiency (dict "cost" 500)))
    "#;
    let result = eval_program(code).unwrap();
    assert_eq!(result, "(1 0.9 0.6 0.3)");
}

#[test]
fn test_full_harness_scoring() {
    let code = r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (urgency intent)
            (let ((deadline (get-default intent "deadline" nil))
                  (last (get-default intent "last-acted" nil))
                  (t0 (now)))
                (cond
                    ((and deadline (> t0 deadline)) 1.0)
                    ((and deadline (< (- deadline t0) 3600000)) 0.9)
                    ((and last (> (elapsed last) 3600000)) 0.7)
                    (t 0.3))))
        (define (cost-efficiency intent)
            (let ((cost (get-default intent "cost" 1)))
                (cond
                    ((= cost 0) 1.0)
                    ((< cost 10) 0.9)
                    ((< cost 100) 0.6)
                    (t 0.3))))
        (define (score-intention intent)
            (let ((u (urgency intent))
                  (e (cost-efficiency intent))
                  (score (+ (* 0.7 u) (* 0.3 e))))
                (dict/set intent "score" score)))
        (define intentions
            (list
                (dict "id" "overdue" "deadline" 1 "cost" 5)
                (dict "id" "cheap" "cost" 0)
                (dict "id" "normal")))
        (map score-intention intentions)
    "#;
    let result = eval_program(code).unwrap();
    assert!(result.contains("score"), "expected score in: {}", result);
    assert!(result.contains("overdue"), "expected overdue in: {}", result);
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
    let result = eval_program(code).unwrap();
    assert_eq!(result, "(10 1 7)");
}


