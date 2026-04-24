use lisp_rlm::*;

fn run_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    run_program(code).unwrap_or_else(|e| format!("ERROR: {}", e))
}

fn eval_val(code: &str) -> LispVal {
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env).unwrap();
    }
    result
}

// --- Map fast path tests ---

#[test]
fn test_map_simple_lambda_arithmetic() {
    let result = eval("(map (lambda (x) (+ x 1)) (list 1 2 3))");
    assert_eq!(result, "(2 3 4)");
}

#[test]
fn test_map_simple_lambda_complex_expr() {
    let result = eval_val("(map (lambda (x) (* (+ x 1) 2)) (list 1 2 3))");
    match result {
        LispVal::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], LispVal::Num(4));
            assert_eq!(items[1], LispVal::Num(6));
            assert_eq!(items[2], LispVal::Num(8));
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

#[test]
fn test_map_empty_list() {
    let result = eval("(map (lambda (x) (+ x 1)) (list))");
    assert_eq!(result, "()");
}

#[test]
fn test_map_lambda_with_string_ops() {
    // Verify str-concat works through bytecode fast path
    let result = eval(r#"(map (lambda (x) (str-concat x "!")) (list "hi" "bye"))"#);
    // Result should contain the concatenated strings
    assert!(result.contains("hi!"), "expected 'hi!' in output, got: {}", result);
    assert!(result.contains("bye!"), "expected 'bye!' in output, got: {}", result);
}

#[test]
fn test_map_with_macro_falls_back() {
    // Macro in lambda body -> bytecode can't handle it -> falls back to eval path
    let code = r#"
        (defmacro dbl (x) (quasiquote (* (unquote x) 2)))
        (map (lambda (x) (dbl x)) (list 1 2 3))
    "#;
    let result = eval(code);
    assert_eq!(result, "(2 4 6)");
}

#[test]
fn test_map_with_user_function_falls_back() {
    // User-defined function in lambda body -> bytecode fails -> falls back
    let code = r#"
        (define triple (lambda (x) (* x 3)))
        (map (lambda (x) (triple x)) (list 1 2 3))
    "#;
    let result = eval(code);
    assert_eq!(result, "(3 6 9)");
}

// --- Filter fast path tests ---

#[test]
fn test_filter_simple_lambda() {
    let result = eval_val("(filter (lambda (x) (> x 3)) (list 1 2 3 4 5))");
    match result {
        LispVal::List(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], LispVal::Num(4));
            assert_eq!(items[1], LispVal::Num(5));
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

#[test]
fn test_filter_empty_list() {
    let result = eval("(filter (lambda (x) (> x 0)) (list))");
    assert_eq!(result, "()");
}

#[test]
fn test_filter_all_pass() {
    let result = eval("(filter (lambda (x) (> x 0)) (list 1 2 3))");
    assert_eq!(result, "(1 2 3)");
}

#[test]
fn test_filter_none_pass() {
    let result = eval("(filter (lambda (x) (> x 10)) (list 1 2 3))");
    assert_eq!(result, "()");
}

#[test]
fn test_filter_with_macro_falls_back() {
    let code = r#"
        (defmacro is-big (x) (quasiquote (> (unquote x) 5)))
        (filter (lambda (x) (is-big x)) (list 1 3 6 8 2 10))
    "#;
    let result = eval_val(code);
    match result {
        LispVal::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], LispVal::Num(6));
            assert_eq!(items[1], LispVal::Num(8));
            assert_eq!(items[2], LispVal::Num(10));
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

// --- Reduce correctness ---

#[test]
fn test_reduce_simple() {
    let result = eval("(reduce (lambda (acc x) (+ acc x)) 0 (list 1 2 3 4 5))");
    assert_eq!(result, "15");
}

// --- Large data tests (verifies fast path handles scale) ---

#[test]
fn test_map_large_list() {
    let result = eval_val("(map (lambda (x) (* x x)) (range 0 1000))");
    match result {
        LispVal::List(items) => {
            assert_eq!(items.len(), 1000);
            assert_eq!(items[0], LispVal::Num(0));
            assert_eq!(items[1], LispVal::Num(1));
            assert_eq!(items[10], LispVal::Num(100));
            assert_eq!(items[999], LispVal::Num(999 * 999));
        }
        _ => panic!("expected list, got {:?}", result),
    }
}

#[test]
fn test_filter_large_list() {
    let result = eval_val("(filter (lambda (x) (= (mod x 2) 0)) (range 0 100))");
    match result {
        LispVal::List(items) => {
            // Even numbers from 0 to 99 = 50 items
            assert_eq!(items.len(), 50);
            assert_eq!(items[0], LispVal::Num(0));
            assert_eq!(items[1], LispVal::Num(2));
            assert_eq!(items[49], LispVal::Num(98));
        }
        _ => panic!("expected list, got {:?}", result),
    }
}
