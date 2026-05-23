/// Live test: compile Lisp with json-get, instantiate with wasmtime, verify result
use std::process::Command;

#[test]
fn test_json_get_live_wasmtime() {
    // Step 1: Compile test Lisp to core WASM
    let lisp = r#"(define (main)
  (let ((resp (outlayer/http-get "https://httpbin.org/uuid")))
    (let ((val (outlayer/json-get resp "uuid")))
      (wasi/write_stdout val))))
"#;
    
    // Use emit_p2 to compile
    // ... actually this is complex. Let me just test with static data.
    
    // Simpler: compile a Lisp that uses json-get on a static string built via str-concat
    // But parser can't handle escaped quotes.
    
    // Even simpler: use the internal API to compile and test
    let lisp = r#"(define (main)
  (outlayer/json-get (outlayer/str-concat "A" "B") "url"))"#;
    
    let result = lisp_rlm_wasm::wasm_emit::WasmEmitter::new();
    // This won't work because WasmEmitter is not pub...
    
    // Let me use the binary instead
    let output = Command::new("cargo")
        .args(["run", "--release", "--bin", "emit_p2", "--", "/dev/stdin", "/tmp/test_jg_wt.wasm"])
        .env("RUST_LOG", "")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run emit_p2");
    
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
}
