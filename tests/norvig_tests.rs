//! Norvig's lis.py test suite translated to lisp-rlm.
//!
//! These tests are from Peter Norvig's "lis.py" test set — the simpler collection
//! that excludes call/cc, define-macro, complex numbers, let, and/or shortcuts.
//!
//! Adaptations for lisp-rlm:
//!   - `null?` → `nil?`   (lisp-rlm uses nil?)
//!   - `length` → `len`   (lisp-rlm uses len)
//!   - `'x` → `(quote x)` (quote shorthand not tokenized)
//!
//! NOTE: The evaluator emits debug traces via eprintln!(\"[call_val]...\") and
//! eprintln!(\"[apply_lambda]...\"). These are unconditional in the source and
//! will appear on stderr during `cargo test`. They do not affect correctness.

use std::sync::atomic::{AtomicUsize, Ordering};

use lisp_rlm::EvalState;
use lisp_rlm::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Evaluate all expressions in `code` sequentially, return the last result
/// as a Display string. Errors are returned prefixed with "ERROR:".
fn run(code: &str, env: &mut Env, state: &mut EvalState) -> String {
    let exprs = match parse_all(code) {
        Ok(e) => e,
        Err(e) => return format!("ERROR: {}", e),
    };
    let mut result = LispVal::Nil;
    for expr in &exprs {
        match lisp_eval(expr, env, state) {
            Ok(v) => result = v,
            Err(e) => return format!("ERROR: {}", e),
        }
    }
    result.to_string()
}

// ---------------------------------------------------------------------------
// Test counters for summary
// ---------------------------------------------------------------------------

static PASS: AtomicUsize = AtomicUsize::new(0);
static FAIL: AtomicUsize = AtomicUsize::new(0);

fn check(name: &str, actual: &str, expected: &str) {
    if actual == expected {
        PASS.fetch_add(1, Ordering::SeqCst);
    } else {
        FAIL.fetch_add(1, Ordering::SeqCst);
        eprintln!("FAIL [{}]: expected {:?}, got {:?}", name, expected, actual);
    }
}

// ---------------------------------------------------------------------------
// Norvig lis_tests
// ---------------------------------------------------------------------------

