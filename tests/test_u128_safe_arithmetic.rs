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
    // String ABI (4b1403e) + address store/load memory roundtrip
    let code = "(let ((a 100) (b 200)) (u128/store a 1000 0) (u128/store b 200 0) (u128/add \"1000\" \"200\") (u128/load a))";
    assert!(eval(code).is_ok(), "normal add should succeed");
}


#[test]
fn u128_sub_normal_interp() {
    let code = "(let ((a 100) (b 200)) (u128/store a 1000 0) (u128/store b 200 0) (u128/sub \"1000\" \"200\") (u128/load a))";
    assert!(eval(code).is_ok(), "normal sub should succeed");
}


#[test]
fn u128_mul_normal_interp() {
    let code = "(let ((a 100)) (u128/store a 100 0) (u128/mul \"100\" \"50\") (u128/load a))";
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
        return (
            1,
            String::new(),
            String::from_utf8_lossy(&compile_out.stderr).to_string(),
        );
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
    // max u128 + 1 must hard-error (string ABI)
    let lisp = r#"
(define (check)
  (near/log (u128/add "340282366920938463463374607431768211455" "1")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        !stdout.contains("Success"),
        "overflow must trap: stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_sub_underflow_traps_wasm() {
    let lisp = r#"
(define (check)
  (near/log (u128/sub "5" "10")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        !stdout.contains("Success"),
        "underflow must trap: stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_mul_by_zero_traps_wasm() {
    // Under the string ABI mul by zero is LEGAL (result "0") — pin the
    // semantic: the old address-ABI trap premise is retired (4b1403e).
    let lisp = r#"
(define (check)
  (near/log (u128/mul "1000" "0")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "mul by zero should succeed (result 0): stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_mul_by_negative_traps_wasm() {
    // "-5" is not a u128 decimal — hard parse error
    let lisp = r#"
(define (check)
  (near/log (u128/mul "1000" "-5")))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        !stdout.contains("Success"),
        "mul by negative must trap: stdout={}, stderr={}", stdout, stderr
    );
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
    assert!(
        !stdout.contains("Success"),
        "checked_to_i64 overflow should trap: stdout={}, stderr={}",
        stdout, stderr
    );
}

#[test]
fn u128_add_normal_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a 1000 0)
    (u128/store b 200 0)
    (near/log (u128/add "1000" "200"))
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "normal add should succeed: stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_sub_normal_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 1000 0)
    (near/log (u128/sub "1000" "200"))
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "normal sub should succeed: stdout={}, stderr={}", stdout, stderr
    );
}


#[test]
fn u128_mul_normal_wasm() {
    let lisp = r#"
(define (check)
  (let ((a 100))
    (u128/store a 100 0)
    (near/log (u128/mul "100" "50"))
    (u128/load a)))
(export "check" check)
"#;
    let (_code, stdout, stderr) = run_near_mock(lisp);
    assert!(
        stdout.contains("Success"),
        "normal mul should succeed: stdout={}, stderr={}", stdout, stderr
    );
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
    assert!(
        stdout.contains("Success"),
        "fit_i64 small should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
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
    assert!(
        stdout.contains("Success"),
        "checked_to_i64 small should succeed: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

// ═══════════════════════════════════════════════════════════════════════
// ADDRESS FAMILY (interpreter runtime — scratch linear memory, mirrors wasm)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn u128_addr_store_load_roundtrip_interp() {
    let code = "(let ((a 100)) (u128/store a 42 0) (u128/load a))";
    let r = eval(code);
    assert!(r.is_ok(), "store/load roundtrip: {:?}", r.err());
    assert!(r.unwrap().contains("42"), "load must return 42");
}

#[test]
fn u128_addr_store_load_high_interp() {
    let code = "(let ((a 100)) (u128/store a 42 7) (u128/load_high a))";
    let r = eval(code);
    assert!(r.is_ok());
    assert!(r.unwrap().contains("7"), "load_high must return 7");
}

#[test]
fn u128_addr_to_str_interp() {
    let code = "(let ((a 100) (buf 200)) (u128/store a 1000000000000 0) (u128/to_str a buf))";
    let r = eval(code);
    assert!(r.is_ok());
    assert!(r.unwrap().contains("1000000000000"), "to_str must render decimal");
}

#[test]
fn u128_addr_fit_and_checked_interp() {
    assert!(eval("(let ((a 100)) (u128/store a 42 0) (u128/fit_i64 a))")
        .unwrap().contains("1"), "42 fits i64");
    assert!(eval("(let ((a 100)) (u128/store a 42 1) (u128/fit_i64 a))")
        .unwrap().contains("0"), "2^64+42 does not fit");
    assert!(eval("(let ((a 100)) (u128/store a 42 0) (u128/checked_to_i64 a))").is_ok(),
        "checked_to_i64 fits path ok");
    assert!(eval("(let ((a 100)) (u128/store a 42 1) (u128/checked_to_i64 a))").is_err(),
        "checked_to_i64 overflow must hard-error");
}

#[test]
fn u128_addr_new_from_i64_from_yocto_interp() {
    let r = eval("(let ((b 200)) (u128/new 7 8 b) (+ (u128/load b) (* (u128/load_high b) 1000)))");
    assert!(r.is_ok());
    assert!(r.unwrap().contains("7008"), "new stores (hi=7, lo=8)");
    let r = eval("(let ((a 100)) (u128/from_i64 5 a) (u128/load a))");
    assert!(r.unwrap().contains("5"), "from_i64 stores 5 low");
    let r = eval("(let ((a 100)) (u128/from_yocto \"18446744073709551616\" a) (u128/load_high a))");
    assert!(r.unwrap().contains("1"), "from_yocto 2^64 → hi=1");
}

#[test]
fn u128_addr_out_of_bounds_interp() {
    // 4 MiB cap (64 wasm pages) — store at the edge must hard-error
    assert!(eval("(u128/store 4194304 1 0)").is_err(), "OOB store must err");
    assert!(eval("(u128/store 4194288 1 0)").is_ok(), "last in-bounds 16B window at cap-16");
    assert!(eval("(u128/load 4194296)").is_err(), "window crossing cap must err");
}
