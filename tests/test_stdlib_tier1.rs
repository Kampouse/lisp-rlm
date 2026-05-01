//! Tests for Tier 1 Scheme stdlib additions.
//!
//! Numeric: abs, min, max, floor, ceiling, round, sqrt, number->string
//! Predicates: zero?, positive?, negative?, even?, odd?, procedure?, symbol?
//! Equivalence: equal?, eq?, symbol=?
//! Conversion: symbol->string, string->symbol
//! Lists: member, assoc, partition, fold-left, fold-right, for-each, cons*
//! Strings: string->list, list->string, string<?, string->number
//! Control: apply, eval
//! IO: delete-file

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

// ── Numeric ──

#[test]
fn test_abs() {
    assert_eq!(eval_str("(abs -5)").unwrap(), LispVal::Num(5));
    assert_eq!(eval_str("(abs 5)").unwrap(), LispVal::Num(5));
    assert_eq!(eval_str("(abs 0)").unwrap(), LispVal::Num(0));
    assert_eq!(eval_str("(abs -3.5)").unwrap(), LispVal::Float(3.5));
}

#[test]
fn test_min_max() {
    assert_eq!(eval_str("(min 3 1 4 1 5)").unwrap(), LispVal::Num(1));
    assert_eq!(eval_str("(max 3 1 4 1 5)").unwrap(), LispVal::Num(5));
    assert_eq!(eval_str("(min -1)").unwrap(), LispVal::Num(-1));
    assert_eq!(eval_str("(max 42)").unwrap(), LispVal::Num(42));
    assert_eq!(eval_str("(min 1.5 2.5)").unwrap(), LispVal::Float(1.5));
    assert_eq!(eval_str("(max 1.5 2.5)").unwrap(), LispVal::Float(2.5));
}

#[test]
fn test_floor_ceiling_round() {
    assert_eq!(eval_str("(floor 3.7)").unwrap(), LispVal::Num(3));
    assert_eq!(eval_str("(floor -3.7)").unwrap(), LispVal::Num(-4));
    assert_eq!(eval_str("(ceiling 3.2)").unwrap(), LispVal::Num(4));
    assert_eq!(eval_str("(ceiling -3.2)").unwrap(), LispVal::Num(-3));
    assert_eq!(eval_str("(round 3.5)").unwrap(), LispVal::Num(4));
    assert_eq!(eval_str("(round 3.4)").unwrap(), LispVal::Num(3));
    assert_eq!(eval_str("(round -3.5)").unwrap(), LispVal::Num(-4));
    // integers pass through
    assert_eq!(eval_str("(floor 5)").unwrap(), LispVal::Num(5));
    assert_eq!(eval_str("(ceiling 5)").unwrap(), LispVal::Num(5));
}

#[test]
fn test_sqrt() {
    assert_eq!(eval_str("(sqrt 9)").unwrap(), LispVal::Num(3));
    assert_eq!(
        eval_str("(sqrt 2)").unwrap(),
        LispVal::Float(std::f64::consts::SQRT_2)
    );
    assert_eq!(eval_str("(sqrt 0)").unwrap(), LispVal::Num(0));
}

#[test]
fn test_number_to_string() {
    assert_eq!(
        eval_str("(number->string 42)").unwrap(),
        LispVal::Str("42".into())
    );
    assert_eq!(
        eval_str("(number->string 3.14)").unwrap(),
        LispVal::Str("3.14".into())
    );
    assert_eq!(
        eval_str("(number->string -7)").unwrap(),
        LispVal::Str("-7".into())
    );
}

// ── Predicates ──

#[test]
fn test_zero_positive_negative() {
    assert_eq!(eval_str("(zero? 0)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(zero? 1)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(positive? 5)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(positive? -1)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(positive? 0)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(negative? -3)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(negative? 3)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(negative? 0)").unwrap(), LispVal::Bool(false));
}

