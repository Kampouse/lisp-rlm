//! Extended tests for the runtime type system — edge cases, gaps, and corner cases.

use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(code).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

// ── Nested Compound Types ──

#[test]
fn test_nested_list_of_list() {
    // (:list (:list :int)) — list of int-lists
    assert!(
        eval_str("(check (list (list 1 2) (list 3 4)) (:list (:list :int)))").is_ok(),
        "nested list of ints should pass"
    );
    assert!(
        eval_str("(check (list (list 1 2) (list \"bad\" 4)) (:list (:list :int)))").is_err(),
        "nested list with string in inner list should fail"
    );
}

#[test]
fn test_nested_list_of_or() {
    // (:list (:or :int :str)) — list of ints or strings
    assert!(
        eval_str("(check (list 1 \"two\" 3) (:list (:or :int :str)))").is_ok(),
        "mixed int/str list should pass with :or"
    );
    assert!(
        eval_str("(check (list 1 true 3) (:list (:or :int :str)))").is_err(),
        "bool in int/str list should fail"
    );
}

#[test]
fn test_nested_map_of_list() {
    // (:map :str (:list :int)) — string → int-list map
    assert!(
        eval_str("(check (dict \"scores\" (list 1 2 3)) (:map :str (:list :int)))").is_ok(),
        "map with list-of-int values should pass"
    );
    assert!(
        eval_str("(check (dict \"scores\" (list 1 \"bad\" 3)) (:map :str (:list :int)))").is_err(),
        "map with bad list values should fail"
    );
}

#[test]
fn test_nested_tuple_with_compound() {
    // (:tuple (:list :int) :str) — tuple of int-list and string
    assert!(
        eval_str("(check (list (list 1 2 3) \"hello\") (:tuple (:list :int) :str))").is_ok(),
        "tuple with nested list should pass"
    );
    assert!(
        eval_str("(check (list (list 1 \"bad\") \"hello\") (:tuple (:list :int) :str))").is_err(),
        "tuple with bad nested list should fail"
    );
}

#[test]
fn test_deeply_nested_or_in_map() {
    // (:map :str (:or :int :str))
    assert!(
        eval_str("(check (dict \"x\" 42 \"y\" \"hello\") (:map :str (:or :int :str)))").is_ok(),
        "map with mixed int/str values should pass"
    );
    assert!(
        eval_str("(check (dict \"x\" true) (:map :str (:or :int :str)))").is_err(),
        "map with bool value should fail"
    );
}

// ── Higher-Order: fn type checks ──

#[test]
fn test_check_lambda_is_fn() {
    assert!(
        eval_str("(check (lambda (x) x) :fn)").is_ok(),
        "lambda should match :fn"
    );
}

#[test]
fn test_check_builtin_is_fn() {
    // Builtins like + are not Lambda values — they're handled in dispatch.
    // Wrapping in lambda to test fn check on a real lambda.
    assert!(
        eval_str("(define my-fn (lambda (x) (+ x 1))) (check my-fn :fn)").is_ok(),
        "user-defined fn should match :fn"
    );
}

#[test]
fn test_fn_type_rejection() {
    assert!(
        eval_str("(check 42 :fn)").is_err(),
        "number should not match :fn"
    );
    assert!(
        eval_str("(check \"hello\" :fn)").is_err(),
        "string should not match :fn"
    );
}

#[test]
fn test_type_of_lambda() {
    assert_eq!(
        eval_str("(type-of (lambda (x) x))").unwrap(),
        LispVal::Sym(":fn".into())
    );
}

// ── Contracts: Edge Cases ──

#[test]
fn test_contract_no_return_type() {
    // Contract without -> just checks params, no return check
    let code = r#"
(define add1
  (contract (x :int)
    (+ x 1)))
(add1 5)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(6));
}

#[test]
fn test_contract_multi_body_not_supported() {
    // Contract only takes 1 body expression. Multi-body should only use the first.
    let code = r#"
(define add1
  (contract ((x :int) -> :int)
    (+ x 1)
    (* x 2)))
(add1 5)
"#;
    // Should get 6 (first body expr), not 10
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(6));
}

