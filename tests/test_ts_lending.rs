//! Lending v3 — u128 + time-based interest (2026-08-31).
//!
//! Deterministic via NEAR_MOCK_BLOCK_TS pinning: deposit/borrow at t0,
//! health at t0+100d shows exactly the predicted accrued debt
//! (10% APY, floor division, lazy accrual on every entry).

use std::process::Command;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/lending.ts");

fn run(method: &str, input: &str, ts_ns: &str) -> String {
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {}", e));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {}", e));
    let tmp = std::env::temp_dir().join(format!("nm_l3_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = Command::new("./target/release/near-mock")
        .env("NEAR_MOCK_BLOCK_TS", ts_ns)
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .output()
        .expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

const TS0: &str = "1800000000000000000";
// TS0 + 100*86400*1e9 = 1_808_640_000_000_000_000 (machine-derived —
// a hand-typed transposition here cost 20min of phantom-bug hunting)
const T100D: &str = "1808640000000000000";

#[test]
fn lending_interest_accrues_exact() {
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");

    // deposit 10 NEAR at t0
    let out = run("deposit", r#"{"amt":"10000000000000000000000000"}"#, TS0);
    assert!(out.contains(r#""dep":10000000000000000000000000"#), "{out}");

    // borrow 4 NEAR → debt 4.2e24 (5% fee), clock stamped t0
    let out = run("borrow", r#"{"amt":"4000000000000000000000000"}"#, TS0);
    assert!(out.contains(r#""bor":4200000000000000000000000"#), "{out}");

    // t0: health = 10e24*5000/4.2e24 = 11904 bp
    let out = run("health", "{}", TS0);
    assert!(out.contains("📄 11904"), "{out}");

    // +100 days: interest = 4.2e24*1000*8640000/(10000*31536000)
    //            = 115068493150684931506849 (floor) → debt 4315068493150684931506849
    // health = 10e24*5000/4315068493150684931506849 = 11587
    let out = run("health", "{}", T100D);
    assert!(out.contains("📄 11587"), "interest must accrue exactly: {out}");

    // repay 0 at +100d returns the accrued debt (pure query effect)
    let out = run("repay", r#"{"amt":"0"}"#, T100D);
    assert!(out.contains(r#""bor":4315068493150684931506849"#), "{out}");

    // over-LTV borrow at t0 again still aborts with reason
    let out = run("borrow", r#"{"amt":"5000000000000000000000000"}"#, T100D);
    assert!(out.contains("insufficient collateral"), "{out}");
}
