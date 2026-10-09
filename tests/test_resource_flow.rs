//! Resource flow gate (2026-10-08, `check_resource_flow`).
//!
//! Tier 1 (compile-time, both pipelines): arity validation on the promise/
//! transfer family (slot maps transcribed from the emitter arms), literal
//! gas sanity (per-tx 300 Tgas cap; per-defn closure literal sum), raw
//! arithmetic on u128 money values, i64-truncated u128 at money slots,
//! non-decimal literals at money slots, one attached_deposit value attached
//! twice, and storage money stamps (u128 write marks the key; reads seed
//! u128). Model-dependent findings (undefined same-program callback, gas
//! floor vs attached) are WARNINGS, never errors — the gate only rejects
//! what is provably wrong.
//!
//! These tests shell out to the `compile` binary the same way
//! tests/test_promise_single_use.rs does (CARGO_TARGET_DIR else ./target).

use std::process::Command;

fn compile_bin() -> std::path::PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "./target".into());
    std::path::PathBuf::from(dir).join("release").join("compile")
}

/// Returns (exit_ok, stderr) for compiling `src`.
fn compile_src(src: &str, tag: &str) -> (bool, String) {
    let tmp = std::env::temp_dir().join(format!("__res_gate_{}_{}.lisp", tag, std::process::id()));
    std::fs::write(&tmp, src).unwrap();
    let out = Command::new(compile_bin())
        .arg(&tmp)
        .arg("-o")
        .arg(std::env::temp_dir().join(format!("__res_gate_{}.wasm", tag)))
        .output()
        .expect("compile binary missing (cargo build --release --bin compile)");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn arity_violations_are_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near" "extra")))
    (near/promise_return p)))
"#;
    let (ok, err) = compile_src(src, "arity");
    assert!(!ok, "wrong arity must be rejected");
    assert!(
        err.contains("takes 1 argument, got 2"),
        "arity error must name expected and actual counts, got: {err}"
    );
}

#[test]
fn gas_over_tx_cap_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r (near/promise_batch_action_function_call p "go" "{}" "0" 400000000000000)))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "gascap");
    assert!(!ok, "400 Tgas attach must be rejected");
    assert!(
        err.contains("400 Tgas") && err.contains("300 Tgas"),
        "cap error must name the attach and the cap, got: {err}"
    );
}

#[test]
fn raw_arithmetic_on_u128_is_rejected() {
    let src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128))
         (s (u128/add d "100")))
    (let* ((bad (+ d s)))
      (near/log "done"))))
"#;
    let (ok, err) = compile_src(src, "arith");
    assert!(!ok, "money-op-money must be rejected");
    assert!(
        err.contains("raw arithmetic on a u128 money value") && err.contains("u128/add"),
        "error must give the u128-family fix, got: {err}"
    );
}

#[test]
fn i64_truncated_u128_at_money_slot_is_rejected() {
    let src = r#"(define (main)
  (near/transfer_u128 "recv.test.near" (u128/to-i64 (near/attached_deposit_u128))))
"#;
    let (ok, err) = compile_src(src, "trunc");
    assert!(!ok, "i64-truncated u128 at a money slot must be rejected");
    assert!(
        err.contains("truncated to i64"),
        "error must name the truncation, got: {err}"
    );
}

#[test]
fn non_decimal_literal_at_money_slot_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r (near/promise_batch_action_transfer p "not-a-number")))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "badlit");
    assert!(!ok, "non-decimal money literal must be rejected");
    assert!(
        err.contains("not a decimal string"),
        "error must name the literal, got: {err}"
    );
}

#[test]
fn one_deposit_attached_twice_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near"))
         (q (near/promise_batch_create "b.test.near"))
         (d (near/attached_deposit)))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" d 10000000000000))
           (r2 (near/promise_batch_action_function_call q "go" "{}" d 10000000000000)))
      (near/promise_return r1))))
"#;
    let (ok, err) = compile_src(src, "dep2x");
    assert!(!ok, "one deposit attached twice must be rejected");
    assert!(
        err.contains("attached to two promise operations"),
        "error must name the double attach, got: {err}"
    );
}

#[test]
fn storage_money_stamps_flow() {
    // Write a u128 value to a literal key, read it back, do raw arithmetic
    // on the read → reject (the stamp flows through the read).
    let src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128)))
    (let* ((_w (near/storage_write "ledger:balance" d)))
      (let* ((bal (near/storage_read "ledger:balance")))
        (let* ((bad (+ bal "1")))
          (near/log "done"))))))
"#;
    let (ok, err) = compile_src(src, "stamp");
    assert!(!ok, "stamped-key read feeding raw arithmetic must be rejected");
    assert!(
        err.contains("raw arithmetic on a u128 money value"),
        "got: {err}"
    );

    // The same read feeding a money sink directly is legal.
    let ok_src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128)))
    (let* ((_w (near/storage_write "ledger:balance" d)))
      (let* ((bal (near/storage_read "ledger:balance")))
        (let* ((t (near/transfer_u128 "recv.test.near" bal)))
          (near/log "paid"))))))
"#;
    let (ok2, err2) = compile_src(ok_src, "stampok");
    assert!(ok2, "stamped-key read at a money sink must compile, got: {err2}");
}

#[test]
fn legal_resource_program_compiles_and_warns_on_dead_callback() {
    // Legal: u128 family on the string domain, decimal amounts, bounded
    // gas, deposit attached once, in-program callback.
    let src = r#"(define (cb) (near/log "ticked"))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near"))
         (d (near/attached_deposit_u128))
         (s (u128/add d "100")))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" "1000000000000000000" 10000000000000))
           (r2 (near/promise_batch_action_transfer p "5")))
      (let* ((t (near/promise_then r1 (near/current_account_id) "cb" "{}" "0" 10000000000000)))
        (if (u128/lt s "1000") (near/promise_return t) (near/promise_return r1))))))
"#;
    let (ok, err) = compile_src(src, "legal");
    assert!(ok, "legal resource program must compile, got: {err}");

    // Undefined same-program callback: compiles, but MUST warn (the
    // dead-receipt class the gate surfaces without breaking compiles).
    let src2 = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((t (near/promise_then p (near/current_account_id) "never_defined" "{}" "0" 10000000000000)))
      (near/promise_return t))))
"#;
    let (ok2, err2) = compile_src(src2, "deadc");
    assert!(ok2, "undefined callback must NOT be a hard error (cross-contract ambiguity), got: {err2}");
    assert!(
        err2.contains("warning:") && err2.contains("never_defined") && err2.contains("MethodNotFound"),
        "undefined callback must produce the dead-receipt warning, got: {err2}"
    );
}
