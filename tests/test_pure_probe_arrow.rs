//! Tests for pure flag, type probing (infer-type), and higher-order contracts (Arrow types).

use lisp_rlm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result)
}

// ══════════════════════════════════════
// Pure flag on lambdas
// ══════════════════════════════════════

#[test]
fn test_pure_lambda_has_type_annotation() {
    // pure lambdas should carry their inferred type
    let code = r#"
(pure (define (inc x) (+ x 1)))
(pure-type inc)
"#;
    let result = eval_str(code).unwrap();
    // Should return a string with the type signature, not nil
    match result {
        LispVal::Str(s) => {
            assert!(s.contains("int") || s.contains("num"), "pure type should mention int/num, got: {}", s);
        }
        other => panic!("expected string, got {}", other),
    }
}

#[test]
fn test_regular_lambda_no_pure_type() {
    let code = r#"
(define (inc x) (+ x 1))
(pure-type inc)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Nil);
}

#[test]
fn test_pure_lambda_still_executes_correctly() {
    let code = r#"
(pure (define (double x) (* x 2)))
(double 21)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

#[test]
fn test_pure_lambda_composes() {
    let code = r#"
(pure (define (inc x) (+ x 1)))
(pure (define (double x) (* x 2)))
(double (inc 5))
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(12));
}

// ══════════════════════════════════════
// Type probing (infer-type)
// ══════════════════════════════════════

#[test]
fn test_infer_type_arithmetic() {
    // (+ x 1) should accept ints, return int
    let code = r#"
(define (inc x) (+ x 1))
(infer-type inc)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Str(sig) => {
            assert!(sig.contains("int") || sig.contains(":int"), "should mention int, got: {}", sig);
        }
        other => panic!("expected string, got {}", other),
    }
}

#[test]
fn test_infer_type_string_function() {
    // str-concat only works with strings
    let code = r#"
(define (greet name) (str-concat "hi " name))
(infer-type greet)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Str(sig) => {
            assert!(sig.contains("str") || sig.contains(":str"), "should mention str, got: {}", sig);
        }
        other => panic!("expected string, got {}", other),
    }
}

#[test]
fn test_infer_type_identity() {
    // (lambda (x) x) should accept anything
    let code = r#"
(define id (lambda (x) x))
(infer-type id)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Str(sig) => {
            // Should have :any or wide union for param type
            assert!(sig.contains("any") || sig.contains("→"), "should have signature, got: {}", sig);
        }
        other => panic!("expected string, got {}", other),
    }
}

#[test]
fn test_infer_type_on_pure_returns_cached() {
    // For pure lambdas, infer-type should return the cached type immediately
    let code = r#"
(pure (define (square x) (* x x)))
(infer-type square)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Str(sig) => {
            // Pure type is the HM-inferred type string
            assert!(!sig.is_empty(), "should have type info");
        }
        other => panic!("expected string, got {}", other),
    }
}

#[test]
fn test_infer_type_rejects_non_lambda() {
    assert!(eval_str("(infer-type 42)").is_err());
    assert!(eval_str("(infer-type \"hello\")").is_err());
}

#[test]
fn test_infer_type_no_args() {
    // Lambda with no params
    let code = r#"
(define (answer) 42)
(infer-type answer)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Str(sig) => {
            assert!(sig.contains("int") || sig.contains("→"), "should mention int or arrow, got: {}", sig);
        }
        other => panic!("expected string, got {}", other),
    }
}

// ══════════════════════════════════════
// Higher-order contracts (Arrow types)
// ══════════════════════════════════════

#[test]
fn test_parse_arrow_type() {
    // Parsing (:fn :int → :int) should produce an Arrow type
    let result = eval_str("(valid-type? (:fn :int → :int))");
    assert!(result.is_ok(), "arrow type should be valid");
    match result.unwrap() {
        LispVal::Str(s) => {
            assert!(s.contains("fn"), "should format as fn type, got: {}", s);
        }
        other => panic!("expected string from valid-type?, got {}", other),
    }
}

