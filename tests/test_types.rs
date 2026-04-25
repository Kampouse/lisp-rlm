//! Tests for the runtime type system: predicates, contracts, and schemas.

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

// ── Layer 1: Predicates ──

#[test]
fn test_type_of_primitives() {
    assert_eq!(eval_str("(type-of 42)").unwrap(), LispVal::Sym(":int".into()));
    assert_eq!(eval_str("(type-of 3.14)").unwrap(), LispVal::Sym(":float".into()));
    assert_eq!(eval_str("(type-of \"hello\")").unwrap(), LispVal::Sym(":str".into()));
    assert_eq!(eval_str("(type-of true)").unwrap(), LispVal::Sym(":bool".into()));
    assert_eq!(eval_str("(type-of nil)").unwrap(), LispVal::Sym(":nil".into()));
    assert_eq!(eval_str("(type-of (quote foo))").unwrap(), LispVal::Sym(":sym".into()));
    assert_eq!(eval_str("(type-of (list 1 2 3))").unwrap(), LispVal::Sym(":list".into()));
    assert_eq!(eval_str("(type-of (dict \"a\" 1))").unwrap(), LispVal::Sym(":map".into()));
    assert_eq!(eval_str("(type-of (lambda (x) x))").unwrap(), LispVal::Sym(":fn".into()));
}

#[test]
fn test_check_pass() {
    assert_eq!(eval_str("(check 42 :int)").unwrap(), LispVal::Num(42));
    assert_eq!(eval_str("(check \"hello\" :str)").unwrap(), LispVal::Str("hello".into()));
    assert_eq!(eval_str("(check true :bool)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(check nil :nil)").unwrap(), LispVal::Nil);
}

#[test]
fn test_check_fail() {
    assert!(eval_str("(check \"hello\" :int)").is_err());
    assert!(eval_str("(check 42 :str)").is_err());
    assert!(eval_str("(check nil :bool)").is_err());
}

#[test]
fn test_check_compound() {
    // (:list :int) — list of ints
    assert_eq!(eval_str("(check (list 1 2 3) (:list :int))").unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3),
    ]));
    // Fails: list with string in int list
    assert!(eval_str("(check (list 1 \"two\" 3) (:list :int))").is_err());

    // (:map :str :int) — string→int map
    assert!(eval_str("(check (dict \"age\" 30) (:map :str :int))").is_ok());

    // (:tuple :int :str) — fixed-length typed list
    assert!(eval_str("(check (list 42 \"hello\") (:tuple :int :str))").is_ok());
    assert!(eval_str("(check (list 42) (:tuple :int :str))").is_err()); // wrong length
}

#[test]
fn test_check_union() {
    // (:or :int :nil) — nullable int
    assert!(eval_str("(check 42 (:or :int :nil))").is_ok());
    assert!(eval_str("(check nil (:or :int :nil))").is_ok());
    assert!(eval_str("(check \"hello\" (:or :int :nil))").is_err());

    // (:or :int :float) = :num
    assert!(eval_str("(check 3.14 (:or :int :float))").is_ok());
}

#[test]
fn test_check_any() {
    assert!(eval_str("(check 42 :any)").is_ok());
    assert!(eval_str("(check \"hello\" :any)").is_ok());
    assert!(eval_str("(check nil :any)").is_ok());
}

#[test]
fn test_check_num() {
    // :num matches both int and float
    assert!(eval_str("(check 42 :num)").is_ok());
    assert!(eval_str("(check 3.14 :num)").is_ok());
    assert!(eval_str("(check \"hello\" :num)").is_err());
}

#[test]
fn test_matches_pred() {
    assert_eq!(eval_str("(matches? 42 :int)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(matches? \"hello\" :int)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(matches? (list 1 2) (:list :int))").unwrap(), LispVal::Bool(true));
}

// ── Layer 2: Contracts ──

#[test]
fn test_contract_basic() {
    let code = r#"
(define add1
  (contract ((x :int) -> :int)
    (+ x 1)))
(add1 5)
"#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(6));
}

#[test]
fn test_contract_param_violation() {
    let code = r#"
(define add1
  (contract ((x :int) -> :int)
    (+ x 1)))
(add1 "hello")
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("contract violation"), "got: {}", err);
    assert!(err.contains("param"), "got: {}", err);
}

#[test]
fn test_contract_return_violation() {
    let code = r#"
(define bad
  (contract ((x :int) -> :str)
    (+ x 1)))
(bad 5)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("contract violation"), "got: {}", err);
    assert!(err.contains("return"), "got: {}", err);
}

#[test]
fn test_contract_multi_param() {
    let code = r#"
(define greet
  (contract (name :str age :int -> :str)
    (str-concat "Hello " name)))
(greet "Jean" 30)
"#;
    let result = eval_str(code);
    assert!(result.is_ok(), "got: {:?}", result.err());
}

// ── Layer 3: Schemas ──

#[test]
fn test_schema_validate_pass() {
    let code = r#"
(defschema :user "name" :str "age" :int)
(validate (dict "name" "Jean" "age" 30) :user)
"#;
    let result = eval_str(code).unwrap();
    match result {
        LispVal::Map(m) => {
            assert_eq!(m.get("name"), Some(&LispVal::Str("Jean".into())));
            assert_eq!(m.get("age"), Some(&LispVal::Num(30)));
        }
        other => panic!("expected map, got {}", other),
    }
}

#[test]
fn test_schema_validate_wrong_type() {
    let code = r#"
(defschema :user2 "name" :str "age" :int)
(validate (dict "name" "Jean" "age" "thirty") :user2)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("age"), "got: {}", err);
    assert!(err.contains(":int"), "got: {}", err);
}

#[test]
fn test_schema_validate_missing_field() {
    let code = r#"
(defschema :user3 "name" :str "age" :int)
(validate (dict "name" "Jean") :user3)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("missing"), "got: {}", err);
    assert!(err.contains("age"), "got: {}", err);
}

#[test]
fn test_schema_strict() {
    let code = r#"
(defschema :strict-user "name" :str "age" :int :strict)
(validate (dict "name" "Jean" "age" 30 "extra" "bad") :strict-user)
"#;
    let err = eval_str(code).unwrap_err();
    assert!(err.contains("unexpected"), "got: {}", err);
    assert!(err.contains("extra"), "got: {}", err);
}

#[test]
fn test_schema_compound_types() {
    let code = r#"
(defschema :profile "name" :str "tags" (:list :str))
(validate (dict "name" "Jean" "tags" (list "dev" "lisp")) :profile)
"#;
    let result = eval_str(code);
    assert!(result.is_ok(), "got: {:?}", result);
}

#[test]
fn test_schema_inspect() {
    let code = r#"
(defschema :test-schema "x" :int "y" :str)
(schema :test-schema)
"#;
    let result = eval_str(code).unwrap();
    // Should return (name ((x :int) (y :str)) false)
    match result {
        LispVal::List(parts) => {
            assert_eq!(parts.len(), 3);
        }
        other => panic!("expected list, got {}", other),
    }
}