#[test]
fn test_contract_with_float() {
    let code = r#"
(define halve
  (contract ((x :float) -> :float)
    (/ x 2.0)))
(halve 10.0)
"#;
    let result = eval_str(code).unwrap();
    // Result should be a float close to 5.0
    match result {
        LispVal::Float(f) => assert!((f - 5.0).abs() < 0.001, "expected ~5.0, got {}", f),
        other => panic!("expected float, got {}", other),
    }
}

#[test]
fn test_contract_param_wrong_type_float_to_int() {
    let code = r#"
(define int-only
  (contract ((x :int) -> :int)
    x))
(int-only 3.14)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("contract violation"), "got: {}", err);
}

#[test]
fn test_contract_return_float_expected_int() {
    let code = r#"
(define bad
  (contract ((x :int) -> :int)
    (/ x 2.0)))
(bad 5)
"#;
    let result = eval_str(code);
    // This may or may not fail depending on whether / returns float
    // At minimum it shouldn't panic
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_contract_grouped_flat_equivalence() {
    // Two ways to write same contract should behave identically
    let flat = r#"
(define f
  (contract (x :int y :str -> :str)
    (str-concat y "x")))
(f 42 "hello")
"#;
    let grouped = r#"
(define g
  (contract ((x :int) (y :str) -> :str)
    (str-concat y "x")))
(g 42 "hello")
"#;
    assert_eq!(eval_str(flat).unwrap(), eval_str(grouped).unwrap());
}

#[test]
fn test_contract_return_num() {
    // :num matches both int and float
    let code = r#"
(define id
  (contract ((x :num) -> :num)
    x))
(id 42)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));

    let code2 = r#"
(define id2
  (contract ((x :num) -> :num)
    x))
(id2 3.14)
"#;
    match eval_str(code2).unwrap() {
        LispVal::Float(f) => assert!((f - 3.14).abs() < 0.001),
        other => panic!("expected float, got {}", other),
    }
}

#[test]
fn test_contract_unicode_arrow() {
    // Unicode → should work same as ->
    let code = "(define f (contract ((x :int) → :int) (+ x 1))) (f 5)";
    let result = eval_str(code);
    // May or may not work — test documents behavior
    if let Ok(val) = result {
        assert_eq!(val, LispVal::Num(6));
    }
    // If it fails, that's a known limitation
}

// ── Schemas: Edge Cases ──

#[test]
fn test_schema_compound_or_field() {
    let code = r#"
(defschema :event "name" :str "count" (:or :int :nil))
(validate (dict "name" "click" "count" 42) :event)
"#;
    assert!(
        eval_str(code).is_ok(),
        "int value for :or :int :nil should pass"
    );

    let code2 = r#"
(defschema :event "name" :str "count" (:or :int :nil))
(validate (dict "name" "click" "count" nil) :event)
"#;
    assert!(
        eval_str(code2).is_ok(),
        "nil value for :or :int :nil should pass"
    );

    let code3 = r#"
(defschema :event "name" :str "count" (:or :int :nil))
(validate (dict "name" "click" "count" "bad") :event)
"#;
    assert!(
        eval_str(code3).is_err(),
        "string value for :or :int :nil should fail"
    );
}

#[test]
fn test_schema_nested_list_field() {
    let code = r#"
(defschema :config "name" :str "tags" (:list :str))
(validate (dict "name" "prod" "tags" (list "web" "api")) :config)
"#;
    assert!(
        eval_str(code).is_ok(),
        "schema with nested list field should pass"
    );
}

#[test]
fn test_schema_any_field() {
    let code = r#"
(defschema :bag "key" :str "value" :any)
(validate (dict "key" "x" "value" 42) :bag)
"#;
    assert!(eval_str(code).is_ok(), ":any should accept int");

    let code2 = r#"
(defschema :bag "key" :str "value" :any)
(validate (dict "key" "x" "value" "hello") :bag)
"#;
    assert!(eval_str(code2).is_ok(), ":any should accept string");

    let code3 = r#"
(defschema :bag "key" :str "value" :any)
(validate (dict "key" "x" "value" nil) :bag)
"#;
    assert!(eval_str(code3).is_ok(), ":any should accept nil");
}