#[test]
fn test_parse_arrow_type_multi_param() {
    let result = eval_str("(valid-type? (:fn :int :str → :bool))");
    assert!(result.is_ok());
}

#[test]
fn test_arrow_matches_lambda() {
    // A lambda value should match an arrow type
    assert!(
        eval_str("(check (lambda (x) x) (:fn :int → :int))").is_ok(),
        "lambda should match arrow type"
    );
}

#[test]
fn test_arrow_rejects_non_function() {
    assert!(
        eval_str("(check 42 (:fn :int → :int))").is_err(),
        "number should not match arrow type"
    );
    assert!(
        eval_str("(check \"hello\" (:fn :int → :int))").is_err(),
        "string should not match arrow type"
    );
}

#[test]
fn test_arrow_matches_pure_lambda() {
    let code = r#"
(pure (define (inc x) (+ x 1)))
(check inc (:fn :int → :int))
"#;
    assert!(eval_str(code).is_ok(), "pure lambda should match arrow type");
}

#[test]
fn test_arrow_matches_memoized() {
    let code = r#"
(define f (memoize (lambda (x) (* x 2))))
(check f (:fn :int → :int))
"#;
    assert!(eval_str(code).is_ok(), "memoized fn should match arrow type");
}

#[test]
fn test_arrow_in_contract_param() {
    // A contract that takes a function parameter
    let code = r#"
(define apply-fn
  (contract ((f (:fn :int → :int)) (x :int) -> :int)
    (f x)))
(apply-fn (lambda (n) (+ n 1)) 5)
"#;
    let result = eval_str(code);
    assert!(result.is_ok(), "contract with arrow param should work, got: {:?}", result);
    assert_eq!(result.unwrap(), LispVal::Num(6));
}

#[test]
fn test_arrow_contract_rejects_non_fn() {
    let code = r#"
(define apply-fn
  (contract ((f (:fn :int → :int)) (x :int) -> :int)
    (f x)))
(apply-fn 42 5)
"#;
    let result = eval_str(code);
    assert!(result.is_err(), "passing non-fn to arrow contract should fail");
    let err = result.unwrap_err();
    // Error is either "contract violation" or "not callable" depending on dispatch order
    assert!(err.contains("contract violation") || err.contains("not callable"), "got: {}", err);
}

#[test]
fn test_arrow_no_return_type() {
    // (:fn :int) without arrow — just checks if it's a function
    let result = eval_str("(valid-type? (:fn :int))");
    assert!(result.is_ok(), "arrow without return should be valid");
}

#[test]
fn test_matches_arrow_type() {
    assert_eq!(
        eval_str("(matches? (lambda (x) x) (:fn :int → :int))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(matches? 42 (:fn :int → :int))").unwrap(),
        LispVal::Bool(false)
    );
}

// ══════════════════════════════════════
// Integration: pure + infer-type + arrow
// ══════════════════════════════════════

#[test]
fn test_pure_infer_check_roundtrip() {
    // Define pure function, infer its type, check it against an arrow
    let code = r#"
(pure (define (square x) (* x x)))
(define sq-type (infer-type square))
(> (length sq-type) 0)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Bool(true));
}

#[test]
fn test_higher_order_pipeline() {
    // map + pure lambda → contract checking at boundaries
    let code = r#"
(pure (define (inc x) (+ x 1)))
(define safe-apply
  (contract ((f (:fn :int → :int)) (xs :list) -> :list)
    (map f xs)))
(safe-apply inc (list 1 2 3))
"#;
    let result = eval_str(code);
    assert!(result.is_ok(), "higher-order contract pipeline should work, got: {:?}", result);
    assert_eq!(
        result.unwrap(),
        LispVal::List(vec![LispVal::Num(2), LispVal::Num(3), LispVal::Num(4)])
    );
}