#[test]
fn norvig_lis_tests() {
    // We run all Norvig lis_tests inside one #[test] so that shared-env tests
    // (define x, then use x) work naturally, and we can print a summary.

    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut local_pass = 0usize;
    let mut local_fail = 0usize;

    // ---- Test 1: quote -------------------------------------------------
    // Norvig: (quote (testing 1 (2.0) -3.14e159)) → (testing 1 (2.0) -3.14e159))
    // lisp-rlm float formatting trims trailing zeros: 2.0 → "2".
    // -3.14e159 is representable as f64; Display formats it as a huge decimal.
    // We verify the quote returns structure rather than matching exact float output.
    {
        let r = run("(quote (testing 1 (2.0) -3.14e159))", &mut env, &mut state);
        // Verify: starts with (testing 1 (2 ...) and the structure is preserved
        if r.starts_with("(testing 1 (2) -") || r.starts_with("(testing 1 (2.0) -") {
            check("quote_basic", "ok", "ok");
            local_pass += 1;
        } else {
            check("quote_basic", &r, "(testing ...)");
            local_fail += 1;
        }
    }

    // ---- Test 2: (+ 2 2) → 4 ------------------------------------------
    {
        let r = run("(+ 2 2)", &mut env, &mut state);
        check("+2+2", &r, "4");
        if r == "4" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 3: (+ (* 2 100) (* 1 10)) → 210 -------------------------
    {
        let r = run("(+ (* 2 100) (* 1 10))", &mut env, &mut state);
        check("+nested_mul", &r, "210");
        if r == "210" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 4: (if (> 6 5) (+ 1 1) (+ 2 2)) → 2 ---------------------
    {
        let r = run("(if (> 6 5) (+ 1 1) (+ 2 2))", &mut env, &mut state);
        check("if_true", &r, "2");
        if r == "2" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 5: (if (< 6 5) (+ 1 1) (+ 2 2)) → 4 ---------------------
    {
        let r = run("(if (< 6 5) (+ 1 1) (+ 2 2))", &mut env, &mut state);
        check("if_false", &r, "4");
        if r == "4" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 6: define / variable lookup ------------------------------
    {
        let r1 = run("(define x 3)", &mut env, &mut state);
        check("define_x", &r1, "nil");
        if r1 == "nil" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }

        let r2 = run("x", &mut env, &mut state);
        check("lookup_x", &r2, "3");
        if r2 == "3" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }

        let r3 = run("(+ x x)", &mut env, &mut state);
        check("+x+x", &r3, "6");
        if r3 == "6" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 7: begin / set! ------------------------------------------
    {
        let r = run(
            "(begin (define y 1) (set! y (+ y 1)) (+ y 1))",
            &mut env,
            &mut state,
        );
        check("begin_set!", &r, "3");
        if r == "3" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 8: immediate lambda application --------------------------
    {
        let r = run("((lambda (x) (+ x x)) 5)", &mut env, &mut state);
        check("immed_lambda", &r, "10");
        if r == "10" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 9: define + call lambda ----------------------------------
    {
        let r = run(
            "(define twice (lambda (x) (* 2 x))) (twice 5)",
            &mut env,
            &mut state,
        );
        check("twice_5", &r, "10");
        if r == "10" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 10: compose (higher-order) -------------------------------
    {
        run(
            "(define compose (lambda (f g) (lambda (x) (f (g x)))))",
            &mut env,
            &mut state,
        );
        // (compose list twice) → λx. list(twice(x))
        // list(twice(5)) = list(10) = (10)
        let r = run("((compose list twice) 5)", &mut env, &mut state);
        check("compose_list_twice", &r, "(10)");
        if r == "(10)" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 11: repeat (compose f f) ---------------------------------
    {
        run(
            "(define repeat (lambda (f) (compose f f)))",
            &mut env,
            &mut state,
        );
        // (repeat twice) = compose(twice, twice) = λx. twice(twice(x))
        // twice(5)=10, twice(10)=20
        let r1 = run("((repeat twice) 5)", &mut env, &mut state);
        check("repeat_twice_5", &r1, "20");
        if r1 == "20" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }

        // (repeat (repeat twice)) = compose(R, R) where R=(repeat twice)
        // R(5) = twice(twice(5)) = 20
        // (repeat (repeat twice))(5) = R(R(5)) = R(20) = twice(twice(20)) = 80
        let r2 = run("((repeat (repeat twice)) 5)", &mut env, &mut state);
        check("repeat_repeat_twice_5", &r2, "80");
        if r2 == "80" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 12: factorial (recursive lambda) -------------------------
    {
        run(
            "(define fact (lambda (n) (if (<= n 1) 1 (* n (fact (- n 1))))))",
            &mut env,
            &mut state,
        );
        let r1 = run("(fact 3)", &mut env, &mut state);
        check("fact_3", &r1, "6");
        if r1 == "6" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }

        // fact(10) = 3628800
        // NOTE: fact(50) would overflow i64 (lisp-rlm uses i64 for integers).
        // Norvig's lis.py uses Python bignums which don't overflow.
        let r2 = run("(fact 10)", &mut env, &mut state);
        let expected = "3628800";
        check("fact_10", &r2, expected);
        if r2 == expected {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 13: abs (first-class operators via if) -------------------
    // ((if (> n 0) + -) 0 n) — uses + or - as first-class callable.
    // lisp-rlm supports this: builtin symbols resolve to themselves and
    // call_val dispatches them. This works because the args (0, n) are
    // simple numbers that survive re-evaluation.
    {
        run(
            "(define abs (lambda (n) ((if (> n 0) + -) 0 n)))",
            &mut env,
            &mut state,
        );
        let r = run("(list (abs -3) (abs 0) (abs 3))", &mut env, &mut state);
        check("abs_list", &r, "(3 0 3)");
        if r == "(3 0 3)" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 14: combine / zip (higher-order recursive) ---------------
    // Builtins (append, cons) are now proper first-class values via dispatch_call_with_args.
    {
        run(
            r#"(define combine (lambda (f)
              (lambda (x y)
                (if (nil? x) (quote ())
                    (f (list (car x) (car y))
                       ((combine f) (cdr x) (cdr y)))))))"#,
            &mut env,
            &mut state,
        );
        // (combine append) merges element-by-element
        let r1 = run(
            r#"((combine append) (quote (a b c)) (list 1 2 3))"#,
            &mut env,
            &mut state,
        );
        check("combine_append", &r1, "(a 1 b 2 c 3)");
        if r1 == "(a 1 b 2 c 3)" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }

        // (combine cons) → zip (Norvig's original uses combine cons)
        run("(define zip (combine cons))", &mut env, &mut state);
        let r2 = run("(zip (list 1 2 3) (list 4 5 6))", &mut env, &mut state);
        check("zip_list", &r2, "((1 4) (2 5) (3 6))");
        if r2 == "((1 4) (2 5) (3 6))" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Test 15: riff-shuffle (complex nested, uses combine) ----------
    // Adapted from Norvig: uses top-level take/drop + combine _append.
    {
        run(
            "(define take (lambda (n seq) (if (<= n 0) (quote ()) (cons (car seq) (take (- n 1) (cdr seq))))))",
            &mut env,
        &mut state,
        );
        run(
            "(define drop (lambda (n seq) (if (<= n 0) seq (drop (- n 1) (cdr seq)))))",
            &mut env,
            &mut state,
        );

        // riff-shuffle: split deck in half, interleave with combine append
        run(
            "(define riff-shuffle (lambda (deck) ((combine append) (take (/ (len deck) 2) deck) (drop (/ (len deck) 2) deck))))",
            &mut env,
        &mut state,
        );

        let r = run("(riff-shuffle (list 1 2 3 4 5 6))", &mut env, &mut state);
        // take 3 of (1 2 3 4 5 6) → (1 2 3)
        // drop 3 of (1 2 3 4 5 6) → (4 5 6)
        // combine append: (1 4 2 5 3 6)
        check("riff_shuffle", &r, "(1 4 2 5 3 6)");
        if r == "(1 4 2 5 3 6)" {
            local_pass += 1;
        } else {
            local_fail += 1;
        }
    }

    // ---- Summary -------------------------------------------------------
    eprintln!("\n===== Norvig lis_tests Summary =====");
    eprintln!("  PASS: {}", local_pass);
    eprintln!("  FAIL: {}", local_fail);
    eprintln!("  SKIP: 0");
    eprintln!("  TOTAL: {}", local_pass + local_fail);
    eprintln!("====================================\n");

    // Fail the test if anything failed
    assert_eq!(
        local_fail, 0,
        "{} Norvig tests failed — see stderr for details",
        local_fail
    );
}
