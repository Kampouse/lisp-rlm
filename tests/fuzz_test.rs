use lisp_rlm::*;

fn run_program(code: &str, env: &mut Env) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(&expr, env)?;
    }
    Ok(result.to_string())
}
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Generate deterministic test strings for fuzz-like coverage
fn fuzz_strings() -> Vec<String> {
    let mut cases = Vec::new();

    // Empty and whitespace
    cases.push("".to_string());
    cases.push("   ".to_string());
    cases.push("\n\t\r".to_string());

    // Random-looking symbol sequences
    for a in &["x", "foo", "-", "--", "++", "///", "nil", "true", "false"] {
        cases.push(a.to_string());
    }

    // Numbers
    for n in &["0", "1", "-1", "999999", "-999999", "3.14", "-0.5", "0.0"] {
        cases.push(n.to_string());
    }

    // Strings
    for s in &[
        r#""""#,
        r#""hello""#,
        r#""hello world""#,
        r#""with \"quotes\"""#,
        r#""with\nnewlines""#,
    ] {
        cases.push(s.to_string());
    }

    // S-expressions of various depths
    cases.push("()".to_string());
    cases.push("(+)".to_string());
    cases.push("(+ 1)".to_string());
    cases.push("(+ 1 2)".to_string());
    cases.push("(+ 1 2 3 4 5)".to_string());
    cases.push("(* (+ 1 2) (- 5 3))".to_string());
    cases.push("((((1))))".to_string());
    cases.push("(lambda (x) x)".to_string());
    cases.push("(define f (lambda (x) (* x x)))".to_string());

    // Deeply nested
    let deep = "(+ ".repeat(50) + "1" + &")".repeat(50);
    cases.push(deep);

    // Malformed
    cases.push("(".to_string());
    cases.push(")".to_string());
    cases.push("((".to_string());
    cases.push("))".to_string());
    cases.push("(+ ".to_string());
    cases.push("(+ 1".to_string());
    cases.push("\"unterminated".to_string());
    cases.push("(lambda)".to_string());
    cases.push("(if)".to_string());
    cases.push("(define)".to_string());

    // Comments
    cases.push(";; comment only".to_string());
    cases.push("(+ 1 ;; inline comment\n2)".to_string());
    cases.push("(; block comment ;) 42".to_string());
    cases.push("(+ 1 (; comment ;) 2)".to_string());

    // Mixed types
    cases.push("(list 1 \"two\" true nil 3.14)".to_string());
    cases.push("(dict \"a\" 1 \"b\" 2)".to_string());
    cases.push("(if true 1 2)".to_string());
    cases.push("(cond ((= 1 2) \"no\") (else \"yes\"))".to_string());
    cases.push("(let ((x 1) (y 2)) (+ x y))".to_string());
    cases.push("(progn 1 2 3)".to_string());
    cases.push("(and true false true)".to_string());
    cases.push("(or false true false)".to_string());
    cases.push("(not true)".to_string());

    cases
}

#[test]
fn test_fuzz_tokenizer_no_panics() {
    for input in fuzz_strings() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = parse_all(&input);
        }));
        assert!(result.is_ok(), "Parser panicked on: {:?}", input);
    }
}

#[test]
fn test_fuzz_parser_no_panics() {
    for input in fuzz_strings() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = parse_all(&input);
        }));
        assert!(result.is_ok(), "Parser panicked on: {:?}", input);
    }
}

