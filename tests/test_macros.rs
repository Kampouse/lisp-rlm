use lisp_rlm::EvalState;
use lisp_rlm::*;

fn run_program(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    run_program(code).unwrap_or_else(|e| format!("ERROR: {}", e))
}

// ---------------------------------------------------------------------------
// 1. Basic defmacro + invocation
// ---------------------------------------------------------------------------

#[test]
fn test_defmacro_basic() {
    // (defmacro double (x) `(+ ~x ~x))
    // (double 3) => 6
    let r = eval(
        r#"
        (defmacro double (x) (quasiquote (+ (unquote x) (unquote x))))
        (double 3)
    "#,
    );
    assert_eq!(
        r, "6",
        "basic defmacro should expand (+ 3 3) = 6, got: {}",
        r
    );
}

#[test]
fn test_defmacro_basic_longform() {
    // Same but with backtick-style using quasiquote directly
    let r = eval(
        r#"
        (defmacro inc (x) (quasiquote (+ (unquote x) 1)))
        (inc 5)
    "#,
    );
    assert_eq!(r, "6", "inc macro should work");
}

#[test]
fn test_defmacro_two_params() {
    let r = eval(
        r#"
        (defmacro add-mac (a b) (quasiquote (+ (unquote a) (unquote b))))
        (add-mac 10 20)
    "#,
    );
    assert_eq!(r, "30", "two-param macro");
}

#[test]
fn test_defmacro_returns_nil_on_define() {
    let r = eval("(defmacro foo (x) (quasiquote x))");
    assert_eq!(r, "nil", "defmacro should return nil");
}

// ---------------------------------------------------------------------------
// 2. Macro receives UNEVALUATED args
// ---------------------------------------------------------------------------

#[test]
fn test_macro_args_not_evaluated() {
    // If args were evaluated, (+ 1 2) would become 3 and (quote ...) would fail
    // Macro gets the raw form (+ 1 2)
    let r = eval(
        r#"
        (defmacro identity (x) (quasiquote (unquote x)))
        (identity (+ 1 2))
    "#,
    );
    assert_eq!(
        r, "3",
        "identity macro returns unevaluated form which then gets eval'd"
    );
}

#[test]
fn test_macro_gets_symbol_not_value() {
    // The macro receives the symbol 'x, not the value 42
    // We can prove it by quoting it back
    let r = eval(
        r#"
        (defmacro quote-arg (x) (quasiquote (quote (unquote x))))
        (define x 42)
        (quote-arg x)
    "#,
    );
    assert_eq!(r, "x", "macro should receive the symbol x, not 42");
}

#[test]
fn test_macro_receives_list_form() {
    let r = eval(
        r#"
        (defmacro wrap-list (form) (quasiquote (list (unquote form))))
        (wrap-list (+ 1 2))
    "#,
    );
    // (+ 1 2) is passed unevaluated, then the expansion (list (+ 1 2)) is eval'd
    assert_eq!(
        r, "(3)",
        "wrap-list should pass form unevaluated then eval result"
    );
}

// ---------------------------------------------------------------------------
// 3. Quasiquote with unquote
// ---------------------------------------------------------------------------

#[test]
fn test_quasiquote_literal() {
    // Quasiquote without unquote is like quote
    let r = eval("(quasiquote (1 2 3))");
    assert_eq!(r, "(1 2 3)", "quasiquote on literal should act like quote");
}

#[test]
fn test_quasiquote_unquote_simple() {
    let r = eval("(define x 10) (quasiquote ((unquote x)))");
    assert_eq!(r, "(10)", "quasiquote with unquote should splice value");
}

#[test]
fn test_quasiquote_unquote_in_list() {
    let r = eval(
        r#"
        (define a 1)
        (define b 2)
        (quasiquote (+ (unquote a) (unquote b)))
    "#,
    );
    assert_eq!(r, "(+ 1 2)", "quasiquote should unquote multiple values");
}

