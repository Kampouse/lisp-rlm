//! FT + lending desk in one contract (TS surface, 2026-09-01).
//!
//! Cross-module composition: the FT keyspaces ("ft:<who>", "ft:supply")
//! feed the lending desk ("lt:<who>" locked, "ld:<who>" debt). Same
//! token both sides → no price feed; borrow cap = locked * 5000 / 10000.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/ftlend.ts");

fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let m = LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn run(method: &str, input: &str, signer: Option<&str>) -> String {
    let ir = ts_to_lisp_source(SRC).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).expect("compile");
    let tmp = std::env::temp_dir().join(format!("nm_ftl_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    if let Some(s) = signer { cmd.env("NEAR_MOCK_SIGNER", s); }
    let out = cmd.arg(&tmp).arg(method).arg(input).output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn ftlend_cdp_lifecycle() {
    let _lock = state_lock();
    let _ = std::fs::remove_file(lisp_rlm_wasm::near_mock_state_file());

    let u = 10u128.pow(18);
    let m = 10u128.pow(6);
    let mint = 10*m*u;               // 10M
    let dep  = 8*m*u;                // 8M locked
    let cap  = 4*m*u;                // dep * 5000 / 10000
    let repay = m*u;                 // 1M
    let debt2 = cap - repay;         // 3M
    let wd_max = dep - 2*debt2;      // 2M — machine-derived bound

    assert!(run("ftMint", &format!(r#"{{"to":"owner.test.near","amount":"{mint}"}}"#), Some("owner.test.near")).contains(&format!("supply:{mint}")));

    assert!(run("lendDeposit", &format!(r#"{{"amount":"{dep}"}}"#), Some("owner.test.near")).contains(&format!("locked:{dep}")));

    // borrow EXACTLY at cap — (debt+amt > cap) is false at equality
    assert!(run("lendBorrow", &format!(r#"{{"amount":"{cap}"}}"#), Some("owner.test.near")).contains(&format!("debt:{cap}")));
    assert!(run("lendBorrow", r#"{"amount":"1"}"#, Some("owner.test.near")).contains("would exceed borrow cap"));

    // 100% utilized: any withdraw undercollateralizes
    assert!(run("lendWithdraw", r#"{"amount":"1"}"#, Some("owner.test.near")).contains("would undercollateralize"));

    // repay 1M → debt 3M; max withdraw = 2M (lands exactly on the line)
    assert!(run("lendRepay", &format!(r#"{{"amount":"{repay}"}}"#), Some("owner.test.near")).contains(&format!("debt:{debt2}")));
    assert!(run("lendWithdraw", &format!(r#"{{"amount":"{wd_max}"}}"#), Some("owner.test.near")).contains(&format!("locked:{}", dep - wd_max)));
    assert!(run("lendWithdraw", r#"{"amount":"1"}"#, Some("owner.test.near")).contains("would undercollateralize"));

    // repay burns: supply 10M - 1M = 9M, holder balance reflects it
    assert!(run("lendHealth", "{}", Some("owner.test.near")).contains(&format!("cap:{} debt:{}", 3*m*u, debt2)));
}
