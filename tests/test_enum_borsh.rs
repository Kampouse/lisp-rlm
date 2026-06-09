#[test]
fn test_enum_serialize_wat() {
    let src = r#"(borsh-schema (Color Red Green Blue))
(define (test) (borsh-serialize "Color" 1))
(export "run" test)"#;
    let wasm = lisp_rlm_wasm::compile_pure(src).unwrap();
    let wat = wasmprinter::print_bytes(&wasm).unwrap();
    eprintln!("WAT:\n{}", wat);
    // Validate the WASM
    let validation = wasmparser::validate(&wasm);
    assert!(validation.is_ok(), "WASM validation failed");
}
