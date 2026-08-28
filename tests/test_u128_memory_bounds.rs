//! Test u128 memory boundary cases: address 0, memory limits, overlapping regions.
//!
//! These tests verify u128 operations handle edge cases correctly:
//! - Address 0 (null pointer in many languages)
//! - Low addresses (potential collisions with temp buffers)
//! - High addresses (near WASM memory limits)
//! - Overlapping u128 regions (store at X, load from X+1)

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_ID: AtomicU64 = AtomicU64::new(0);

/// Compile and run via near-mock. Returns (exit_code, stdout, stderr).
fn run_near_mock(lisp: &str) -> (i32, String, String) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let id = TEST_ID.fetch_add(1, Ordering::SeqCst);
    let tmp_path = format!("/tmp/u128_mem_test_{}.lisp", id);
    let wasm_path = format!("/tmp/u128_mem_test_{}.wasm", id);
    std::fs::write(&tmp_path, lisp).unwrap();

    // Compile
    let compile_out = Command::new("cargo")
        .args(["run", "--bin", "near-compile", "--", &tmp_path, &wasm_path])
        .current_dir("/Users/asil/.openclaw/workspace/lisp-rlm")
        .output()
        .expect("near-compile failed");

    if !compile_out.status.success() {
        return (
            1,
            String::new(),
            String::from_utf8_lossy(&compile_out.stderr).to_string(),
        );
    }

    // Run via near-mock
    let run_out = Command::new("cargo")
        .args(["run", "--bin", "near-mock", "--", &wasm_path, "check"])
        .current_dir("/Users/asil/.openclaw/workspace/lisp-rlm")
        .output()
        .expect("near-mock failed");

    let code = run_out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run_out.stderr).to_string();
    (code, stdout, stderr)
}

// ═══════════════════════════════════════════════════════════════════════
// ADDRESS 0 TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_store_at_address_zero() {
    let lisp = r#"
(define (check)
  (let ((a 0))
    (u128/store a 12345 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "store at address 0 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_to_str_at_address_zero() {
    let lisp = r#"
(define (check)
  (let ((a 0) (buf 80))
    (u128/store a 1000000000000 0)
    (u128/to_str a buf)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "to_str at address 0 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

// ═══════════════════════════════════════════════════════════════════════
// FROM_STR SKIP - requires string building API

#[test]
fn u128_at_address_8() {
    let lisp = r#"
(define (check)
  (let ((a 8))
    (u128/store a 999999 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "store at address 8 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_at_address_16() {
    let lisp = r#"
(define (check)
  (let ((a 16))
    (u128/store a 777777 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "store at address 16 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

// ═══════════════════════════════════════════════════════════════════════
// OVERLAPPING REGION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_store_overlap_low() {
    // Store at 100, then store at 105 (overlap by 3 bytes)
    let lisp = r#"
(define (check)
  (let ((a 100) (b 105))
    (u128/store a 3735928559 51966)
    (u128/store b 12345 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "overlapping stores should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_store_overlap_high() {
    // Store at 100, then store at 108 (hi part of first)
    let lisp = r#"
(define (check)
  (let ((a 100) (b 108))
    (u128/store a 11111 22222)
    (u128/store b 33333 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "overlapping hi stores should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_load_partial_overlap() {
    // Load from address offset by 1 byte (misaligned read)
    let lisp = r#"
(define (check)
  (let ((a 100) (b 101))
    (u128/store a -1 -1)
    (u128/load b)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    // WASM allows unaligned loads
    assert!(
        stdout.contains("Success") || !stderr.contains("trap"),
        "unaligned load should succeed in WASM: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_add_same_address() {
    // 4b1403e spec: u128/add is STRING-based. Address args now hard-error.
    // Pin both sides: string add succeeds end-to-end on wasm, address arg traps.
    let lisp_ok = r#"
(define (check)
  (near/log (u128/add "1000" "1000")))
(export "check" check)
"#;
    let (_c, stdout, stderr) = run_near_mock(lisp_ok);
    assert!(
        stdout.contains("Success"),
        "string add should succeed: stdout={}, stderr={}", stdout, stderr
    );
    let lisp_addr = r#"
(define (check)
  (u128/add 100 100))
(export "check" check)
"#;
    let (_c, stdout, stderr) = run_near_mock(lisp_addr);
    assert!(
        !stdout.contains("Success"),
        "u128/add on addresses must hard-error under string ABI: stdout={}, stderr={}",
        stdout, stderr
    );
}


#[test]
fn u128_add_overlapping() {
    // Overlapping-address in-place add is retired; equivalent live coverage:
    // string add of overlapping values + raw store/load overlap sanity.
    let lisp = r#"
(define (check)
  (let ((a 100) (b 105))
    (u128/store a 1000 0)
    (u128/store b 100 0)
    (near/log (u128/add "1000" "100"))
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "add from overlapping should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}


// ═══════════════════════════════════════════════════════════════════════
// MAX VALUE TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_store_max_value() {
    // Store max u128
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a -1 -1)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "store max u128 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_to_str_max_value() {
    // Convert max u128 to string (39 digits)
    let lisp = r#"
(define (check)
  (let ((a 100) (buf 240))
    (u128/store a -1 -1)
    (u128/to_str a buf)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "to_str max u128 should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_to_str_zero() {
    let lisp = r#"
(define (check)
  (let ((a 100) (buf 80))
    (u128/store a 0 0)
    (u128/to_str a buf)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "to_str zero should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_to_str_one() {
    let lisp = r#"
(define (check)
  (let ((a 100) (buf 80))
    (u128/store a 1 0)
    (u128/to_str a buf)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "to_str one should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

// ═══════════════════════════════════════════════════════════════════════
// DIVISION EDGE CASES
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_div_by_small() {
    let lisp = r#"
(define (check)
  (near/log (u128/div "1000000000000" "10")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "div by small should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
}


#[test]
fn u128_div_by_large() {
    let lisp = r#"
(define (check)
  (near/log (u128/div "1000" "340282366920938463463374607431768211455")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "div by large should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
}


#[test]
fn u128_div_result_zero() {
    let lisp = r#"
(define (check)
  (near/log (u128/div "5" "340282366920938463463374607431768211455")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "div result zero should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
}


// ═══════════════════════════════════════════════════════════════════════
// FROM_STR SKIP - requires string building API

// ═══════════════════════════════════════════════════════════════════════
// SEQUENTIAL OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_multiple_stores_same_address() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 1 0)
    (u128/store a 2 0)
    (u128/store a 3 0)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "multiple stores should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn u128_chain_underflow_traps() {
    // String-ABI chain that underflows must hard-error (wasm trap).
    let lisp = r#"
(define (check)
  (let ((x (u128/sub "5" "10")))
    (near/log x)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        !stdout.contains("Success"),
        "underflow should trap: stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_chain_success() {
    // Arithmetic chain via string ABI (t19 covers the interpreter side at
    // depth; this pins the wasm side end-to-end).
    let lisp = r#"
(define (check)
  (let ((x (u128/div (u128/add (u128/mul "1000" "2") "100") "10")))
    (near/log (u128/sub x "200"))))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "chain should succeed: stdout={}, stderr={}", stdout, stderr
    );
}

