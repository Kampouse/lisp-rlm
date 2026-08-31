//! Arrow-function block bodies (M2 lowering, 2026-08-31).
//! Multi-statement arrow bodies reuse function-body statement lowering —
//! the case that motivated this: `(v) => { console.log(v); return v + 1; }`
//! was a hard M1 error, blocking even basic debugging.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn lower(src: &str) -> String {
    ts_to_lisp_source(src).expect("must lower")
}

fn compile(src: &str) {
    let ir = lower(src);
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

/// The motivating case: console.log + return in one arrow body.
#[test]
fn arrow_console_log_then_return_compiles() {
    compile(
        "function f(xs: number[]): number {\n  const g = (v: number): number => { console.log(v); return v + 1; };\n  return g(xs[0]);\n}\n",
    );
}

/// const binding inside an arrow body lowers to let.
#[test]
fn arrow_const_binding_compiles() {
    compile(
        "function f(v: number): number {\n  const g = (x: number): number => { const y = x * 2; return y + 1; };\n  return g(21);\n}\n",
    );
}

/// Early return mid-body (flag-guard path).
#[test]
fn arrow_early_return_compiles() {
    compile(
        "function f(v: number): number {\n  const g = (x: number): number => { if (x < 0) { return 0; } return x; };\n  return g(-5);\n}\n",
    );
}

/// Multi-statement arrow used as a .map callback (the resolve_lambda inline path).
#[test]
fn arrow_multistmt_map_callback_compiles() {
    compile(
        "function f(xs: number[]): number[] {\n  return xs.map((x: number): number => { console.log(x); return x * 2; });\n}\n",
    );
}

/// Shape stability: single-statement bodies must NOT grow a begin wrapper.
#[test]
fn single_statement_bodies_stay_unwrapped() {
    let expr_ir = lower("function f(v: number): number { return v + 1; }\n");
    assert!(!expr_ir.contains("begin"), "expr-body drifted: {}", expr_ir);

    let ret_ir = lower(
        "function f(v: number): number {\n  const g = (x: number): number => { return x + 1; };\n  return g(v);\n}\n",
    );
    // the g body itself must be the bare (+ x 1), not (begin (+ x 1))
    assert!(ret_ir.contains("(+ x 1))"), "return-body drifted: {}", ret_ir);
}

/// Empty block arrow still hard-errors (no silent undefined).
#[test]
fn empty_arrow_body_still_errors() {
    let r = ts_to_lisp_source(
        "function f(v: number): number {\n  const g = (x: number): number => {};\n  return g(v);\n}\n",
    );
    assert!(r.is_err(), "empty arrow body must not silently compile");
}

/// `export const f = arrow` → real entry point (function-shaped define).
#[test]
fn exported_named_arrow_compiles_to_entry() {
    compile("export const double = (x: number): number => { console.log(x); return x * 2; };\n");
}

/// Expression-bodied exported arrow (fast path).
#[test]
fn exported_arrow_expression_body_compiles() {
    compile("export const add1 = (x: number): number => x + 1;\n");
}

/// Exported non-arrow const stays a hard error.
#[test]
fn exported_non_arrow_const_errors() {
    let r = ts_to_lisp_source("export const ANSWER = 42;\n");
    assert!(r.is_err(), "non-arrow export must fail loud");
    assert!(
        r.unwrap_err().contains("arrow initializer"),
        "error must point at the fix"
    );
}

/// get_* exported arrow inherits the view convention.
#[test]
fn exported_get_arrow_is_view() {
    let ir = lower("export const get_count = (): number => 7;\n");
    assert!(ir.contains("#t"), "get_* must be a view export: {}", ir);
}
