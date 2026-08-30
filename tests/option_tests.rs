//! Option types — (opt T), (default v fallback), TS `??` sugar.
//! storage_get is honestly typed; unhandled nil is a compile error.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

#[test]
fn coalesce_lowers_to_default() {
    let ir = ts_to_lisp_source(
        "function g(k: string): number {\n  return strToNum(near.storageGet(k) ?? \"0\");\n}\n",
    )
    .unwrap();
    assert!(ir.contains("(default (near/storage_get k) \"0\")"), "IR: {}", ir);
}

#[test]
fn unhandled_option_is_compile_error() {
    let ir = ts_to_lisp_source(
        "function g(k: string): number {\n  return strToNum(near.storageGet(k));\n}\n",
    )
    .unwrap();
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    let err = lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect_err("must reject unhandled nil");
    assert!(err.contains("opt"), "error should mention opt: {}", err);
}

#[test]
fn default_compiles_and_runs_in_wasm() {
    let src = "(define (g) :: -> int (str->num (default (near/storage_get \"k\") \"7\")))\n(export \"g\" g #t)\n";
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("check");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("compile");
    assert!(wasm.len() > 100);
}

#[test]
fn opt_annotation_parses() {
    let src = "(define (f x) :: (opt str) -> str (default x \"d\"))\n(export \"f\" f #t)\n";
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("(opt str) must parse");
}

#[test]
fn default_rejects_non_option_value() {
    let src = "(define (f) :: -> str (default \"always-str\" \"d\"))\n(export \"f\" f #t)\n";
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    assert!(
        lisp_rlm_wasm::typing::type_check_program(&exprs, true).is_err(),
        "default on a non-nil value must fail"
    );
}
