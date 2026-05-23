//! Integration test: compile Lisp → P2 WASM component, run via `inlayer run`.
//!
//! Requires:
//!   - `emit_p2` binary in PATH or built via `cargo run --bin emit_p2`
//!   - `inlayer` binary in PATH
//!   - Network access to httpbin.org
//!
//! Skipped automatically if either binary is missing.

use std::process::Command;

fn find_bin(name: &str) -> Option<String> {
    if let Ok(output) = Command::new("which").arg(name).output() {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    None
}

fn inlayer_bin() -> Option<String> {
    find_bin("inlayer")
}

/// Helper: compile Lisp source to a P2 WASM component file.
fn compile_p2(lisp: &str, output_path: &str) -> Result<(), String> {
    // Write lisp source to a temp file
    let src_path = format!("/tmp/lisp_test_{}.lisp", std::process::id());
    std::fs::write(&src_path, lisp).map_err(|e| e.to_string())?;

    let output = Command::new("cargo")
        .args(["run", "--release", "--bin", "emit_p2", "--", &src_path, output_path])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .map_err(|e| format!("failed to run emit_p2: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "emit_p2 failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Verify the component file exists
    if !std::path::Path::new(output_path).exists() {
        return Err(format!("output file not created: {}", output_path));
    }

    Ok(())
}

/// Helper: run a P2 component via inlayer and check success.
fn run_inlayer(wasm_path: &str, input: &str) -> Result<String, String> {
    let inlayer = inlayer_bin().ok_or("inlayer not found in PATH")?;

    let output = Command::new(&inlayer)
        .args(["run", wasm_path, input])
        .output()
        .map_err(|e| format!("failed to run inlayer: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !stdout.contains("✅ Success: true") {
        return Err(format!("inlayer run failed:\n{}", stdout));
    }

    Ok(stdout)
}

#[test]
fn test_http_get_live_inlayer() {
    let inlayer = match inlayer_bin() {
        Some(b) => b,
        None => {
            eprintln!("skipping: inlayer not found");
            return;
        }
    };

    let wasm_path = "/tmp/test_int_get.wasm";
    let lisp = r#"(define (main)
  (begin
    (outlayer/http-get "https://httpbin.org/get")
    0))
"#;

    compile_p2(lisp, wasm_path).expect("compile failed");
    let output = run_inlayer(wasm_path, "{}").expect("inlayer run failed");
    assert!(output.contains("Success: true"), "unexpected output: {}", output);
}

#[test]
fn test_http_post_live_inlayer() {
    if inlayer_bin().is_none() {
        eprintln!("skipping: inlayer not found");
        return;
    };

    let wasm_path = "/tmp/test_int_post.wasm";
    let lisp = r#"(define (main)
  (begin
    (outlayer/http-post "https://httpbin.org/post" "{}")
    0))
"#;

    compile_p2(lisp, wasm_path).expect("compile failed");
    let output = run_inlayer(wasm_path, "{}").expect("inlayer run failed");
    assert!(output.contains("Success: true"), "unexpected output: {}", output);
}

#[test]
fn test_http_post_custom_ct_live_inlayer() {
    if inlayer_bin().is_none() {
        eprintln!("skipping: inlayer not found");
        return;
    };

    let wasm_path = "/tmp/test_int_post_ct.wasm";
    let lisp = r#"(define (main)
  (begin
    (outlayer/http-post "https://httpbin.org/post" "hello" "text/plain")
    0))
"#;

    compile_p2(lisp, wasm_path).expect("compile failed");
    let output = run_inlayer(wasm_path, "{}").expect("inlayer run failed");
    assert!(output.contains("Success: true"), "unexpected output: {}", output);
}

#[test]
fn test_str_concat_json_get_live_inlayer() {
    if inlayer_bin().is_none() {
        eprintln!("skipping: inlayer not found");
        return;
    };

    let wasm_path = "/tmp/test_int_str_json.wasm";
    let lisp = r#"(define (main)
  (let ((resp (outlayer/http-post "https://httpbin.org/post" "hello=world" "application/x-www-form-urlencoded")))
    (let ((url (outlayer/json-get resp "url")))
      (wasi/write_stdout (outlayer/str-concat "URL: " url)))))
"#;

    compile_p2(lisp, wasm_path).expect("compile failed");
    let output = run_inlayer(wasm_path, "{}").expect("inlayer run failed");
    assert!(output.contains("URL: https://httpbin.org/post"), "unexpected output: {}", output);
}
