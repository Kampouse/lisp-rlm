//! Promise single-use gate (2026-10-08).
//!
//! Tier 1 (compile-time): a promise handle consumed twice inside one defn is
//! a hard error naming both consuming lines (`src/typing/checker.rs`).
//! Tier 2 (runtime): a second consuming host op on a live handle traps in
//! near-mock (`src/bin/near_mock/mod.rs`, `CONSUMED_PROMISES`).
//!
//! These tests shell out to the `compile` binary the same way
//! tests/test_compile_cli.rs resolves it (CARGO_TARGET_DIR else ./target,
//! release profile — run the suite with `cargo test --release`).

use std::process::Command;

fn compile_bin() -> std::path::PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "./target".into());
    std::path::PathBuf::from(dir)
        .join("release")
        .join("compile")
}

fn compile_src(src: &str, tag: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!(
        "__promise_gate_{}_{}.lisp",
        tag,
        std::process::id()
    ));
    std::fs::write(&tmp, src).unwrap();
    let out = Command::new(compile_bin())
        .arg(&tmp)
        .arg("-o")
        .arg(std::env::temp_dir().join(format!("__promise_gate_{}.wasm", tag)))
        .output()
        .expect("compile binary missing (cargo build --release --bin compile)");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

#[test]
fn double_consume_in_one_defn_is_rejected_naming_both_lines() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "recv.test.near")))
    (let* ((a (near/promise_then p (near/current_account_id) "cb1" "{}" "0" 10000000000000)))
      (near/promise_then p (near/current_account_id) "cb2" "{}" "0" 10000000000000))))
"#;
    let err = compile_src(src, "double").expect_err("double promise_then must be rejected");
    assert!(
        err.contains("consumed 2 times"),
        "error must name the double consumption, got: {err}"
    );
    assert_eq!(
        err.matches("promise_then (line").count(),
        2,
        "both consuming lines must be named, got: {err}"
    );
    assert!(
        err.contains("single-use"),
        "error must state the single-use contract, got: {err}"
    );
}

#[test]
fn single_use_promise_dance_compiles() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "recv.test.near")))
    (let* ((p2 (near/promise_then p (near/current_account_id) "cb" "{}" "0" 10000000000000)))
      (near/promise_return p2))))
"#;
    compile_src(src, "single").expect("single-use promise chain must compile");
}
