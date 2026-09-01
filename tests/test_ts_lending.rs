//! Lending-protocol lifecycle — u128 precision (TS surface, 2026-08-31).
//!
//! deposit → borrow-reject (LTV) → borrow-ok (fee math, ceiled) → repay →
//! withdraw, in YOCTO units (1 NEAR = 10^24) across separate near-mock
//! invocations sharing the persistent state file. Amounts deliberately
//! exceed i64 (~9.2e18) — the whole point of the u128 string ABI.

use std::process::Command;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, typing, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/lending.ts");

fn run(method: &str, input: &str) -> String {
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("ts lowering failed: {}", e));
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile failed: {}", e));
    let tmp = std::env::temp_dir().join(format!("nm_lending_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .output()
        .expect("near-mock should run");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn lending_lifecycle_u128() {
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin"); // fresh state

    // deposit 5.25 NEAR = 5.25e24 yocto (25 digits — over i64)
    let out = run("deposit", r#"{"amt":"5250000000000000000000000"}"#);
    assert!(out.contains(r#"📄 {"dep":5250000000000000000000000,"bor":"0"}"#), "deposit: {out}");

    // borrow 3 NEAR: debt 3.15e24, cover only 2.625e24 → ABORT with reason
    let out = run("borrow", r#"{"amt":"3000000000000000000000000"}"#);
    assert!(
        out.contains("insufficient collateral") && out.contains("ABORT"),
        "over-LTV borrow must abort with reason: {out}"
    );

    // borrow 2 NEAR: debt = 2e24*10500/10000 = 2.1e24 exactly (fee ceiled)
    let out = run("borrow", r#"{"amt":"2000000000000000000000000"}"#);
    assert!(out.contains(r#"📄 {"dep":5250000000000000000000000,"bor":2100000000000000000000000}"#), "borrow: {out}");

    // health = dep*LTV/bor = 5.25e24*5000/2.1e24 = 12500 (bp)
    let out = run("health", "{}");
    assert!(out.contains("📄 12500"), "health: {out}");

    // repay the full 2.1 NEAR debt
    let out = run("repay", r#"{"amt":"2100000000000000000000000"}"#);
    assert!(out.contains(r#""bor":0"#), "repay: {out}");

    // withdraw everything
    let out = run("withdraw", r#"{"amt":"5250000000000000000000000"}"#);
    assert!(out.contains(r#"📄 {"dep":0,"bor":0}"#), "withdraw: {out}");
}
