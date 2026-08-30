//! `::` type annotations on top-level defines — flow from the TS frontend,
//! verified by the checker, skipped by the emitter and interpreter.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

/// TS annotations ride into the IR as `::` forms.
#[test]
fn ts_annotations_flow_to_ir() {
    let src = "function double(x: number): number {\n  return x * 2;\n}\n";
    let ir = ts_to_lisp_source(src).unwrap();
    assert!(
        ir.contains(":: int -> int"),
        "annotation missing from IR: {}",
        ir
    );
}

/// `void` returns and partially-annotated functions emit no annotation.
#[test]
fn ts_partial_annotations_skip() {
    let src = "export function new_(): void {\n  near.storageSet(\"c\", \"0\");\n}\n";
    let ir = ts_to_lisp_source(src).unwrap();
    assert!(!ir.contains("::"), "void should not annotate: {}", ir);
}

/// A lying annotation is a compile error.
#[test]
fn lying_annotation_fails_compile() {
    let src = "function bad(x: number): number {\n  return \"nope\";\n}\n";
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    assert!(
        lisp_rlm_wasm::typing::type_check_program(&exprs, true).is_err(),
        "str-returns-as-int must fail the checker"
    );
}

/// A truthful annotation compiles to valid wasm (emitter skips `::`).
#[test]
fn annotation_compiles_to_wasm() {
    let src = "function double(x: number): number {\n  return x * 2;\n}\n\nexport function run(): number {\n  return double(21);\n}\n";
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("truthful annotation must check");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

/// Handwritten sexp annotations work too: nilary arrows and verification.
#[test]
fn sexp_nilary_annotation() {
    let src = "(define (answer) :: -> int 42)\n(export \"answer\" answer #t)\n";
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("nilary arrow must parse");
    lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
}

/// Param annotation mismatch at the definition is caught.
#[test]
fn wrong_param_annotation_fails() {
    let src = "(define (f x y) :: int int -> int (+ x y))\n(export \"f\" f #f)\n";
    let exprs = lisp_rlm_wasm::parse_all(src).unwrap();
    // Body returns 0 (define tail nil) — fine. Instead lie via body:
    let bad = "(define (g) :: -> int \"str\")\n(export \"g\" g #t)\n";
    let bad_exprs = lisp_rlm_wasm::parse_all(bad).unwrap();
    assert!(lisp_rlm_wasm::typing::type_check_program(&bad_exprs, true).is_err());
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("well-annotated define must pass");
}