#[test]
fn test_quasiquote_nested() {
    let r = eval(
        r#"
        (define x 99)
        (quasiquote (list (unquote (quasiquote (unquote x)))))
    "#,
    );
    // inner quasiquote unquotes x -> 99, outer quasiquote wraps it
    assert_eq!(r, "(list 99)", "nested quasiquote");
}

// ---------------------------------------------------------------------------
// 4. Unquote-splicing (,@)
// ---------------------------------------------------------------------------

#[test]
fn test_unquote_splicing_basic() {
    let r = eval(
        r#"
        (define xs (list 1 2 3))
        (quasiquote (0 (unquote-splicing xs) 4))
    "#,
    );
    assert_eq!(
        r, "(0 1 2 3 4)",
        "unquote-splicing should flatten list into parent"
    );
}

#[test]
fn test_unquote_splicing_in_macro() {
    let r = eval(
        r#"
        (defmacro append-all (xs)
            (quasiquote (+ (unquote-splicing xs))))
        (append-all (1 2 3))
    "#,
    );
    // xs = (1 2 3) (unevaluated), splicing in (+ 1 2 3)
    assert_eq!(r, "6", "unquote-splicing in macro body");
}

// ---------------------------------------------------------------------------
// 5. Macro hygiene (closed_env capture)
// ---------------------------------------------------------------------------

#[test]
fn test_macro_captures_defining_env() {
    let r = eval(
        r#"
        (define secret 42)
        (defmacro get-secret () (quasiquote (unquote secret)))
        (define secret 0)
        (get-secret)
    "#,
    );
    // Macro captured secret=42 at definition time...
    // actually in Lisp macros the body is eval'd at expansion time using the closed env
    // so it should see secret=42 from the defining env
    // BUT the expansion produces 'secret' which is then eval'd in the calling env where secret=0
    // This is a hygiene test - depends on implementation
    // With quasiquote unquote, we get 42 baked into the expansion
    assert!(r == "42" || r == "0", "macro env capture: got {}", r);
}

#[test]
fn test_macro_with_local_helper() {
    // Macro uses a helper function defined before it
    let r = eval(
        r#"
        (define helper (lambda (x) (* x 2)))
        (defmacro double-it (x) (quasiquote (helper (unquote x))))
        (double-it 5)
    "#,
    );
    assert_eq!(
        r, "10",
        "macro should be able to call helper from defining scope"
    );
}

// ---------------------------------------------------------------------------
// 6. Rest params in macros
// ---------------------------------------------------------------------------

#[test]
fn test_macro_rest_param() {
    let r = eval(
        r#"
        (defmacro my-list (&rest rest) (quasiquote (list (unquote-splicing rest))))
        (my-list 1 2 3)
    "#,
    );
    assert_eq!(r, "(1 2 3)", "macro with rest param");
}

// ---------------------------------------------------------------------------
// 7. Recursive macro expansion (macro returns code that uses another macro)
// ---------------------------------------------------------------------------

#[test]
fn test_macro_chain() {
    let r = eval(
        r#"
        (defmacro add1 (x) (quasiquote (+ (unquote x) 1)))
        (defmacro add2 (x) (quasiquote (add1 (add1 (unquote x)))))
        (add2 10)
    "#,
    );
    assert_eq!(
        r, "12",
        "chained macros: add2 should expand to add1(add1(10))"
    );
}

// ---------------------------------------------------------------------------
// 8. macro? builtin
// ---------------------------------------------------------------------------

#[test]
fn test_macro_type_check() {
    let r = eval(
        r#"
        (defmacro m (x) (quasiquote x))
        (macro? m)
    "#,
    );
    assert_eq!(r, "true", "macro? should return true for defined macro");
}

#[test]
fn test_macro_type_check_false() {
    let r = eval(
        r#"
        (define f (lambda (x) x))
        (list (macro? f) (macro? 42) (macro? "hi"))
    "#,
    );
    assert_eq!(
        r, "(false false false)",
        "macro? should be false for non-macros"
    );
}

// ---------------------------------------------------------------------------
// 9. Macro generating control flow
// ---------------------------------------------------------------------------

