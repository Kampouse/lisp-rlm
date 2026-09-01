//! Cross-contract engine — token + vault via await near.call (2026-09-01).
//!
//! Exercises: near-mock `cross` mode (multi-contract manifest, per-account
//! storage partitions, synchronous promise DAG resolution, failed-receipt
//! partition revert, sub-call signer = promise creator), the TS async V1
//! lowering (entry param save → call-await → continuation restore), and the
//! NEP-141-style allowance pattern where the VAULT CONTRACT is the spender.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const TOKEN_SRC: &str = include_str!("../fixtures/token_allowance.ts");
const VAULT_SRC: &str = include_str!("../fixtures/vault_cross.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn wasm_path(src: &str, tag: &str) -> String {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("lower: {e}"));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let w = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {e}"));
    let p = std::env::temp_dir().join(format!("cc_{}_{}.wasm", tag, std::process::id()));
    std::fs::write(&p, &w).unwrap();
    p.to_str().unwrap().into()
}

fn run(acct: &str, method: &str, args: &str, signer: &str) -> String {
    let token = wasm_path(TOKEN_SRC, "tok");
    let vault = wasm_path(VAULT_SRC, "vlt");
    let manifest = format!("token.cc.test.near={},vault.cc.test.near={}", token, vault);
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross").arg("/tmp/cc-test-state.bin").arg(&manifest).arg(acct).arg(method).arg(args)
        .env("NEAR_MOCK_SIGNER", signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn cross_contract_deposit_and_fail_closed() {
    let _l = lock();
    let _ = std::fs::remove_file("/tmp/cc-test-state.bin");
    let u: u128 = 10u128.pow(18);

    // setup on the token
    assert!(run("token.cc.test.near", "ftMint", &format!(r#"{{"to":"alice.test.near","amount":"{}"}}"#, 1_000_000*u), "alice.test.near").contains("supply:"));
    assert!(run("token.cc.test.near", "ftIncreaseAllowance", &format!(r#"{{"spender":"vault.cc.test.near","amount":"{}"}}"#, 500_000*u), "alice.test.near").contains("allowance:"));

    // THE cross-call: vault.deposit → promise → token.ftTransferFrom → callback
    let out = run("vault.cc.test.near", "deposit", &format!(r#"{{"user":"alice.test.near","amount":"{}"}}"#, 300_000*u), "alice.test.near");
    assert!(out.contains(&format!("deposited:{}", 300_000*u)), "deposit failed: {out}");

    // state moved across BOTH contracts
    assert!(run("token.cc.test.near", "ftBalanceOf", r#"{"who":"alice.test.near"}"#, "alice.test.near").contains(&format!("bal:{}", 700_000*u)));
    assert!(run("token.cc.test.near", "ftBalanceOf", r#"{"who":"vault.cc.test.near"}"#, "alice.test.near").contains(&format!("bal:{}", 300_000*u)));
    assert!(run("vault.cc.test.near", "getTotalDeposits", r#"{"who":"alice.test.near"}"#, "alice.test.near").contains(&format!("deposits:{}", 300_000*u)));

    // fail-closed: 300K > 200K remaining allowance → sub-call traps → revert → vault aborts
    let out = run("vault.cc.test.near", "deposit", &format!(r#"{{"user":"alice.test.near","amount":"{}"}}"#, 300_000*u), "alice.test.near");
    assert!(out.contains("token transfer failed"), "should fail closed: {out}");
    // NO partial state: balances and deposits unchanged by the failed deposit
    assert!(run("token.cc.test.near", "ftBalanceOf", r#"{"who":"alice.test.near"}"#, "alice.test.near").contains(&format!("bal:{}", 700_000*u)));
    assert!(run("vault.cc.test.near", "getTotalDeposits", r#"{"who":"alice.test.near"}"#, "alice.test.near").contains(&format!("deposits:{}", 300_000*u)));

    // a second in-allowance deposit works and both ledgers agree
    let out = run("vault.cc.test.near", "deposit", &format!(r#"{{"user":"alice.test.near","amount":"{}"}}"#, 150_000*u), "alice.test.near");
    assert!(out.contains(&format!("deposited:{}", 450_000*u)), "{out}");
    assert!(run("token.cc.test.near", "ftBalanceOf", r#"{"who":"vault.cc.test.near"}"#, "alice.test.near").contains(&format!("bal:{}", 450_000*u)));
}
