/// Comprehensive syntax coverage test for the lisp-rlm compiler frontend.
///
/// Every special form, builtin, and syntactic construct gets tested here.
/// This file is the single source of truth for "what works in the bytecode compiler."
///
/// Categories:
/// 1. Literals & atoms
/// 2. Arithmetic
/// 3. Comparison
/// 4. Logic (and, or, not)
/// 5. Control flow (if, cond, when, unless)
/// 6. Bindings (define, let, let*, set!)
/// 7. Functions (lambda, variadic, closures)
/// 8. Loop/recur (basic, multi-binding, named-let)
/// 9. Quote/quasiquote
/// 10. Begin/progn/do
/// 11. Macros (defmacro, macroexpand)
/// 12. Match
/// 13. Try/catch
/// 14. Collections (list, dict, map, filter, reduce)
/// 15. String operations
/// 16. Type system (check, matches?, contract, defschema)
/// 17. File I/O
/// 18. Higher-order (compose, apply)
/// 19. Deftype / sum types
/// 20. Stdlib modules (require)

use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(code)?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

fn eval_str_err(code: &str) -> String {
    match eval_str(code) {
        Ok(v) => format!("{:?}", v),
        Err(e) => format!("ERROR: {}", e),
    }
}

// ============================================================
// 1. LITERALS & ATOMS
// ============================================================

#[test]
fn test_literal_int() {
    assert_eq!(eval_str("42").unwrap(), LispVal::Num(42));
}

#[test]
fn test_literal_neg_int() {
    assert_eq!(eval_str("-7").unwrap(), LispVal::Num(-7));
}

#[test]
fn test_literal_float() {
    assert!(matches!(eval_str("3.14"), Ok(LispVal::Float(f)) if (f - 3.14).abs() < 0.001));
}

