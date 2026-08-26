#[test]
fn dump_wasm_full() {
    let src = r#"(borsh-schema (MyResult (value (Option i64))))
(define (deser) (borsh-deserialize "MyResult" 36864))
(export "run" deser)"#;
    let wasm = lisp_rlm_wasm::compile_pure(src).unwrap();
    let wat = wasmprinter::print_bytes(&wasm).unwrap();
    eprintln!("FULL WAT:\n{}", wat);
}
