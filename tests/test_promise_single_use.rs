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

// ---------------------------------------------------------------------------
// Cross-defn flow (2026-10-08): per-defn consumption summaries + call edges.
// The x10 class — a promise passed to two defns that each consume it once —
// is now a COMPILE error, not just a mock runtime trap.
// ---------------------------------------------------------------------------

#[test]
fn cross_defn_double_consume_is_rejected_naming_both_defns() {
    let src = r#"(define (use1 p)
  (near/promise_then p (near/current_account_id) "cb1" "{}" "0" 10000000000000))
(define (use2 p)
  (near/promise_then p (near/current_account_id) "cb2" "{}" "0" 10000000000000))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r1 (use1 p)))
      (use2 p))))
"#;
    let err = compile_src(src, "cross").expect_err("cross-defn double consume must be rejected");
    assert!(
        err.contains("consumed 2 times across calls"),
        "cross-defn rejection must use the across-calls message, got: {err}"
    );
    assert!(
        err.contains("in use1") && err.contains("in use2"),
        "both consuming defns must be named, got: {err}"
    );
    assert!(
        err.contains("single-use"),
        "error must state the single-use contract, got: {err}"
    );
}

#[test]
fn sibling_defns_same_binding_name_compiles() {
    // Regression guard: the pre-flow lint keyed uses by binding NAME across
    // the whole program, so two sibling defns each consuming their OWN `p`
    // once were falsely rejected. Keys are now (defn, binding).
    let src = r#"(define (a)
  (let* ((p (near/promise_batch_create "x.test.near")))
    (near/promise_then p (near/current_account_id) "cb" "{}" "0" 10000000000000)))
(define (b)
  (let* ((p (near/promise_batch_create "y.test.near")))
    (near/promise_then p (near/current_account_id) "cb" "{}" "0" 10000000000000)))
(define (main)
  (let* ((r1 (a)))
    (b)))
"#;
    compile_src(src, "sibling")
        .expect("sibling defns with same binding name must NOT be flagged");
}

#[test]
fn param_double_consume_is_rejected() {
    // A defn consuming its own param twice: every caller passing a promise
    // loses both callbacks. Bucket 2 (caller's local flows in) and bucket 3
    // (param's own total) both fire; either rejection satisfies the gate.
    let src = r#"(define (use2 p)
  (let* ((r1 (near/promise_then p (near/current_account_id) "cb1" "{}" "0" 10000000000000)))
    (near/promise_then p (near/current_account_id) "cb2" "{}" "0" 10000000000000)))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (use2 p)))
"#;
    let err = compile_src(src, "paramdouble").expect_err("param double consume must be rejected");
    assert!(
        err.contains("consumed 2 times"),
        "rejection must state the count, got: {err}"
    );
}

#[test]
fn two_hop_chain_rejected_and_self_recursion_terminates() {
    // Flow through an intermediate defn must reach the leaf's consume (2
    // total), and a self-recursive forwarder must terminate the analysis
    // without a false positive (cycle guard; per-iteration re-consume stays
    // the runtime trap's job).
    let chain = r#"(define (leaf p)
  (near/promise_then p (near/current_account_id) "cb" "{}" "0" 10000000000000))
(define (mid p)
  (leaf p))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r1 (mid p)))
      (leaf p))))
"#;
    let err = compile_src(chain, "chain").expect_err("two-hop flow must be rejected");
    assert!(
        err.contains("consumed 2 times") && err.contains("in leaf"),
        "chain rejection must name the leaf site, got: {err}"
    );

    let rec = r#"(define (f p n)
  (near/promise_then p (near/current_account_id) "cb" "{}" "0" 10000000000000))
(define (g p)
  (let* ((r (f p 1)))
    (g p)))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (g p)))
"#;
    compile_src(rec, "selfrec")
        .expect("self-recursive forwarder must terminate and compile");
}