#[test]
fn test_literal_string() {
    assert_eq!(eval_str(r#""hello""#).unwrap(), LispVal::Str("hello".into()));
}

#[test]
fn test_literal_bool_true() {
    assert_eq!(eval_str("true").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_literal_bool_false() {
    assert_eq!(eval_str("false").unwrap(), LispVal::Bool(false));
}

#[test]
fn test_literal_nil() {
    assert_eq!(eval_str("nil").unwrap(), LispVal::Nil);
}

#[test]
fn test_empty_list() {
    assert_eq!(eval_str("()").unwrap(), LispVal::Nil);
}

#[test]
fn test_quote_symbol() {
    assert_eq!(eval_str("'foo").unwrap(), LispVal::Sym("foo".into()));
}

#[test]
fn test_quote_list() {
    assert_eq!(eval_str("'(1 2 3)").unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)
    ]));
}

// ============================================================
// 2. ARITHMETIC
// ============================================================

#[test]
fn test_add() {
    assert_eq!(eval_str("(+ 1 2 3)").unwrap(), LispVal::Num(6));
}

#[test]
fn test_sub() {
    assert_eq!(eval_str("(- 10 3 2)").unwrap(), LispVal::Num(5));
}

#[test]
fn test_mul() {
    assert_eq!(eval_str("(* 6 7)").unwrap(), LispVal::Num(42));
}

#[test]
fn test_div() {
    assert_eq!(eval_str("(/ 10 3)").unwrap(), LispVal::Num(3)); // integer div
}

#[test]
fn test_mod() {
    assert_eq!(eval_str("(% 10 3)").unwrap(), LispVal::Num(1));
}

#[test]
fn test_float_arithmetic() {
    let r = eval_str("(/ 10.0 3)").unwrap();
    assert!(matches!(r, LispVal::Float(f) if (f - 3.333).abs() < 0.01));
}

#[test]
fn test_nested_arithmetic() {
    assert_eq!(eval_str("(+ (* 2 3) (- 10 4))").unwrap(), LispVal::Num(12));
}

// ============================================================
// 3. COMPARISON
// ============================================================

#[test]
fn test_eq() {
    assert_eq!(eval_str("(= 42 42)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_lt() {
    assert_eq!(eval_str("(< 1 2)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_gt() {
    assert_eq!(eval_str("(> 2 1)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_le() {
    assert_eq!(eval_str("(<= 2 2)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_ge() {
    assert_eq!(eval_str("(>= 3 2)").unwrap(), LispVal::Bool(true));
}

// ============================================================
// 4. LOGIC
// ============================================================

#[test]
fn test_and_true() {
    assert_eq!(eval_str("(and 1 2 3)").unwrap(), LispVal::Num(3));
}

#[test]
fn test_and_false() {
    assert_eq!(eval_str("(and 1 false 3)").unwrap(), LispVal::Bool(false));
}

#[test]
fn test_or_true() {
    assert_eq!(eval_str("(or false nil 42)").unwrap(), LispVal::Num(42));
}

#[test]
fn test_or_false() {
    assert_eq!(eval_str("(or false nil false)").unwrap(), LispVal::Bool(false));
}

#[test]
fn test_not() {
    assert_eq!(eval_str("(not false)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(not 42)").unwrap(), LispVal::Bool(false));
}

// ============================================================
// 5. CONTROL FLOW
// ============================================================

#[test]
fn test_if_true() {
    assert_eq!(eval_str("(if true 1 2)").unwrap(), LispVal::Num(1));
}

#[test]
fn test_if_false() {
    assert_eq!(eval_str("(if false 1 2)").unwrap(), LispVal::Num(2));
}

#[test]
fn test_if_no_else() {
    assert_eq!(eval_str("(if false 1)").unwrap(), LispVal::Nil);
}

#[test]
fn test_cond() {
    let code = "(cond ((< 1 0) \"a\") ((> 2 1) \"b\") (else \"c\"))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Str("b".into()));
}

#[test]
fn test_cond_else() {
    let code = "(cond ((< 5 0) \"a\") ((> 0 5) \"b\") (else \"c\"))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Str("c".into()));
}

#[test]
fn test_when() {
    let code = "(when true 42)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

#[test]
fn test_when_false() {
    let code = "(when false 42)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Nil);
}

#[test]
fn test_unless() {
    let code = "(unless false 99)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(99));
}

#[test]
fn test_unless_true() {
    let code = "(unless true 99)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Nil);
}

// ============================================================
// 6. BINDINGS
// ============================================================

#[test]
fn test_define() {
    let code = "(begin (define x 42) x)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

#[test]
fn test_define_function_shorthand() {
    let code = "(begin (define (add a b) (+ a b)) (add 3 4))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(7));
}

#[test]
fn test_let() {
    let code = "(let ((x 10) (y 20)) (+ x y))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(30));
}

#[test]
fn test_let_star() {
    let code = "(let* ((x 10) (y (+ x 5))) y)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

#[test]
fn test_set_bang() {
    let code = "(begin (define x 1) (set! x 42) x)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

// ============================================================
// 7. FUNCTIONS (lambda, closures)
// ============================================================

#[test]
fn test_lambda_basic() {
    let code = "((lambda (x) (+ x 1)) 5)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(6));
}

#[test]
fn test_lambda_multi_body() {
    let code = "((lambda (x) (+ x 1) (+ x 2)) 5)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(7));
}

#[test]
fn test_lambda_variadic() {
    let code = "((lambda (a &rest rest) (len rest)) 1 2 3 4)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(3));
}

#[test]
fn test_closure() {
    let code = "(begin (define (make-adder n) (lambda (x) (+ n x))) ((make-adder 10) 5))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(15));
}

#[test]
fn test_fn_shorthand() {
    let code = "((fn (x) (* x x)) 5)";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(25));
}

// ============================================================
// 8. LOOP/RECUR
// ============================================================

#[test]
fn test_loop_basic() {
    let code = "(loop ((i 0)) (if (>= i 5) i (recur (+ i 1))))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(5));
}

#[test]
fn test_loop_sum() {
    let code = "(loop ((i 0) (sum 0)) (if (> i 10) sum (recur (+ i 1) (+ sum i))))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(55));
}

#[test]
fn test_named_let() {
    let code = "(let countdown ((n 5)) (if (= n 0) 0 (countdown (- n 1))))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(0));
}

// ============================================================
// 9. QUOTE / QUASIQUOTE
// ============================================================

#[test]
fn test_quote() {
    assert_eq!(eval_str("(quote foo)").unwrap(), LispVal::Sym("foo".into()));
}

#[test]
fn test_quote_shorthand() {
    assert_eq!(eval_str("'bar").unwrap(), LispVal::Sym("bar".into()));
}

#[test]
fn test_quasiquote_unquote() {
    let code = "(let ((x 42)) (quasiquote (+ (unquote x) 1)))";
    // Should produce the list (+ 42 1), not evaluate it
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Sym("+".into()),
        LispVal::Num(42),
        LispVal::Num(1),
    ]));
}

#[test]
fn test_quasiquote_splicing() {
    let code = "(let ((xs (list 1 2 3))) (quasiquote (0 (unquote-splicing xs) 4)))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Num(0),
        LispVal::Num(1),
        LispVal::Num(2),
        LispVal::Num(3),
        LispVal::Num(4),
    ]));
}

// ============================================================
// 10. BEGIN / PROGN / DO
// ============================================================

#[test]
fn test_begin() {
    assert_eq!(eval_str("(begin 1 2 3)").unwrap(), LispVal::Num(3));
}

#[test]
fn test_progn() {
    assert_eq!(eval_str("(progn 1 2 3)").unwrap(), LispVal::Num(3));
}

#[test]
fn test_do() {
    assert_eq!(eval_str("(do 1 2 3)").unwrap(), LispVal::Num(3));
}

// ============================================================
// 11. MACROS
// ============================================================

#[test]
fn test_defmacro_basic() {
    let code = "(begin (defmacro swap (a b) (list (quote list) b a)) (swap 1 2))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Num(2), LispVal::Num(1)
    ]));
}

#[test]
fn test_macroexpand() {
    let code = "(begin (defmacro double (x) (list (quote +) x x)) (macroexpand (double 5)))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Sym("+".into()), LispVal::Num(5), LispVal::Num(5)
    ]));
}

