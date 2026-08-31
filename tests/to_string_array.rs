//! (to-string <array>) — wasm renders "[e0, e1, ...]" (2026-08-31).
//! Previously arrays fell into int_to_str_clean's NUM path and logged the
//! raw heap pointer as decimal ("327680"). Now: flat rendering of
//! num/bool/nil/str elements, nested arrays as `<vec>`, strings quoted —
//! matching the interpreter's LispVal::Vec Display format.
//!
//! Runtime behavior machine-verified via the node instrumented-host harness:
//!   console.log([1, 2, 3])            → "[1, 2, 3]"
//!   console.log(["hi", -5, 0, 42])    → "[\"hi\", -5, 0, 42]"
//!   console.log([])                   → "[]"
//! (node proof lives in the session log; these tests pin the compile path.)

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

#[test]
fn ts_console_log_array_compiles() {
    let src = "export const main = (): number => {\n  const xs = [1, 2, 3];\n  console.log(xs);\n  return xs.length;\n}\n";
    let ir = ts_to_lisp_source(src).expect("must lower");
    assert!(ir.contains("(to-string xs)"), "IR: {}", ir);
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must check");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

#[test]
fn to_string_array_direct_sexp_compiles() {
    let src = r#"(define (main) (begin (near/log (to-string (array 1 2 3))) 0))"#;
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must check");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

/// Strings/empties exercise the quoted-element and zero-element paths.
#[test]
fn ts_console_log_mixed_and_empty_arrays_compile() {
    let src = "export const main = (): number => {\n  console.log([\"hi\", -5, 0, 42]);\n  console.log([]);\n  return 1;\n}\n";
    let ir = ts_to_lisp_source(src).expect("must lower");
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must check");
    lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
}