#[test]
fn test_even_odd() {
    assert_eq!(eval_str("(even? 4)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(even? 3)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(odd? 3)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(odd? 4)").unwrap(), LispVal::Bool(false));
    assert_eq!(eval_str("(even? 0)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_procedure_predicate() {
    assert_eq!(
        eval_str("(procedure? (lambda (x) x))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(eval_str("(procedure? 42)").unwrap(), LispVal::Bool(false));
    assert_eq!(
        eval_str("(procedure? \"hello\")").unwrap(),
        LispVal::Bool(false)
    );
}

#[test]
fn test_symbol_predicate() {
    assert_eq!(
        eval_str("(symbol? (quote foo))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(eval_str("(symbol? 42)").unwrap(), LispVal::Bool(false));
    assert_eq!(
        eval_str("(symbol? \"hello\")").unwrap(),
        LispVal::Bool(false)
    );
}

// ── Equivalence ──

#[test]
fn test_equal() {
    // Deep structural equality
    assert_eq!(
        eval_str("(equal? (list 1 2 3) (list 1 2 3))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(equal? (list 1 2 3) (list 1 2 4))").unwrap(),
        LispVal::Bool(false)
    );
    assert_eq!(
        eval_str("(equal? \"hello\" \"hello\")").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(eval_str("(equal? 42 42)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(equal? nil nil)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(equal? true true)").unwrap(), LispVal::Bool(true));
    assert_eq!(
        eval_str("(equal? true false)").unwrap(),
        LispVal::Bool(false)
    );
}

#[test]
fn test_equal_nested() {
    // Nested structures
    assert_eq!(
        eval_str("(equal? (list 1 (list 2 3)) (list 1 (list 2 3)))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(equal? (list 1 (list 2 3)) (list 1 (list 2 4)))").unwrap(),
        LispVal::Bool(false)
    );
}

#[test]
fn test_eq_identity() {
    // eq? tests identity, not structural equality
    // Symbols with same name should be eq? (interned)
    assert_eq!(
        eval_str("(eq? (quote foo) (quote foo))").unwrap(),
        LispVal::Bool(true)
    );
    // Different types
    assert_eq!(eval_str("(eq? 1 1)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(eq? nil nil)").unwrap(), LispVal::Bool(true));
    assert_eq!(eval_str("(eq? true true)").unwrap(), LispVal::Bool(true));
}

#[test]
fn test_symbol_equality() {
    assert_eq!(
        eval_str("(symbol=? (quote foo) (quote foo))").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(symbol=? (quote foo) (quote bar))").unwrap(),
        LispVal::Bool(false)
    );
}

// ── Conversion ──

#[test]
fn test_symbol_string_conversion() {
    assert_eq!(
        eval_str("(symbol->string (quote foo))").unwrap(),
        LispVal::Str("foo".into())
    );
    assert_eq!(
        eval_str("(string->symbol \"bar\")").unwrap(),
        LispVal::Sym("bar".into())
    );
    // Round-trip
    assert_eq!(
        eval_str("(symbol->string (string->symbol \"hello\"))").unwrap(),
        LispVal::Str("hello".into())
    );
}

// ── Lists ──

#[test]
fn test_member() {
    // Returns tail from match
    let r = eval_str("(member 3 (list 1 2 3 4 5))").unwrap();
    assert_eq!(
        r,
        LispVal::List(vec![LispVal::Num(3), LispVal::Num(4), LispVal::Num(5)])
    );
    // Not found
    assert_eq!(
        eval_str("(member 99 (list 1 2 3))").unwrap(),
        LispVal::Bool(false)
    );
    // String
    assert_eq!(
        eval_str("(member \"b\" (list \"a\" \"b\" \"c\"))").unwrap(),
        LispVal::List(vec![LispVal::Str("b".into()), LispVal::Str("c".into())])
    );
}

#[test]
fn test_assoc() {
    // alist = list of pairs (lists of 2 elements)
    let code = "(define alist (list (list \"a\" 1) (list \"b\" 2) (list \"c\" 3)))";
    eval_str(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    for expr in parse_all(code).unwrap() {
        lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state).unwrap();
    }
    // Assoc finds by car
    let r = lisp_rlm_wasm::program::run_program(
        &parse_all("(assoc \"b\" alist)").unwrap(),
        &mut env,
        &mut state,
    )
    .unwrap();
    assert_eq!(
        r,
        LispVal::List(vec![LispVal::Str("b".into()), LispVal::Num(2)])
    );
    // Not found
    let r = lisp_rlm_wasm::program::run_program(
        &parse_all("(assoc \"z\" alist)").unwrap(),
        &mut env,
        &mut state,
    )
    .unwrap();
    assert_eq!(r, LispVal::Bool(false));
}

#[test]
fn test_partition() {
    let r = eval_str("(partition (lambda (x) (> x 3)) (list 1 5 2 6 3 7))").unwrap();
    // Returns (matching . non-matching)
    match r {
        LispVal::List(pair) if pair.len() == 2 => {
            assert_eq!(
                pair[0],
                LispVal::List(vec![LispVal::Num(5), LispVal::Num(6), LispVal::Num(7)])
            );
            assert_eq!(
                pair[1],
                LispVal::List(vec![LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)])
            );
        }
        _ => panic!("partition should return 2-element list, got {:?}", r),
    }
}

#[test]
fn test_fold_left() {
    // (fold-left + 0 (list 1 2 3)) = ((0+1)+2)+3 = 6
    assert_eq!(
        eval_str("(fold-left (lambda (acc x) (+ acc x)) 0 (list 1 2 3))").unwrap(),
        LispVal::Num(6)
    );
    // Build reversed list
    assert_eq!(
        eval_str("(fold-left (lambda (acc x) (cons x acc)) (list) (list 1 2 3))").unwrap(),
        LispVal::List(vec![LispVal::Num(3), LispVal::Num(2), LispVal::Num(1)])
    );
}

#[test]
fn test_fold_right() {
    // (fold-right cons (list) (list 1 2 3)) = (1 . (2 . (3 . ())))
    assert_eq!(
        eval_str("(fold-right (lambda (x acc) (cons x acc)) (list) (list 1 2 3))").unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)])
    );
}

#[test]
fn test_for_each() {
    // for-each returns nil
    let r = eval_str("(for-each (lambda (x) x) (list 1 2 3))").unwrap();
    assert_eq!(r, LispVal::Nil);

    // for-each on empty list returns nil
    let r = eval_str("(for-each (lambda (x) x) (list))").unwrap();
    assert_eq!(r, LispVal::Nil);

    // Verify it actually invokes the function: use it with a pure function
    // and check no error is raised
    let r = eval_str("(for-each print (list 1 2 3))").unwrap();
    assert_eq!(r, LispVal::Nil);
}

#[test]
fn test_cons_star() {
    assert_eq!(
        eval_str("(cons* 1 2 3)").unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)])
    );
    assert_eq!(eval_str("(cons* 1)").unwrap(), LispVal::Num(1));
    assert_eq!(
        eval_str("(cons* 1 2 (list 3 4))").unwrap(),
        LispVal::List(vec![
            LispVal::Num(1),
            LispVal::Num(2),
            LispVal::Num(3),
            LispVal::Num(4)
        ])
    );
}

