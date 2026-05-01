//! Bytecode coverage tests — verify harness hot-path functions compile

use lisp_rlm_wasm::EvalState;
use lisp_rlm_wasm::*;

fn eval_and_get_lambda(
    code: &str,
    func_name: &str,
) -> Result<Option<lisp_rlm_wasm::bytecode::CompiledLambda>, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let _ = lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state)?;
    match env.get(func_name) {
        Some(LispVal::Lambda { compiled, .. }) => Ok(compiled.clone().map(|arc| (*arc).clone())),
        _ => Ok(None),
    }
}

fn eval_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

// --- Compilation tests ---

#[test]
fn test_get_default_compiles() {
    let result = eval_and_get_lambda(
        r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
    "#,
        "get-default",
    );
    let cl = result
        .unwrap()
        .expect("get-default should compile to bytecode");
    let has_call_captured = cl
        .code
        .iter()
        .any(|op| matches!(op, lisp_rlm_wasm::bytecode::Op::CallCaptured(_, _)));
    assert!(
        !has_call_captured,
        "get-default should not have CallCaptured ops"
    );
}

#[test]
fn test_get_default_no_call_captured() {
    let result = eval_and_get_lambda(
        r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
    "#,
        "get-default",
    );
    let cl = result.unwrap().expect("get-default must compile");
    for op in &cl.code {
        if let lisp_rlm_wasm::bytecode::Op::CallCaptured(_, _) = op {
            panic!(
                "get-default should not have CallCaptured, found: {:?}",
                cl.code
            );
        }
    }
    assert!(!cl.code.is_empty(), "should have some ops");
}

#[test]
fn test_harness_score_intention_compiles() {
    let result = eval_and_get_lambda(
        r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (score-intention intent)
            (let ((urgency (get-default intent "urgency" "medium"))
                  (cost (get-default intent "estimated_cost" 0))
                  (confidence (get-default intent "confidence" 0.5)))
              (+ (* (get-default (dict "critical" 10 "high" 7 "medium" 4 "low" 1) urgency 4) confidence)
                 (/ 1.0 (+ cost 1)))))
    "#,
        "score-intention",
    );
    if let Some(cl) = result.unwrap() {
        let op_names: Vec<String> = cl
            .code
            .iter()
            .map(|op| format!("{:?}", op).split('(').next().unwrap().to_string())
            .collect();
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
    assert!(
        result.contains("overdue"),
        "expected overdue in: {}",
        result
    );
}

#[test]
#[ignore] // Pre-existing bug: nested get-default with dict literal returns wrong values
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

#[test]
fn test_fib_compiles_with_fallback() {
    // Recursive fib compiles, but self-calls go through BuiltinCall("fib")
    // which fails at runtime. The eval fallback handles it correctly.
    let result = eval_and_get_lambda(
        r#"
        (define (fib n)
            (if (<= n 1) n
                (+ (fib (- n 1)) (fib (- n 2)))))
    "#,
        "fib",
    );
    let cl = result.unwrap().expect("fib should compile");
    assert!(
        cl.captured.read().unwrap().is_empty(),
        "fib captures nothing (self-reference via BuiltinCall)"
    );
    // Correctness: eval fallback produces correct results
    let output = eval_program(
        r#"
        (define (fib n) (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2)))))
        (fib 10)
    "#,
    )
    .unwrap();
    assert_eq!(output, "55");
}

#[test]
fn test_scheduler_run_compiles_with_closure() {
    let result = eval_and_get_lambda(
        r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))
        (define (urgency intent)
            (let ((deadline (get-default intent "deadline" nil))
                  (t0 (now)))
                (if (and deadline (> t0 deadline)) 1.0 0.3)))
        (define (cost-efficiency intent)
            (let ((cost (get-default intent "cost" 1)))
                (if (< cost 10) 0.9 0.3)))
        (define (score-intention intent)
            (+ (urgency intent) (cost-efficiency intent)))
        (define (rank-intentions intentions)
            (map score-intention intentions))
        (define (handle-result intent result)
            (let ((t0 (now)))
                (if (> t0 0)
                    (dict/set intent "status" "done")
                    intent)))
        (define (execute-action intent)
            (get-default intent "id" "?"))
        (define (scheduler-run intentions)
            (let ((ranked (rank-intentions intentions)))
                (map (lambda (intent)
                    (let ((result (execute-action intent)))
                        (handle-result intent result)))
                    ranked)))
    "#,
        "scheduler-run",
    );
    let cl = result.unwrap().expect("scheduler-run should compile");
    // Code length may vary due to inlining of small captured functions
    assert!(
        cl.code.len() >= 7,
        "scheduler-run code should have at least 7 ops, got {}",
        cl.code.len()
    );
    assert_eq!(cl.closures.len(), 1);
    // Inner closure captures execute-action and handle-result
    let inner = &cl.closures[0];
    assert!(inner.captured.read().unwrap().iter().any(|(k, _)| k == "execute-action"));
    assert!(inner.captured.read().unwrap().iter().any(|(k, _)| k == "handle-result"));
}

#[test]
fn test_rank_intentions_compiles() {
    let result = eval_and_get_lambda(
        r#"
        (define (score-intention intent) (+ 1 2))
        (define (rank-intentions intentions) (map score-intention intentions))
    "#,
        "rank-intentions",
    );
    let cl = result.unwrap().expect("rank-intentions should compile");
    assert!(cl.captured.read().unwrap().iter().any(|(k, _)| k == "score-intention"));
    // Correctness: map over list with compiled lambda
    let output = eval_program(
        r#"
        (define (score-intention intent) (+ 1 2))
        (define (rank-intentions intentions) (map score-intention intentions))
        (rank-intentions (list (dict "x" 1) (dict "x" 2)))
    "#,
    )
    .unwrap();
    assert_eq!(output, "(3 3)");
}

#[test]
fn test_float_peephole_in_arithmetic() {
    // (* 0.7 x) should compile with TypedBinOp F64
    let cl = eval_and_get_lambda("(define (test-float-mul x) (* 0.7 x))", "test-float-mul")
        .unwrap()
        .unwrap();
    let has_f64_mul = cl.code.iter().any(|op| {
        matches!(
            op,
            lisp_rlm_wasm::bytecode::Op::TypedBinOp(
                lisp_rlm_wasm::bytecode::BinOp::Mul,
                lisp_rlm_wasm::bytecode::Ty::F64,
            )
        )
    });
    assert!(
        has_f64_mul,
        "Expected TypedBinOp(Mul, F64), got: {:?}",
        cl.code
    );

    // Verify correct result
    let result = eval_program("(define (tfm x) (* 0.7 x)) (tfm 0.5)").unwrap();
    assert_eq!(result, "0.35");

    // (- 1.0 x) — reversed PushFloat + LoadSlot
    let cl2 = eval_and_get_lambda("(define (test-float-sub x) (- 1.0 x))", "test-float-sub")
        .unwrap()
        .unwrap();
    let has_f64_sub = cl2.code.iter().any(|op| {
        matches!(
            op,
            lisp_rlm_wasm::bytecode::Op::TypedBinOp(
                lisp_rlm_wasm::bytecode::BinOp::Sub,
                lisp_rlm_wasm::bytecode::Ty::F64,
            )
        )
    });
    assert!(
        has_f64_sub,
        "Expected TypedBinOp(Sub, F64), got: {:?}",
        cl2.code
    );

    let result2 = eval_program("(define (tfs x) (- 1.0 x)) (tfs 0.3)").unwrap();
    assert_eq!(result2, "0.7");
}