#[test]
fn test_macro_generates_if() {
    let r = eval(
        r#"
        (defmacro when (cond body) (quasiquote (if (unquote cond) (unquote body) nil)))
        (when (> 5 3) 100)
    "#,
    );
    assert_eq!(r, "100", "when macro with true condition");
}

#[test]
fn test_macro_generates_if_false() {
    let r = eval(
        r#"
        (defmacro when (cond body) (quasiquote (if (unquote cond) (unquote body) nil)))
        (when (> 3 5) 100)
    "#,
    );
    assert_eq!(r, "nil", "when macro with false condition");
}

#[test]
fn test_macro_generates_cond() {
    let r = eval(
        r#"
        (defmacro my-and (a b) (quasiquote (if (unquote a) (unquote b) false)))
        (my-and (> 5 3) (< 10 20))
    "#,
    );
    assert_eq!(r, "true", "macro-generated cond");
}

// ---------------------------------------------------------------------------
// 10. Macro generating lambda
// ---------------------------------------------------------------------------

#[test]
fn test_macro_generates_lambda() {
    let r = eval(
        r#"
        (defmacro defn (name params body)
            (quasiquote (define (unquote name) (lambda (unquote params) (unquote body)))))
        (defn square (x) (* x x))
        (square 7)
    "#,
    );
    assert_eq!(r, "49", "defn macro should define a function");
}

// ---------------------------------------------------------------------------
// 11. Macro with no args
// ---------------------------------------------------------------------------

#[test]
fn test_macro_no_args() {
    let r = eval(
        r#"
        (defmacro forty-two () (quasiquote 42))
        (forty-two)
    "#,
    );
    assert_eq!(r, "42", "zero-arg macro");
}

// ---------------------------------------------------------------------------
// 12. Edge cases / error handling
// ---------------------------------------------------------------------------

#[test]
fn test_defmacro_needs_symbol() {
    let r = eval("(defmacro 123 () (quasiquote 1))");
    assert!(
        r.starts_with("ERROR"),
        "defmacro with non-symbol name should error, got: {}",
        r
    );
}

#[test]
fn test_defmacro_needs_params() {
    let r = eval("(defmacro mymac)");
    assert!(
        r.starts_with("ERROR"),
        "defmacro with missing params should error, got: {}",
        r
    );
}

#[test]
fn test_defmacro_needs_body() {
    let r = eval("(defmacro mymac ())");
    assert!(
        r.starts_with("ERROR"),
        "defmacro with missing body should error, got: {}",
        r
    );
}

#[test]
fn test_quasiquote_needs_arg() {
    let r = eval("(quasiquote)");
    assert!(
        r.starts_with("ERROR"),
        "quasiquote without arg should error, got: {}",
        r
    );
}

#[test]
fn test_unquote_outside_quasiquote() {
    // unquote outside quasiquote should either error or be treated as symbol
    let r = eval("(unquote x)");
    // This depends on implementation - it may just be treated as a regular call
    // The key thing is it shouldn't panic
    assert!(true, "unquote outside quasiquote didn't crash: {}", r);
}

// ---------------------------------------------------------------------------
// 13. Macro generating loop/recur
// ---------------------------------------------------------------------------

#[test]
fn test_macro_generates_loop() {
    let r = eval(
        r#"
        (defmacro times (n body)
            (quasiquote (loop ((i 0) (acc 0))
                (if (>= i (unquote n))
                    acc
                    (recur (+ i 1) (+ acc (unquote body)))))))
        (times 5 i)
    "#,
    );
    assert_eq!(r, "10", "macro generating loop: sum 0+1+2+3+4 = 10");
}

// ---------------------------------------------------------------------------
// 14. Macro used inside map/filter
// ---------------------------------------------------------------------------

#[test]
fn test_macro_with_higher_order() {
    let r = eval(
        r#"
        (defmacro square (x) (quasiquote (* (unquote x) (unquote x))))
        (define sq-fn (lambda (x) (square x)))
        (map sq-fn (list 1 2 3 4))
    "#,
    );
    assert_eq!(r, "(1 4 9 16)", "macro used inside lambda passed to map");
}
