//! WASM validation tests for http/get and http/post HTTPLIB P2 path.
//! Uses wasm-tools validate for full WASM type checking.

use lisp_rlm_wasm::wasm_emit::compile_pure;

fn validate_wasm(wasm: &[u8], label: &str) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), wasm).unwrap();

    let output = std::process::Command::new("wasm-tools")
        .args(["validate", tmp.path().to_str().unwrap()])
        .output()
        .expect("wasm-tools not found");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "[{}] WASM validation failed: {}",
        label,
        stderr
    );
    println!("[{}] {} bytes — valid ✓", label, wasm.len());
}

#[test]
fn test_http_post_httplib_valid() {
    let src = r#"(define (send) (http/post "https://httpbin.org/post" "{}"))"#;
    let wasm = compile_pure(src).expect("compile failed");
    validate_wasm(&wasm, "httplib/http-post");
}

#[test]
fn test_http_get_httplib_valid() {
    let src = r#"(define (fetch) (http/get "https://httpbin.org/get"))"#;
    let wasm = compile_pure(src).expect("compile failed");
    validate_wasm(&wasm, "httplib/http-get");
}

#[test]
fn test_http_get_and_post_httplib_valid() {
    let src = r#"
(define (fetch) (http/get "https://httpbin.org/get"))
(define (send) (http/post "https://httpbin.org/post" "{}"))
"#;
    let wasm = compile_pure(src).expect("compile failed");
    validate_wasm(&wasm, "httplib/get+post");
}
