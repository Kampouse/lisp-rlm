#[test]
fn test_enum_deser_wat() {
    let src = r#"(borsh-schema (Color Red Green Blue))
(define (deser) (borsh-deserialize "Color" 36864))
(export "run" deser)"#;
    let wasm = lisp_rlm_wasm::compile_pure(src).unwrap();
    let wat = wasmprinter::print_bytes(&wasm).unwrap();
    eprintln!("WAT:\n{}", wat);
    let validation = wasmparser::validate(&wasm);
    assert!(validation.is_ok(), "WASM validation failed");
}