#[test]
fn test_macro_rest_params() {
    let code = r#"
        (begin
            (defmacro my-list (&rest items)
                (cons (quote list) items))
            (my-list 1 2 3))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)
    ]));
}

// ============================================================
// 12. MATCH (currently NOT in bytecode compiler)
// ============================================================

#[test]
fn test_match_wildcard() {
    // This should fail if match isn't in the compiler
    let r = eval_str("(match 42 (_ \"matched\"))");
    match r {
        Ok(LispVal::Str(s)) if s == "matched" => {},
        Ok(v) => panic!("Expected \"matched\", got {:?}", v),
        Err(e) => panic!("match not compiled: {}", e),
    }
}

#[test]
fn test_match_literal() {
    let r = eval_str("(match 42 (1 \"one\") (42 \"found\") (_ \"other\"))");
    match r {
        Ok(LispVal::Str(s)) if s == "found" => {},
        Ok(v) => panic!("Expected \"found\", got {:?}", v),
        Err(e) => panic!("match not compiled: {}", e),
    }
}

// ============================================================
// 13. TRY/CATCH (currently NOT in bytecode compiler)
// ============================================================

#[test]
fn test_try_catch() {
    let r = eval_str("(try (/ 1 0) (catch e (str-concat \"caught: \" e)))");
    match r {
        Ok(LispVal::Str(s)) if s.starts_with("caught:") => {},
        Ok(v) => panic!("Expected caught string, got {:?}", v),
        Err(e) => panic!("try/catch not compiled: {}", e),
    }
}

#[test]
fn test_try_success() {
    let r = eval_str("(try (+ 1 2) (catch e 0))");
    match r {
        Ok(LispVal::Num(3)) => {},
        Ok(v) => panic!("Expected 3, got {:?}", v),
        Err(e) => panic!("try not compiled: {}", e),
    }
}

// ============================================================
// 14. COLLECTIONS
// ============================================================

#[test]
fn test_list() {
    assert_eq!(eval_str("(list 1 2 3)").unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)
    ]));
}

#[test]
fn test_car() {
    assert_eq!(eval_str("(car (list 1 2 3))").unwrap(), LispVal::Num(1));
}

#[test]
fn test_cdr() {
    assert_eq!(eval_str("(cdr (list 1 2 3))").unwrap(), LispVal::List(vec![
        LispVal::Num(2), LispVal::Num(3)
    ]));
}

