fn main() {
    let src = r#"(memory 4)
(define (f) (near/storage_get "k"))
(export "f" f true)"#;
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near_untyped(src).expect("compile");
    std::fs::write("/tmp/test_fuzz.wasm", &wasm).unwrap();
    println!("{} bytes", wasm.len());
}
