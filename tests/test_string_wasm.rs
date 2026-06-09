use lisp_rlm_wasm::wasm_emit::compile_pure;

#[test]
fn test_str_length_compiles() {
    let src = r#"
(define (my-len s)
    (str-length s))
"#;
    let wasm = compile_pure(src).expect("str-length should compile");
    assert!(!wasm.is_empty(), "WASM should not be empty");
    let wat = wasmprinter::print_bytes(&wasm).expect("WASM should be valid");
    // Verify it compiles to a shr_u instruction (extract length from tagged string)
    assert!(
        wat.contains("i64.shr_u"),
        "str-length should emit shr_u to extract length"
    );
}

#[test]
fn test_str_substring_compiles() {
    let src = r#"
(define (my-sub s start end)
    (str-substring s start end))
"#;
    let wasm = compile_pure(src).expect("str-substring should compile");
    assert!(!wasm.is_empty());
}

#[test]
fn test_str_contains_compiles() {
    let src = r#"
(define (has-world s)
    (str-contains s "world"))
"#;
    let wasm = compile_pure(src).expect("str-contains should compile");
    assert!(!wasm.is_empty());
}

#[test]
fn test_str_index_of_compiles() {
    let src = r#"
(define (find-at s)
    (str-index-of s "at"))
"#;
    let wasm = compile_pure(src).expect("str-index-of should compile");
    assert!(!wasm.is_empty());
}

#[test]
fn test_str_repeat_compiles() {
    let src = r#"
(define (echo s)
    (str-repeat s 3))
"#;
    let wasm = compile_pure(src).expect("str-repeat should compile");
    assert!(!wasm.is_empty());
}

#[test]
fn test_string_length_alias_compiles() {
    let src = r#"
(define (my-len s)
    (string-length s))
"#;
    let wasm = compile_pure(src).expect("string-length should compile");
    assert!(!wasm.is_empty());
}

#[test]
fn test_str_contains_with_dynamic_haystack() {
    let src = r#"
(define (check-hello s)
    (if (= (str-contains s "hello") 1) 1 0))
"#;
    let wasm = compile_pure(src).expect("str-contains in if should compile");
    assert!(!wasm.is_empty());
}