#[test]
fn test_cons() {
    assert_eq!(eval_str("(cons 0 (list 1 2))").unwrap(), LispVal::List(vec![
        LispVal::Num(0), LispVal::Num(1), LispVal::Num(2)
    ]));
}

#[test]
fn test_len() {
    assert_eq!(eval_str("(len (list 1 2 3))").unwrap(), LispVal::Num(3));
}

#[test]
fn test_append() {
    assert_eq!(eval_str("(append (list 1 2) (list 3 4))").unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3), LispVal::Num(4)
    ]));
}

#[test]
fn test_nth() {
    assert_eq!(eval_str("(nth (list 10 20 30) 1)").unwrap(), LispVal::Num(20));
}

#[test]
fn test_map() {
    assert_eq!(eval_str("(map (lambda (x) (* x 2)) (list 1 2 3))").unwrap(), LispVal::List(vec![
        LispVal::Num(2), LispVal::Num(4), LispVal::Num(6)
    ]));
}

#[test]
fn test_filter() {
    assert_eq!(eval_str("(filter (lambda (x) (> x 2)) (list 1 2 3))").unwrap(), LispVal::List(vec![
        LispVal::Num(3)
    ]));
}

#[test]
fn test_reduce() {
    assert_eq!(eval_str("(reduce + 0 (list 1 2 3))").unwrap(), LispVal::Num(6));
}

#[test]
fn test_reverse() {
    assert_eq!(eval_str("(reverse (list 1 2 3))").unwrap(), LispVal::List(vec![
        LispVal::Num(3), LispVal::Num(2), LispVal::Num(1)
    ]));
}

#[test]
fn test_sort() {
    assert_eq!(eval_str("(sort (list 3 1 2))").unwrap(), LispVal::List(vec![
        LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)
    ]));
}

#[test]
fn test_range() {
    assert_eq!(eval_str("(range 0 5)").unwrap(), LispVal::List(vec![
        LispVal::Num(0), LispVal::Num(1), LispVal::Num(2), LispVal::Num(3), LispVal::Num(4)
    ]));
}

#[test]
fn test_dict() {
    let code = "(begin (define d (dict \"a\" 1 \"b\" 2)) (dict/get d \"a\"))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(1));
}

// ============================================================
// 15. STRING OPERATIONS
// ============================================================