// ── Strings ──

#[test]
fn test_string_list_conversion() {
    assert_eq!(
        eval_str("(string->list \"abc\")").unwrap(),
        LispVal::List(vec![
            LispVal::Str("a".into()),
            LispVal::Str("b".into()),
            LispVal::Str("c".into())
        ])
    );
    assert_eq!(
        eval_str("(list->string (list \"x\" \"y\" \"z\"))").unwrap(),
        LispVal::Str("xyz".into())
    );
}

#[test]
fn test_string_less_than() {
    assert_eq!(
        eval_str("(string<? \"abc\" \"def\")").unwrap(),
        LispVal::Bool(true)
    );
    assert_eq!(
        eval_str("(string<? \"def\" \"abc\")").unwrap(),
        LispVal::Bool(false)
    );
    assert_eq!(
        eval_str("(string<? \"abc\" \"abc\")").unwrap(),
        LispVal::Bool(false)
    );
}

#[test]
fn test_string_to_number() {
    assert_eq!(
        eval_str("(string->number \"42\")").unwrap(),
        LispVal::Num(42)
    );
    assert_eq!(
        eval_str("(string->number \"3.14\")").unwrap(),
        LispVal::Float(3.14)
    );
    assert_eq!(
        eval_str("(string->number \"not a number\")").unwrap(),
        LispVal::Bool(false)
    );
}

// ── Control ──

#[test]
fn test_apply() {
    assert_eq!(eval_str("(apply + (list 1 2 3))").unwrap(), LispVal::Num(6));
    assert_eq!(
        eval_str("(apply (lambda (x y) (+ x y)) (list 3 4))").unwrap(),
        LispVal::Num(7)
    );
    // With extra args before list
    assert_eq!(
        eval_str("(apply + 1 2 (list 3 4))").unwrap(),
        LispVal::Num(10)
    );
}

#[test]
fn test_eval() {
    // eval evaluates a datum as code
    assert_eq!(eval_str("(eval (quote (+ 1 2)))").unwrap(), LispVal::Num(3));
    assert_eq!(
        eval_str("(eval (quote (list 1 2 3)))").unwrap(),
        LispVal::List(vec![LispVal::Num(1), LispVal::Num(2), LispVal::Num(3)])
    );
    // eval with constructed code
    assert_eq!(
        eval_str("(eval (list (quote +) 10 20))").unwrap(),
        LispVal::Num(30)
    );
}
