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
    assert!(
        wat.contains("call"),
        "should emit function calls: {}",
        &wat[..200.min(wat.len())]
    );
}

#[test]
fn test_json_get_str_compiles() {
    let src = r#"
(define (my-name resp)
    (json-get-str "name" resp))
"#;
    let wasm = compile_pure(src).expect("json-get-str should compile");
    let wat = to_wat(&wasm);
    assert!(
        wat.contains("call"),
        "should emit function calls: {}",
        &wat[..200.min(wat.len())]
    );
}

#[test]
fn test_json_get_float_compiles() {
    let src = r#"
(define (my-price resp)
    (json-get-float "price" resp))
"#;
    let wasm = compile_pure(src).expect("json-get-float should compile");
    let wat = to_wat(&wasm);
    assert!(
        wat.contains("call"),
        "should emit function calls: {}",
        &wat[..200.min(wat.len())]
    );
}

#[test]
fn test_json_extract_compiles() {
    let src = r#"
(define (my-extract resp)
    (json-extract resp "price" "name"))
"#;
    let wasm = compile_pure(src).expect("json-extract should compile");
    let wat = to_wat(&wasm);
    assert!(
        wat.contains("call"),
        "should emit function calls: {}",
        &wat[..200.min(wat.len())]
    );
}

// ── 1-arg json-get (implicit input scanner) — near-compile + near-mock pins ──
// Regression for the __json_get not-found guard: the scan loop exits when
// scan_i + pat_len > json_len, but the old guard only checked
// scan_i >= json_len, so key-misses fell through to value extraction and
// returned garbage (len-0 spans) instead of NIL.
use std::process::Command;

fn near_json_probe(body: &str, args_json: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let lisp = format!("/tmp/__json_probe_{id}.lisp");
    let wasm = format!("/tmp/__json_probe_{id}.wasm");
    let src = format!("(define (run x) {})\n", body);
    std::fs::write(&lisp, &src).unwrap();
    let out = Command::new("./target/debug/near-compile")
        .args([&lisp, "--out", &wasm])
        .output()
        .expect("near-compile");
    assert!(out.status.success(), "compile failed: {}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new("./target/debug/near-mock")
        .args([&wasm, "_run", args_json])
        .output()
        .expect("near-mock");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    stdout
        .lines()
        .find(|l| l.starts_with("📄"))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| panic!("no result line in: {}", stdout))
}

#[test]
fn test_json_get_1arg_numeric() {
    let line = near_json_probe(r#"(json-get "amount")"#, r#"{"amount":42}"#);
    assert!(line.contains("42"), "numeric: {}", line);
}

#[test]
fn test_json_get_1arg_string() {
    // string result → untagged payload (len << 32 | ptr), len == 5 ("world")
    let line = near_json_probe(r#"(json-get "name")"#, r#"{"name":"world"}"#);
    let payload: i64 = line
        .split_whitespace()
        .find(|t| t.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .and_then(|t| t.parse().ok())
        .expect("payload i64");
    assert_eq!(payload >> 32, 5, "string len 5: {}", line);
}

#[test]
fn test_json_get_1arg_missing_key_returns_nil() {
    let line = near_json_probe(r#"(json-get "amount")"#, r#"{"name":"world"}"#);
    assert!(line.contains("0 (raw"), "missing key must return NIL/0: {}", line);
}
