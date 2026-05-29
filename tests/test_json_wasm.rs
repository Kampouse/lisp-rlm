use lisp_rlm_wasm::wasm_emit::compile_pure;

fn to_wat(wasm: &[u8]) -> String {
    wasmprinter::print_bytes(wasm).expect("wasmprinter")
}

#[test]
fn test_json_get_compiles() {
    let src = r#"
(define (my-price resp)
    (json-get "price" resp))
"#;
    let wasm = compile_pure(src).expect("json-get should compile");
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit function calls: {}", &wat[..200.min(wat.len())]);
}

#[test]
fn test_json_get_str_compiles() {
    let src = r#"
(define (my-name resp)
    (json-get-str "name" resp))
"#;
    let wasm = compile_pure(src).expect("json-get-str should compile");
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit function calls: {}", &wat[..200.min(wat.len())]);
}

#[test]
fn test_json_get_float_compiles() {
    let src = r#"
(define (my-price resp)
    (json-get-float "price" resp))
"#;
    let wasm = compile_pure(src).expect("json-get-float should compile");
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit function calls: {}", &wat[..200.min(wat.len())]);
}

#[test]
fn test_json_extract_compiles() {
    let src = r#"
(define (my-extract resp)
    (json-extract resp "price" "name"))
"#;
    let wasm = compile_pure(src).expect("json-extract should compile");
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit function calls: {}", &wat[..200.min(wat.len())]);
}
