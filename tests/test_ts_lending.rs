//! Lending-protocol lifecycle (TS surface, 2026-08-31).
//!
//! deposit → borrow-reject (LTV) → borrow-ok (fee math) → repay →
//! withdraw-ok, executed across separate near-mock invocations sharing the
//! persistent state file. One #[test] for the whole sequence — the mock's
//! STATE_FILE is global, so parallel lending tests would race.

use std::process::Command;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

const SRC: &str = include_str!("../fixtures/lending.ts");

fn run(method: &str, input: &str) -> String {
    let ir = ts_to_lisp_source(SRC)
        .unwrap_or_else(|e| panic!("ts lowering failed: {}", e));
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs)
        .unwrap_or_else(|e| panic!("compile failed: {}", e));
    let tmp = std::env::temp_dir().join(format!(
        "nm_lending_{}.wasm",
        std::process::id()
    ));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .output()
        .expect("near-mock should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn lending_lifecycle() {
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin"); // fresh state

    // deposit 1000
    let out = run("deposit", r#"{"amt":"1000"}"#);
    assert!(out.contains(r#"📄 {"dep":1000,"bor":0}"#), "deposit: {out}");

    // borrow 500 → debt 525; 1000*5000 = 5M < 525*10000 = 5.25M → ABORT
    let out = run("borrow", r#"{"amt":"500"}"#);
    assert!(
        out.contains("insufficient collateral") && out.contains("ABORT"),
        "over-LTV borrow must abort with reason: {out}"
    );

    // borrow 400 → debt 420; 5M >= 4.2M → ok
    let out = run("borrow", r#"{"amt":"400"}"#);
    assert!(out.contains(r#"📄 {"dep":1000,"bor":420}"#), "borrow: {out}");

    // repay 420 → debt 0
    let out = run("repay", r#"{"amt":"420"}"#);
    assert!(out.contains(r#"📄 {"dep":1000,"bor":0}"#), "repay: {out}");

    // withdraw 800 → dep 200 (no borrow left, allowed)
    let out = run("withdraw", r#"{"amt":"800"}"#);
    assert!(out.contains(r#"📄 {"dep":200,"bor":0}"#), "withdraw: {out}");
}
