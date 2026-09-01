//! Fungible Token (NEP-141 subset) — TS surface (2026-09-01).
//!
//! Exercises what lending never did: raw u128-string balances under
//! plain keys (no JSON), two-key double-writes in one call (transfer),
//! param-keyed reads (balanceOf anyone), and mixed `"prefix:" + bigint`
//! returns (the str-cat/u128 coercion bug lived there).

use std::process::Command;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/ft.ts");

use std::sync::{Mutex, OnceLock};

fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let m = LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}



fn run(method: &str, input: &str, signer: Option<&str>) -> String {
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {}", e));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {}", e));
    let tmp = std::env::temp_dir().join(format!("nm_ft_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let mut cmd = Command::new("./target/release/near-mock");
    if let Some(s) = signer { cmd.env("NEAR_MOCK_SIGNER", s); }
    let out = cmd.arg(&tmp).arg(method).arg(input).output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn ft_full_lifecycle() {
    let _lock = state_lock();
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");

    // first minter becomes owner; mints 2.5M (24-digit, 18dp-style)
    let out = run("ftMint", r#"{"to":"owner.test.near","amount":"2500000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("supply:2500000000000000000000000"), "{out}");

    // non-owner cannot mint
    let out = run("ftMint", r#"{"to":"alice.test.near","amount":"1000000000000000000000000"}"#, Some("alice.test.near"));
    assert!(out.contains("only the owner may mint"), "{out}");

    // transfer 1M owner→bob; double-write must land both keys
    let out = run("ftTransfer", r#"{"to":"bob.test.near","amount":"1000000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"bob.test.near"}"#, None);
    assert!(out.contains("1000000000000000000000000"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"owner.test.near"}"#, None);
    assert!(out.contains("1500000000000000000000000"), "{out}");

    // over-transfer by ONE yocto refused (u128/lt boundary)
    let out = run("ftTransfer", r#"{"to":"alice.test.near","amount":"1000000000000000000000001"}"#, Some("bob.test.near"));
    assert!(out.contains("insufficient balance"), "{out}");

    // exact transfer passes; supply unchanged by transfers
    let out = run("ftTransfer", r#"{"to":"alice.test.near","amount":"1000000000000000000000000"}"#, Some("bob.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftBurn", r#"{"amount":"0"}"#, Some("owner.test.near"));
    assert!(out.contains("supply:2500000000000000000000000"), "{out}");

    // burn reduces both balance and supply
    let out = run("ftBurn", r#"{"amount":"500000000000000000000000"}"#, Some("alice.test.near"));
    assert!(out.contains("supply:2000000000000000000000000"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"alice.test.near"}"#, None);
    assert!(out.contains("500000000000000000000000"), "{out}");
}

#[test]
fn ft_allowances_nep141() {
    let _lock = state_lock();
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");

    // fresh deployment: mint 2.5M to owner
    let out = run("ftMint", r#"{"to":"owner.test.near","amount":"2500000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("supply:2500000000000000000000000"), "{out}");

    // allowance starts at zero (missing key → default)
    let out = run("ftAllowance", r#"{"owner":"owner.test.near","spender":"bob.test.near"}"#, None);
    assert!(out.contains("📄 0"), "{out}");

    // approve 500K, spend 400K via transferFrom (three keys written)
    let out = run("ftApprove", r#"{"spender":"bob.test.near","amount":"500000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftTransferFrom", r#"{"from":"owner.test.near","to":"alice.test.near","amount":"400000000000000000000000"}"#, Some("bob.test.near"));
    assert!(out.contains("ok"), "{out}");

    let out = run("ftAllowance", r#"{"owner":"owner.test.near","spender":"bob.test.near"}"#, None);
    assert!(out.contains("100000000000000000000000"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"owner.test.near"}"#, None);
    assert!(out.contains("2100000000000000000000000"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"alice.test.near"}"#, None);
    assert!(out.contains("400000000000000000000000"), "{out}");

    // +1-yocto over-allowance refused (|| guard covers allowance AND balance)
    let out = run("ftTransferFrom", r#"{"from":"owner.test.near","to":"bob.test.near","amount":"100000000000000000000001"}"#, Some("bob.test.near"));
    assert!(out.contains("allowance or balance too low"), "{out}");

    // NEP-141 race rule: nonzero → nonzero must abort
    let out = run("ftApprove", r#"{"spender":"bob.test.near","amount":"1000000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("reset allowance to zero first"), "{out}");

    // reset to zero, then re-approve 1M and spend it
    let out = run("ftApprove", r#"{"spender":"bob.test.near","amount":"0"}"#, Some("owner.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftApprove", r#"{"spender":"bob.test.near","amount":"1000000000000000000000000"}"#, Some("owner.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftTransferFrom", r#"{"from":"owner.test.near","to":"carol.test.near","amount":"1000000000000000000000000"}"#, Some("bob.test.near"));
    assert!(out.contains("ok"), "{out}");
    let out = run("ftBalanceOf", r#"{"who":"carol.test.near"}"#, None);
    assert!(out.contains("1000000000000000000000000"), "{out}");
}

#[test]
fn bigint_plus_string_literal_is_concat() {
    // "supply:" + (supply + amount) must lower to str-cat, not u128/add
    // (u128/add would trap parsing "supply:"). Found via the FT contract.
    let ts = r#"
        export function t(supply: bigint, amount: bigint): string {
          return "supply:" + (supply + amount);
        }
    "#;
    let ir = ts_to_lisp_source(ts).expect("lowering");
    assert!(ir.contains(r#"(str-cat "supply:" (u128/add supply amount))"#), "IR: {ir}");
    // numeric literals still mean arithmetic, not concat
    let ts2 = r#"
        export function t2(a: bigint): bigint {
          return a + 5n;
        }
    "#;
    let ir2 = ts_to_lisp_source(ts2).expect("lowering2");
    assert!(ir2.contains(r#"(u128/add a "5")"#), "IR2: {ir2}");
}
