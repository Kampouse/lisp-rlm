//! Test u128 money-safe arithmetic: overflow/underflow traps.
//!
//! Interpreter tests + WASM trap verification via near-mock.

use lisp_rlm_wasm::{parse_all, Env, EvalState};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_ID: AtomicU64 = AtomicU64::new(0);

// ═══════════════════════════════════════════════════════════════════════
// INTERPRETER TESTS (verify basic semantics)
// ═══════════════════════════════════════════════════════════════════════

fn eval(code: &str) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = String::new();
    for expr in &exprs {
        let val = lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state)?;
        result = val.to_string();
    }
    Ok(result)
}

#[test]
fn u128_add_normal_interp() {
    let code = "(let ((a 100) (b 200)) (u128/store a 1000 0) (u128/store b 200 0) (u128/add a b) (u128/load a))";
    assert!(eval(code).is_ok(), "normal add should succeed");
}

#[test]
fn u128_sub_normal_interp() {
    let code = "(let ((a 100) (b 200)) (u128/store a 1000 0) (u128/store b 200 0) (u128/sub a b) (u128/load a))";
    assert!(eval(code).is_ok(), "normal sub should succeed");
}

#[test]
fn u128_mul_normal_interp() {
    let code = "(let ((a 100)) (u128/store a 100 0) (u128/mul a 50) (u128/load a))";
    assert!(eval(code).is_ok(), "normal mul should succeed");
}

// ═══════════════════════════════════════════════════════════════════════
// WASM TRAP TESTS (via near-mock binary)
// ═══════════════════════════════════════════════════════════════════════

/// Compile and run via near-mock. Returns (exit_code, stdout, stderr).
fn run_near_mock(lisp: &str) -> (i32, String, String) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    
    // Use unique temp file per test to avoid parallel race
    let id = TEST_ID.fetch_add(1, Ordering::SeqCst);
    let tmp_path = format!("/tmp/u128_test_{}.lisp", id);
    let wasm_path = format!("/tmp/u128_test_{}.wasm", id);
    std::fs::write(&tmp_path, lisp).unwrap();
    
    // Compile
    let compile_out = Command::new("cargo")
        .args(["run", "--bin", "near-compile", "--", &tmp_path, &wasm_path])
        .output()
        .expect("near-compile failed");
    
    if !compile_out.status.success() {
        return (1, String::new(), String::from_utf8_lossy(&compile_out.stderr).to_string());
    }
    
    // Run via near-mock (pass method name "check")
    let run_out = Command::new("cargo")
        .args(["run", "--bin", "near-mock", "--", &wasm_path, "check"])
        .output()
        .expect("near-mock failed");
    
    let code = run_out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run_out.stderr).to_string();
    (code, stdout, stderr)
}

#[test]
fn u128_add_overflow_traps_wasm() {
    // max u128 + 1 should trap
    let lisp = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a -1 -1) ;; max u128
    (u128/store b 1 0)
    (u128/add a b)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    // Should fail with trap message
    assert!(stderr.contains("trap") || stderr.contains("error") || stderr.contains("❌"),
        "overflow should trap: stderr={}", stderr);
}

#[test]
fn u128_sub_underflow_traps_wasm() {
    // 100 - 200 = underflow
    let lisp = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a 100 0)
    (u128/store b 200 0)
    (u128/sub a b)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stderr.contains("trap") || stderr.contains("error") || stderr.contains("❌"),
        "underflow should trap: stderr={}", stderr);
}

#[test]
fn u128_mul_by_zero_traps_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 1000 0)
    (u128/mul a 0)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stderr.contains("trap") || stderr.contains("error") || stderr.contains("❌"),
        "mul by zero should trap: stderr={}", stderr);
}

#[test]
fn u128_mul_by_negative_traps_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 1000 0)
    (u128/mul a -5)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stderr.contains("trap") || stderr.contains("error") || stderr.contains("❌"),
        "mul by negative should trap: stderr={}", stderr);
}

#[test]
fn u128_checked_to_i64_overflow_traps_wasm() {
    // Value too large for i64 should trap
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 100 1) ;; hi != 0, value > 2^64
    (u128/checked_to_i64 a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stderr.contains("trap") || stderr.contains("error") || stderr.contains("❌"),
        "checked_to_i64 overflow should trap: stderr={}", stderr);
}

#[test]
fn u128_add_normal_wasm() {
    // 1000 + 200 = 1200 (should succeed)
    let lisp = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a 1000 0)
    (u128/store b 200 0)
    (u128/add a b)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    // Should succeed (exit 0, "Success" in stdout)
    assert!(stdout.contains("Success"), "normal add should succeed: stdout={}, stderr={}", stdout, stderr);
}

#[test]
fn u128_sub_normal_wasm() {
    // 1000 - 200 = 800 (should succeed)
    let lisp = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a 1000 0)
    (u128/store b 200 0)
    (u128/sub a b)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stdout.contains("Success"), "normal sub should succeed: stdout={}, stderr={}", stdout, stderr);
}

#[test]
fn u128_mul_normal_wasm() {
    // 100 * 50 = 5000 (should succeed)
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 100 0)
    (u128/mul a 50)
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stdout.contains("Success"), "normal mul should succeed: stdout={}, stderr={}", stdout, stderr);
}

#[test]
fn u128_fit_i64_small_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 42 0)
    (u128/fit_i64 a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stdout.contains("Success"), "fit_i64 small should succeed: stdout={}, stderr={}", stdout, stderr);
}

#[test]
fn u128_checked_to_i64_small_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 12345 0)
    (u128/checked_to_i64 a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(stdout.contains("Success"), "checked_to_i64 small should succeed: stdout={}, stderr={}", stdout, stderr);
}