#[test]
fn test_str_concat() {
    assert_eq!(eval_str(r#"(str-concat "hello" " " "world")"#).unwrap(), LispVal::Str("hello world".into()));
}

#[test]
fn test_str_contains() {
    assert_eq!(eval_str(r#"(str-contains "hello" "ell")"#).unwrap(), LispVal::Bool(true));
}

#[test]
fn test_str_length() {
    assert_eq!(eval_str(r#"(str-length "hello")"#).unwrap(), LispVal::Num(5));
}

#[test]
fn test_str_split() {
    let r = eval_str(r#"(str-split "a,b,c" ",")"#).unwrap();
    assert_eq!(r, LispVal::List(vec![
        LispVal::Str("a".into()), LispVal::Str("b".into()), LispVal::Str("c".into())
    ]));
}

#[test]
fn test_to_string() {
    assert_eq!(eval_str("(to-string 42)").unwrap(), LispVal::Str("42".into()));
}

// ============================================================
// 16. TYPE SYSTEM
// ============================================================

#[test]
fn test_type_of() {
    assert_eq!(eval_str("(type? 42)").unwrap(), LispVal::Str("number".into()));
    assert_eq!(eval_str("(type? \"hi\")").unwrap(), LispVal::Str("string".into()));
}

#[test]
fn test_check_int() {
    assert!(eval_str("(check 42 :int)").is_ok());
}

#[test]
fn test_check_fail() {
    assert!(eval_str("(check \"hi\" :int)").is_err());
}

#[test]
fn test_matches() {
    assert_eq!(eval_str("(matches? 42 :int)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(matches? \"hi\" :int)").unwrap(), LispVal::Bool(false));
}

#[test]
fn test_contract() {
    let code = "(begin (define add1 (contract ((x :int) -> :int) (+ x 1))) (add1 5))";
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(6));
}

#[test]
fn test_contract_violation() {
    let code = "(begin (define add1 (contract ((x :int) -> :int) (+ x 1))) (add1 \"hi\"))";
    assert!(eval_str(code).is_err());
}

// ============================================================
// 17. FILE I/O
// ============================================================

#[test]
fn test_file_roundtrip() {
    let path = "/tmp/lisp_syntax_test_io.txt";
    let _ = std::fs::remove_file(path);
    eval_str(&format!(r#"(write-file "{}" "test")"#, path)).unwrap();
    let r = eval_str(&format!(r#"(read-file "{}")"#, path)).unwrap();
    assert_eq!(r, LispVal::Str("test".into()));
    let _ = std::fs::remove_file(path);
}

// ============================================================
// 18. HIGHER-ORDER
// ============================================================

#[test]
fn test_apply() {
    assert_eq!(eval_str("(apply + (list 1 2 3))").unwrap(), LispVal::Num(6));
}

#[test]
fn test_builtin_as_value() {
    let code = "(map (lambda (f) (f 5)) (list (lambda (x) (+ x 1)) (lambda (x) (* x 2))))";
    assert_eq!(eval_str(code).unwrap(), LispVal::List(vec![
        LispVal::Num(6), LispVal::Num(10)
    ]));
}

// ============================================================
// 19. DEFTYPE / SUM TYPES
// ============================================================

#[test]
fn test_deftype_nullary() {
    let code = r#"
        (begin
            (deftype Color Red Green Blue)
            (Red))
    "#;
    let r = eval_str(code).unwrap();
    // Should be a tagged value
    match r {
        LispVal::Tagged { type_name, variant_id, fields } => {
            assert_eq!(type_name, "Color");
            assert_eq!(variant_id, 0);
            assert!(fields.is_empty());
        }
        other => panic!("Expected Tagged, got {:?}", other),
    }
}

#[test]
fn test_deftype_with_fields() {
    let code = r#"
        (begin
            (deftype Shape (Circle 1) (Rect 2))
            (Circle 5.0))
    "#;
    let r = eval_str(code).unwrap();
    match r {
        LispVal::Tagged { type_name, variant_id, fields } => {
            assert_eq!(type_name, "Shape");
            assert_eq!(variant_id, 0);
            assert_eq!(fields.len(), 1);
        }
        other => panic!("Expected Tagged, got {:?}", other),
    }
}

// ============================================================
// 20. REQUIRE / STDLIB
// ============================================================

#[test]
fn test_require_math() {
    let code = r#"
        (begin
            (require "math")
            (abs -5))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(5));
}

#[test]
fn test_require_list() {
    let code = r#"
        (begin
            (require "list")
            (identity 42))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(42));
}

// ============================================================
// CROSS-CUTTING: complex programs
// ============================================================

#[test]
fn test_fibonacci() {
    let code = r#"
        (begin
            (define (fib n)
                (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2)))))
            (fib 10))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(55));
}

#[test]
fn test_fizzbuzz() {
    let code = r#"
        (loop ((i 1))
            (if (> i 15)
                nil
                (begin
                    (cond
                        (= (% i 15) 0) "FizzBuzz"
                        (= (% i 3) 0) "Fizz"
                        (= (% i 5) 0) "Buzz"
                        (else i))
                    (recur (+ i 1)))))
    "#;
    // Just check it doesn't crash
    let _ = eval_str(code);
}

#[test]
fn test_closures_chain() {
    let code = r#"
        (begin
            (define (compose f g) (lambda (x) (f (g x))))
            (define add1 (lambda (x) (+ x 1)))
            (define double (lambda (x) (* x 2)))
            ((compose add1 double) 5))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Num(11));
}

#[test]
fn test_mutual_recursion_via_set() {
    let code = r#"
        (begin
            (define even? nil)
            (define odd? nil)
            (set! even? (lambda (n) (if (= n 0) true (odd? (- n 1)))))
            (set! odd? (lambda (n) (if (= n 0) false (even? (- n 1)))))
            (even? 10))
    "#;
    assert_eq!(eval_str(code).unwrap(), LispVal::Bool(true));
}
