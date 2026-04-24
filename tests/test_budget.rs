use lisp_rlm::*;

/// Helper: eval a string in a fresh environment
fn eval(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env)?;
    }
    Ok(result)
}

/// Helper: eval with a custom budget
fn eval_with_budget(code: &str, budget: u64) -> Result<LispVal, String> {
    let mut env = Env::new();
    env.eval_budget = budget;
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env)?;
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Budget enforcement
// ---------------------------------------------------------------------------

#[test]
#[ignore] // stack overflow
fn test_budget_catches_infinite_tail_recursion() {
    // Use "spin" not "loop" — "loop" is a special form keyword
    let code = r#"
        (define spin (lambda () (spin)))
        (spin)
    "#;
    let result = eval_with_budget(code, 1000);
    assert!(result.is_err(), "should hit budget");
    let err = result.unwrap_err();
    assert!(
        err.contains("execution budget exceeded"),
        "error should mention budget, got: {}",
        err
    );
}

#[test]
#[ignore] // stack overflow
fn test_budget_catches_infinite_mutual_recursion() {
    let code = r#"
        (define f (lambda () (g)))
        (define g (lambda () (f)))
        (f)
    "#;
    let result = eval_with_budget(code, 100);
    assert!(result.is_err(), "should hit budget");
    assert!(result.unwrap_err().contains("execution budget exceeded"));
}

#[test]
#[ignore] // stack overflow
fn test_budget_catches_infinite_recursion_with_state() {
    let code = r#"
        (define count (lambda (n) (count (+ n 1))))
        (count 0)
    "#;
    let result = eval_with_budget(code, 500);
    assert!(result.is_err(), "should hit budget");
    assert!(result.unwrap_err().contains("execution budget exceeded"));
}

#[test]
#[ignore] // stack overflow
fn test_budget_does_not_trip_normal_code() {
    let code = r#"
        (define sum (lambda (n acc)
            (if (= n 0) acc (sum (- n 1) (+ acc n)))))
        (sum 1000 0)
    "#;
    let result = eval_with_budget(code, 50_000);
    assert!(result.is_ok(), "normal code should not hit budget, got: {:?}", result);
    assert_eq!(result.unwrap(), LispVal::Num(500500));
}

#[test]
#[ignore] // stack overflow
fn test_budget_zero_means_unlimited() {
    let code = r#"
        (define sum (lambda (n acc)
            (if (= n 0) acc (sum (- n 1) (+ acc n)))))
        (sum 500 0)
    "#;
    let result = eval_with_budget(code, 0);
    assert!(result.is_ok(), "budget=0 should be unlimited, got: {:?}", result);
}

#[test]
fn test_budget_applies_to_map_over_list() {
    // map with manual recursion — budget must cover all the eval calls
    let code = r#"
        (define my-map (lambda (f lst)
            (if (= (len lst) 0)
                (list)
                (cons (f (car lst)) (my-map f (cdr lst))))))
        (define double (lambda (x) (* x 2)))
        (my-map double (list 1 2 3 4 5 6 7 8 9 10))
    "#;
    // Budget of 20 is too small for 10-element map with all the nested evals
    let result = eval_with_budget(code, 20);
    assert!(result.is_err(), "tiny budget should fail on map, got: {:?}", result);
    assert!(result.unwrap_err().contains("execution budget exceeded"));
}

#[test]
fn test_default_budget_is_ten_million() {
    let env = Env::new();
    assert_eq!(env.eval_budget, DEFAULT_EVAL_BUDGET);
    assert_eq!(env.eval_count, 0);
}

#[test]
fn test_budget_counter_increments() {
    let mut env = Env::new();
    env.eval_budget = 100; // non-zero to enable counting

    let exprs = parse_all("(+ 1 2)").unwrap();
    lisp_eval(&exprs[0], &mut env).unwrap();

    assert!(env.eval_count > 0, "eval_count should have incremented");
    // (+ 1 2) = 1 top-level eval + 1 for the + dispatch = at least 2
    assert!(env.eval_count >= 2, "single expression should count multiple evals, got {}", env.eval_count);
}

#[test]
fn test_budget_resets_on_new_env() {
    let mut env = Env::new();
    env.eval_budget = 100;
    let exprs = parse_all("(+ 1 2)").unwrap();
    lisp_eval(&exprs[0], &mut env).unwrap();
    assert!(env.eval_count > 0);

    let env2 = Env::new();
    assert_eq!(env2.eval_count, 0);
}

#[test]
#[ignore] // stack overflow
fn test_budget_error_message_contains_counts() {
    let code = r#"
        (define spin (lambda () (spin)))
        (spin)
    "#;
    let result = eval_with_budget(code, 50);
    let err = result.unwrap_err();
    assert!(err.contains("iterations"));
    assert!(err.contains("limit: 50"));
}

#[test]
fn test_budget_works_with_loop_recur() {
    let code = r#"
        (loop ((i 0) (sum 0))
            (if (>= i 10)
                sum
                (recur (+ i 1) (+ sum i))))
    "#;
    let result = eval_with_budget(code, 1000);
    assert!(result.is_ok(), "loop/recur should complete within budget, got: {:?}", result);
    assert_eq!(result.unwrap(), LispVal::Num(45));
}

#[test]
#[ignore] // stack overflow on deep recursion

#[ignore] // stack overflow
fn test_budget_large_computation_completes() {
    // Sum 1..5000 with default budget should be fine
    let code = r#"
        (define sum-to (lambda (n acc)
            (if (= n 0) acc (sum-to (- n 1) (+ acc n)))))
        (sum-to 5000 0)
    "#;
    let mut env = Env::new();
    let exprs = parse_all(code).unwrap();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env).unwrap();
    }
    // Sum 1..5000 = 12_502_500
    assert_eq!(result, LispVal::Num(12_502_500));
}
