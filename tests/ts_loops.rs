//! TS while/for loop lowering — M1.5 (2026-10-05).
//! Plain `for`/`while` were advertised for 5 weeks but hard-errored
//! ("not in M1 subset"); a stub-brain's instinctive for-loop died on the
//! first try. These tests lock the desugaring:
//!   for (init; test; update) body → (begin pre (let* binds (while test body... update)))
//!   while (test) body            → (while test body...)
//! with hard-errors on break/continue/return inside loop bodies (the
//! exit-protocol machinery exists only for for..of).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn lower(src: &str) -> String {
    ts_to_lisp_source(src).expect("must lower")
}

fn lower_err(src: &str) -> String {
    ts_to_lisp_source(src).expect_err("must NOT lower")
}

fn compile(src: &str) {
    let ir = lower(src);
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

/// Top-level for with let-init, assignment update — the exact shape the
/// stub-brain wrote when t1_sumsq@ts died.
#[test]
fn top_level_for_lowers_to_let_while() {
    let ir = lower(
        "let s: number = 0;\nfor (let i: number = 1; i <= 100; i = i + 1) { s = s + i * i; }\nrlm_set(\"answer\", s);\n",
    );
    assert!(ir.contains("(while"), "got: {ir}");
    assert!(ir.contains("let*"), "got: {ir}");
    assert!(ir.contains("(set! s"), "got: {ir}");
}

/// i++ update expressions lower as statements inside the loop body.
#[test]
fn top_level_for_iplus_update() {
    let ir = lower(
        "let s: number = 0;\nfor (let i: number = 0; i < 5; i++) { s = s + 1; }\nrlm_set(\"n\", s);\n",
    );
    assert!(ir.contains("(while"), "got: {ir}");
}

/// Top-level while.
#[test]
fn top_level_while_lowers() {
    let ir = lower(
        "let s: number = 0;\nlet i: number = 1;\nwhile (i <= 10) { s = s + i; i = i + 1; }\nrlm_set(\"answer\", s);\n",
    );
    assert!(ir.contains("(while"), "got: {ir}");
}

/// Function-body for loop: full lower + typecheck + wasm compile.
#[test]
fn function_for_loop_compiles() {
    compile(
        "export function sum(n: number): number {\n  let s: number = 0;\n  for (let i: number = 1; i <= n; i = i + 1) { s = s + i; }\n  return s;\n}\n",
    );
}

#[test]
fn function_while_loop_compiles() {
    compile(
        "export function count(n: number): number {\n  let s: number = 0;\n  let i: number = n;\n  while (i > 0) { s = s + 1; i = i - 1; }\n  return s;\n}\n",
    );
}

/// break/continue at TOP level hard-error with the fix hint (function
/// bodies keep the full exit-protocol path — see test below).
#[test]
fn loop_exits_hard_error_top_level() {
    let e = lower_err("for (let i: number = 0; i < 3; i++) { if (i === 1) { break; } }");
    assert!(e.contains("break/continue/return"), "got: {e}");
    let e = lower_err("let i: number = 0; while (i < 3) { if (i > 1) { continue; } i = i + 1; }");
    assert!(e.contains("break/continue/return"), "got: {e}");
}

/// Regression guard: `return` inside a function-body while keeps working
/// through the exit-protocol path (lower_while_parts) — the new top-level
/// arms must not shadow it.
#[test]
fn function_while_return_still_lowers() {
    let ir = lower(
        "export function f(n: number): number {\n  let i: number = 0;\n  while (i < 3) { i = i + 1; if (i === 2) { return i; } }\n  return 0;\n}\n",
    );
    assert!(ir.contains("__wl_ret"), "exit protocol gone: {ir}");
    assert!(ir.contains("(while"), "got: {ir}");
}

/// Top-level for..of — the gap the brain hit live at 14:08 (cheatsheet
/// advertised it, M1 catch rejected it). Reuses the function-body
/// machinery via lower_prefix_around.
#[test]
fn top_level_for_of_lowers() {
    let ir = lower(
        "const xs: number[] = [1, 2, 3];\nlet s: number = 0;\nfor (const x of xs) { s = s + x; }\nrlm_set(\"total\", s);\n",
    );
    assert!(ir.contains("(while"), "got: {ir}");
    assert!(ir.contains("vec-nth") || ir.contains("vec-length"), "iter machinery missing: {ir}");
}