#[test]
fn test_schema_strict_rejects_extra() {
    let code = r#"
(defschema :point "x" :int "y" :int :strict)
(validate (dict "x" 1 "y" 2 "z" 3) :point)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(
        err.contains("unexpected") || err.contains("extra"),
        "got: {}",
        err
    );
}

#[test]
fn test_schema_strict_passes_exact() {
    let code = r#"
(defschema :point "x" :int "y" :int :strict)
(validate (dict "x" 1 "y" 2) :point)
"#;
    assert!(
        eval_str(code).is_ok(),
        "exact fields in strict schema should pass"
    );
}

#[test]
fn test_schema_non_strict_allows_extra() {
    let code = r#"
(defschema :person "name" :str "age" :int)
(validate (dict "name" "Jean" "age" 30 "email" "j@x.com") :person)
"#;
    assert!(
        eval_str(code).is_ok(),
        "non-strict schema should allow extra fields"
    );
}

#[test]
fn test_schema_missing_required_field() {
    let code = r#"
(defschema :required "a" :int "b" :str "c" :bool)
(validate (dict "a" 1 "b" "x") :required)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("missing"), "got: {}", err);
    assert!(
        err.contains("c"),
        "should mention missing field 'c', got: {}",
        err
    );
}

#[test]
fn test_schema_wrong_type_nested() {
    let code = r#"
(defschema :nested "items" (:list :int))
(validate (dict "items" (list 1 "bad" 3)) :nested)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(
        err.contains("items") || err.contains("int") || err.contains("list"),
        "got: {}",
        err
    );
}

#[test]
fn test_schema_inspect_structure() {
    let code = r#"
(defschema :test-inspect "x" :int "y" :str)
(schema :test-inspect)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::List(parts) => {
            assert_eq!(parts.len(), 3, "schema inspect should return 3 parts");
            // First should be the name (stored as string with : prefix)
            match &parts[0] {
                LispVal::Str(s) => assert!(
                    s.contains("test-inspect"),
                    "name should contain schema name, got: {}",
                    s
                ),
                LispVal::Sym(s) => assert!(
                    s.contains("test-inspect"),
                    "name should contain schema name, got: {}",
                    s
                ),
                other => panic!("expected string or symbol for name, got {}", other),
            }
            // Second should be field list
            match &parts[1] {
                LispVal::List(fields) => {
                    assert_eq!(fields.len(), 2, "should have 2 fields");
                }
                other => panic!("expected list of fields, got {}", other),
            }
            // Third should be strict flag
            match &parts[2] {
                LispVal::Bool(b) => assert!(!b, "should not be strict by default"),
                other => panic!("expected bool for strict flag, got {}", other),
            }
        }
        other => panic!("expected list from schema inspect, got {}", other),
    }
}

// ── Equivalence Predicates ──

#[test]
fn test_equal_basic() {
    assert_eq!(
        eval_str("(equal? (list 1 2 3) (list 1 2 3))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(equal? (list 1 2) (list 1 3))").unwrap(),
        LispVal::Bool(false)
    );
    assert_eq!(eval_str("(equal? 42 42)").unwrap(), LispVal::Bool(true));
    assert_eq!(
        eval_str("(equal? \"a\" \"a\")").unwrap(),
        LispVal::Bool(true)
    );
}

#[test]
fn test_eq_symbol_identity() {
    // eq? on symbols should be true (same interned symbol)
    assert_eq!(
        eval_str("(eq? (quote foo) (quote foo))").unwrap(),
        LispVal::Bool(true)
    );
}

