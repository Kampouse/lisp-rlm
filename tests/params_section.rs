use lisp_rlm_wasm::wasm_emit::{compile_near, param_map};

#[test]
fn params_section_roundtrip() {
    let src = r#"
        (default
          (let ((who (near/json_get_str "account"))
                (n (near/json_get_int "count")))
            (near/log (str-concat "ping " who)))
          "fb")
    "#;
    let wasm = compile_near(src).expect("compile");
    let keys = param_map::decode_params_section(&wasm).expect("params section present");
    assert!(keys.contains(&"account".to_string()), "keys: {keys:?}");
    assert!(keys.contains(&"count".to_string()), "keys: {keys:?}");
}

#[test]
fn no_params_no_section() {
    let src = r#"(default (near/log "hi") "fb")"#;
    let wasm = compile_near(src).expect("compile");
    assert!(param_map::decode_params_section(&wasm).is_none());
}

#[test]
fn decode_name_section_still_works() {
    // sanity: appending our custom section doesn't break existing name-section decode
    let src = r#"(default (let ((x (near/json_get_str "k"))) x) "fb")"#;
    let wasm = compile_near(src).expect("compile");
    let names =
        lisp_rlm_wasm::wasm_emit::name_map::decode_function_names(&wasm).expect("name section");
    assert!(!names.is_empty());
}
