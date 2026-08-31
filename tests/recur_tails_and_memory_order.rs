//! Deliverable B: `recur` outside direct tail position must be a HARD
//! compile error on BOTH compile paths (bytecode interpreter + wasm
//! emitter) — never a silently-accepted plain call (GAPS t21).

use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    lisp_rlm_wasm::program::run_program(&parse_all(code).unwrap(), &mut env, &mut state)
        .map(|v| v.to_string())
        .unwrap_or_else(|e| format!("ERROR: {}", e))
}

// ── legit corpus tails keep working (interp) ──

#[test]
fn corpus_loop_tails_still_work() {
    // full_syntax.lisp §11 / all_syntax.lisp patterns
    assert_eq!(
        eval_str("(loop ((i 0)) (if (= i 5) i (recur (+ i 1))))"),
        "5"
    );
    assert_eq!(
        eval_str("(loop ((n 6) (acc 1)) (if (= n 0) acc (recur (- n 1) (* acc n))))"),
        "720"
    );
    assert_eq!(
        eval_str("(loop ((i 3) (acc ())) (if (= i 0) acc (recur (- i 1) (cons i acc))))"),
        "(1 2 3)"
    );
    // define wrapper (test_let_loop.lisp pattern)
    assert_eq!(
        eval_str(
            "(define (sum-list lst) (loop ((remaining lst) (acc 0)) (if (nil? remaining) acc (recur (cdr remaining) (+ acc (car remaining)))))) (sum-list (list 1 2 3 4 5))"
        ),
        "15"
    );
}

// ── interp path: hard errors ──

#[test]
fn interp_recur_outside_loop_is_error() {
    assert!(eval_str("(recur 1)").starts_with("ERROR"));
}

#[test]
fn interp_non_tail_recur_is_error() {
    // operand position — the silent-hazard class
    assert!(eval_str("(loop ((i 0)) (+ 1 (recur 2)))").starts_with("ERROR"));
    // mid-begin
    assert!(eval_str("(loop ((i 0)) (begin (recur 1) 2))").starts_with("ERROR"));
    // arity mismatch
    assert!(
        eval_str("(loop ((i 0)) (if (= i 3) i (recur 1 2)))").starts_with("ERROR")
    );
}

// ── wasm path: hard compile errors ──

#[test]
fn wasm_non_tail_recur_is_compile_error() {
    let err = wasm_emit::compile_near_untyped(
        "(define (f) (loop ((i 0)) (+ 1 (recur 2))))",
    )
    .unwrap_err();
    assert!(err.contains("not in tail position"), "got: {err}");
}

#[test]
fn wasm_recur_in_lambda_is_compile_error() {
    let err = wasm_emit::compile_near_untyped(
        "(define (f) (loop ((i 0)) ((lambda (x) (recur x)) 1)))",
    )
    .unwrap_err();
    assert!(
        err.contains("recur") && err.contains("loop"),
        "got: {err}"
    );
}

#[test]
fn wasm_recur_outside_loop_is_compile_error() {
    let err = wasm_emit::compile_near_untyped("(define (f) (recur 1))").unwrap_err();
    assert!(err.contains("recur"), "got: {err}");
}

#[test]
fn wasm_corpus_loop_tails_still_compile() {
    wasm_emit::compile_near_untyped(
        "(define (fact n) (loop ((n n) (acc 1)) (if (= n 0) acc (recur (- n 1) (* acc n)))))",
    )
    .expect("legit tail recur must still compile");
}

// ── deliverable C: (memory N) decl order ──

#[test]
fn memory_decl_order_does_not_matter() {
    // The memory declaration after the defines that (transitively) embed
    // mem_limit constants must yield the SAME module as decl-first.
    let body = "(define (alloc-it) (buf-alloc 1024)) (define (run) (alloc-it))";
    let a = wasm_emit::compile_near_untyped(&format!("(memory 17) {body}"))
        .expect("decl-first compiles");
    let b = wasm_emit::compile_near_untyped(&format!("{body} (memory 17)"))
        .expect("decl-last compiles");
    assert_eq!(
        a, b,
        "(memory N) after defines must emit identical wasm (hoisted like consts)"
    );
}