#[test]
fn test_fuzz_evaluator_no_panics() {
    let eval_cases = vec![
        "(+ 1 2)",
        "(- 10 3)",
        "(* 4 5)",
        "(/ 10 2)",
        "(mod 7 3)",
        "(= 1 1)",
        "(< 1 2)",
        "(> 2 1)",
        "(and true true)",
        "(or false true)",
        "(not false)",
        "(if true 1 2)",
        "(list 1 2 3)",
        "(car (list 1 2 3))",
        "(cdr (list 1 2 3))",
        "(cons 0 (list 1 2))",
        "(len (list 1 2 3))",
        "(append (list 1) (list 2))",
        "(str-concat \"hello\" \" \" \"world\")",
        "(str-length \"hello\")",
        "(str-contains \"hello\" \"ell\")",
        "(str-upcase \"hello\")",
        "(str-downcase \"HELLO\")",
        "(str-trim \"  hi  \")",
        "(str-split \"a,b,c\" \",\")",
        "(str-substring \"hello\" 1 3)",
        "(str-index-of \"hello\" \"ll\")",
        "(str-starts-with \"hello\" \"hel\")",
        "(str-ends-with \"hello\" \"llo\")",
        "(nil? nil)",
        "(list? (list 1))",
        "(number? 42)",
        "(string? \"hi\")",
        "(bool? true)",
        "(to-string 42)",
        "(to-json (list 1 2 3))",
        "(from-json \"{\\\"a\\\":1}\")",
        "(dict \"x\" 1 \"y\" 2)",
        "(dict/get (dict \"x\" 1) \"x\")",
        "(dict/has? (dict \"x\" 1) \"x\")",
        "(dict/keys (dict \"a\" 1 \"b\" 2))",
        "(dict/set (dict) \"k\" 42)",
        "(dict/remove (dict \"x\" 1 \"y\" 2) \"x\")",
        "(dict/merge (dict \"a\" 1) (dict \"b\" 2))",
        "(+ 1.5 2.5)",
        "(* 3 1.5)",
        "(to-float 42)",
        "(to-int 3.7)",
        "(< 1.5 2)",
        "(> 2.5 2)",
        "(loop for i in (list 1 2 3) sum i)",
        "(require \"math\")",
        "(require \"list\")",
    ];

    for code in eval_cases {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut env = Env::new();
            let _ = run_program(code, &mut env);
        }));
        assert!(result.is_ok(), "Evaluator panicked on: {:?}", code);
    }
}

#[test]
fn test_arithmetic_commutativity() {
    let mut env = Env::new();
    let pairs: Vec<(i64, i64)> = vec![
        (0, 0),
        (1, 1),
        (5, 3),
        (-1, 100),
        (-50, -7),
        (-13, 999),
        (42, 0),
    ];

    for (a, b) in pairs {
        let code_ab = format!("(+ {} {})", a, b);
        let code_ba = format!("(+ {} {})", b, a);
        let r_ab = run_program(&code_ab, &mut env).unwrap();
        let r_ba = run_program(&code_ba, &mut env).unwrap();
        assert_eq!(r_ab, r_ba, "+ not commutative for ({}, {})", a, b);

        let code_ab = format!("(* {} {})", a, b);
        let code_ba = format!("(* {} {})", b, a);
        let r_ab = run_program(&code_ab, &mut env).unwrap();
        let r_ba = run_program(&code_ba, &mut env).unwrap();
        assert_eq!(r_ab, r_ba, "* not commutative for ({}, {})", a, b);
    }
}

#[test]
fn test_arithmetic_identity() {
    let mut env = Env::new();
    let values: Vec<i64> = vec![0, 1, 42, -1, 100, -999, 999999];

    for v in values {
        // a + 0 = a
        let code = format!("(+ {} 0)", v);
        let result = run_program(&code, &mut env).unwrap();
        assert_eq!(result, v.to_string(), "+ identity failed for {}", v);

        // a * 1 = a
        let code = format!("(* {} 1)", v);
        let result = run_program(&code, &mut env).unwrap();
        assert_eq!(result, v.to_string(), "* identity failed for {}", v);
    }
}

#[test]
fn test_arithmetic_associativity() {
    let mut env = Env::new();
    let triples: Vec<(i64, i64, i64)> = vec![(1, 2, 3), (0, 0, 0), (5, -3, 7), (10, 20, -5)];

    for (a, b, c) in triples {
        let left = format!("(+ (+ {} {}) {})", a, b, c);
        let right = format!("(+ {} (+ {} {}))", a, b, c);
        let r_left = run_program(&left, &mut env).unwrap();
        let r_right = run_program(&right, &mut env).unwrap();
        assert_eq!(
            r_left, r_right,
            "+ not associative for ({}, {}, {})",
            a, b, c
        );
    }
}

#[test]
fn test_boolean_invariants() {
    let mut env = Env::new();

    // not(not(x)) = x
    for val in &["true", "false"] {
        let code = format!("(not (not {}))", val);
        let result = run_program(&code, &mut env).unwrap();
        assert_eq!(result, *val, "double negation failed for {}", val);
    }

    // and(x, y) = and(y, x)
    let ab = run_program("(and true false)", &mut env).unwrap();
    let ba = run_program("(and false true)", &mut env).unwrap();
    assert_eq!(ab, ba, "and not commutative");

    // or(x, y) = or(y, x)
    let ab = run_program("(or true false)", &mut env).unwrap();
    let ba = run_program("(or false true)", &mut env).unwrap();
    assert_eq!(ab, ba, "or not commutative");
}
