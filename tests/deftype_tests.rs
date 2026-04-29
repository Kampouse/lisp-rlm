use lisp_rlm::EvalState;
use lisp_rlm::*;

fn run_all(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    run_all(code).unwrap_or_else(|e| format!("ERROR: {}", e))
}

#[test]
fn test_deftype_basic() {
    assert_eq!(eval(r#"(deftype (Option a) (Some a) None) (Some 42)"#), "(Option::0 42)");
}

#[test]
fn test_deftype_nullary() {
    assert_eq!(eval(r#"(deftype (Option a) (Some a) None) None"#), "#<Option::1>");
}

#[test]
fn test_deftype_multi_field() {
    assert_eq!(eval(r#"(deftype Result (Ok val) (Err msg)) (Ok 42)"#), "(Result::0 42)");
}

#[test]
fn test_deftype_tag_test_positive() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(let ((x (Some 42))) (tag-test x "Option::0"))
"#), "true");
}

#[test]
fn test_deftype_tag_test_negative() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(let ((x (Some 42))) (tag-test x "Option::1"))
"#), "false");
}

#[test]
fn test_deftype_get_field() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(let ((x (Some 42))) (get-field x 0))
"#), "42");
}

#[test]
fn test_deftype_pattern_match_some() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(define (unwrap-or opt default)
  (if (tag-test opt "Option::0")
    (get-field opt 0)
    default))
(unwrap-or (Some 99) 0)
"#), "99");
}

#[test]
fn test_deftype_pattern_match_none() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(define (unwrap-or opt default)
  (if (tag-test opt "Option::0")
    (get-field opt 0)
    default))
(unwrap-or None 7)
"#), "7");
}

#[test]
fn test_deftype_nested() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(deftype Result (Ok val) (Err msg))
(Ok (Some "hello"))
"#), "(Result::0 (Option::0 \"hello\"))");
}

#[test]
fn test_deftype_in_function() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(define (wrap x) (Some x))
(wrap 42)
"#), "(Option::0 42)");
}

#[test]
fn test_deftype_equality() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(= (Some 1) (Some 1))
"#), "true");
}

#[test]
fn test_deftype_higher_order_map() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(map (fn (x) (Some (* x 2))) (list 1 2 3))
"#), "((Option::0 2) (Option::0 4) (Option::0 6))");
}

#[test]
fn test_deftype_err_variant() {
    assert_eq!(eval(r#"
(deftype Result (Ok val) (Err msg))
(Err "something went wrong")
"#), "(Result::1 \"something went wrong\")");
}

#[test]
fn test_deftype_two_fields() {
    assert_eq!(eval(r#"
(deftype Pair (MkPair fst snd))
(MkPair 10 20)
"#), "(Pair::0 10 20)");
}

#[test]
fn test_deftype_recursive_pattern() {
    assert_eq!(eval(r#"
(deftype (Option a) (Some a) None)
(define (map-opt f opt)
  (if (tag-test opt "Option::0")
    (Some (f (get-field opt 0)))
    None))
(map-opt (fn (x) (* x 10)) (Some 5))
"#), "(Option::0 50)");
}