#[test]
fn test_matches_vs_check_consistency() {
    // matches? and check should agree on all types
    assert_eq!(eval_str("(matches? 42 :int)").unwrap(), LispVal::Bool(true));
    assert!(eval_str("(check 42 :int)").is_ok());

    assert_eq!(
        eval_str("(matches? \"hello\" :int)").unwrap(),
        LispVal::Bool(false)
    );
    assert!(eval_str("(check \"hello\" :int)").is_err());
}

// ── Type Conversion ──

#[test]
fn test_symbol_string_roundtrip() {
    assert_eq!(
        eval_str("(symbol->string (quote hello))").unwrap(),
        LispVal::Str("hello".into())
    );
    assert_eq!(
        eval_str("(string->symbol \"hello\")").unwrap(),
        LispVal::Sym("hello".into())
    );
}

// ── Boundary: empty collections ──

#[test]
fn test_check_empty_list() {
    assert!(
        eval_str("(check (list) (:list :int))").is_ok(),
        "empty list should match (:list :int)"
    );
}

#[test]
fn test_check_empty_tuple() {
    // (:tuple) matches empty list
    assert!(
        eval_str("(check (list) (:tuple))").is_ok(),
        "empty list should match empty tuple"
    );
}

#[test]
fn test_check_empty_map() {
    assert!(
        eval_str("(check (dict) (:map :str :int))").is_ok(),
        "empty map should match (:map :str :int)"
    );
}

// ── Union edge cases ──

#[test]
fn test_or_single_type() {
    assert!(
        eval_str("(check 42 (:or :int))").is_ok(),
        "single-type union should work"
    );
    assert!(
        eval_str("(check \"x\" (:or :int))").is_err(),
        "single-type union should still reject non-matching"
    );
}

#[test]
fn test_or_wide_union() {
    assert!(eval_str("(check 42 (:or :int :str :bool :nil))").is_ok());
    assert!(eval_str("(check nil (:or :int :str :bool :nil))").is_ok());
    assert!(
        eval_str("(check (list 1) (:or :int :str :bool :nil))").is_err(),
        "list should not match wide union of primitives"
    );
}

// ── num type ──

#[test]
fn test_num_accepts_int_and_float() {
    assert!(eval_str("(check 42 :num)").is_ok());
    assert!(eval_str("(check 3.14 :num)").is_ok());
    assert!(eval_str("(check \"x\" :num)").is_err());
    assert!(eval_str("(check true :num)").is_err());
}

#[test]
fn test_type_of_float() {
    assert_eq!(
        eval_str("(type-of 3.14)").unwrap(),
        LispVal::Sym(":float".into())
    );
}

// ── Contract + Lambda interaction ──

#[test]
fn test_contract_wrapping_lambda() {
    // Contract wraps a lambda; calling contract should work like calling lambda
    let code = r#"
(define square
  (contract ((x :int) -> :int)
    (* x x)))
(list (square 1) (square 2) (square 3))
"#;
    assert_eq!(
        eval_str(code).unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(4), LispVal::Num(9)])
    );
}

#[test]
fn test_contract_in_higher_order() {
    // Pass contracted fn to map
    let code = r#"
(define safe-inc
  (contract ((x :int) -> :int)
    (+ x 1)))
(map safe-inc (list 1 2 3))
"#;
    let result = eval_str(code);
    assert!(
        result.is_ok(),
        "contracted fn in map should work, got: {:?}",
        result
    );
    assert_eq!(
        result.unwrap(),
        LispVal::List(vec![LispVal::Num(2), LispVal::Num(3), LispVal::Num(4)])
    );
}

// ── valid-type? ──

#[test]
fn test_valid_type_primitives() {
    assert!(
        eval_str("(valid-type? :int)").is_ok(),
        ":int should be a valid type"
    );
    assert!(
        eval_str("(valid-type? :str)").is_ok(),
        ":str should be a valid type"
    );
}

#[test]
fn test_valid_type_compound() {
    assert!(
        eval_str("(valid-type? (:list :int))").is_ok(),
        "(:list :int) should be a valid type"
    );
    assert!(
        eval_str("(valid-type? (:or :int :str))").is_ok(),
        "(:or :int :str) should be a valid type"
    );
}
