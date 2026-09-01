//! Atomic swap — two mirrored HTLC legs sharing one secret (2026-09-01).
//!
//! Safety property: Bob's claim window ends at the MIDPOINT between his
//! lock and Alice's refund deadline, so Alice can never reclaim her A
//! while Bob's B is still claimable. States: A → BOTH → B_CLAIMED → DONE,
//! with refund side-exits (B_REFUND returns to A for Alice's refund).

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/swap.ts");

fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let m = LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn run(method: &str, input: &str, signer: Option<&str>, ts: Option<i64>) -> String {
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {e}"));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {e}"));
    let tmp = std::env::temp_dir().join(format!("nm_swap_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg(&tmp).arg(method).arg(input);
    if let Some(s) = signer { cmd.env("NEAR_MOCK_SIGNER", s); }
    if let Some(t) = ts { cmd.env("NEAR_MOCK_BLOCK_TS", t.to_string()); }
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn swap_happy_path_and_aborts() {
    let _lock = state_lock();
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");
    let u: u128 = 10u128.pow(18);
    let (mint, amt_a, amt_b) = (1_000_000*u, 300_000*u, 250_000*u);
    let t0: i64 = 1_800_000_000_000_000_000;
    let tla: i64 = t0 + 600_000_000_000;              // 600s
    let lock: i64 = t0 + 100_000_000_000;             // bob locks at T0+100s
    let tlb: i64 = lock + (tla - lock) / 2;           // machine-derived midpoint
    let clb: i64 = lock + 50_000_000_000;             // alice claims B (before tlB)
    let cla: i64 = lock + 150_000_000_000;            // bob claims A (before tlA)

    assert!(run("faMint", &format!(r#"{{"to":"alice.test.near","amount":"{mint}"}}"#), Some("alice.test.near"), Some(t0)).contains(&format!("supply:{mint}")));
    assert!(run("fbMint", &format!(r#"{{"to":"bob.test.near","amount":"{mint}"}}"#), Some("bob.test.near"), Some(t0)).contains(&format!("supply:{mint}")));

    // pre-conditions refuse
    assert!(run("swapClaimA", r#"{"id":"1","secret":"swap-secret-1"}"#, Some("bob.test.near"), Some(t0)).contains("not in state B_CLAIMED"));
    assert!(run("swapLockB", r#"{"id":"9"}"#, Some("bob.test.near"), Some(t0)).contains("not in state A"));

    assert!(run("swapNew", &format!(r#"{{"amountA":"{amt_a}","amountB":"{amt_b}","timeoutSec":"600","secret":"swap-secret-1"}}"#), Some("alice.test.near"), Some(t0)).contains("swap:1"));
    assert!(run("faBalanceOf", r#"{"who":"alice.test.near"}"#, None, Some(t0)).contains(&format!("📄 {}", mint - amt_a)));
    assert!(run("swapLockB", r#"{"id":"1"}"#, Some("bob.test.near"), Some(lock)).contains(&format!("locked:{tlb}")));
    assert!(run("fbBalanceOf", r#"{"who":"bob.test.near"}"#, None, Some(lock)).contains(&format!("📄 {}", mint - amt_b)));

    // guard ladder while BOTH
    assert!(run("swapRefundA", r#"{"id":"1"}"#, Some("alice.test.near"), Some(lock)).contains("not in state A"));
    assert!(run("swapRefundB", r#"{"id":"1"}"#, Some("bob.test.near"), Some(lock)).contains("not yet timed out"));
    assert!(run("swapClaimB", r#"{"id":"1","secret":"nope"}"#, Some("alice.test.near"), Some(clb)).contains("wrong secret"));
    assert!(run("swapClaimB", r#"{"id":"1","secret":"swap-secret-1"}"#, Some("bob.test.near"), Some(clb)).contains("only the initiator"));

    // the swap itself
    assert!(run("swapClaimB", r#"{"id":"1","secret":"swap-secret-1"}"#, Some("alice.test.near"), Some(clb)).contains(&format!("claimedB:{amt_b}")));
    assert!(run("swapClaimB", r#"{"id":"1","secret":"swap-secret-1"}"#, Some("alice.test.near"), Some(clb)).contains("not in state BOTH"));
    assert!(run("swapClaimA", r#"{"id":"1","secret":"swap-secret-1"}"#, Some("bob.test.near"), Some(cla)).contains(&format!("claimedA:{amt_a}")));
    assert!(run("faBalanceOf", r#"{"who":"bob.test.near"}"#, None, Some(cla)).contains(&format!("📄 {amt_a}")));
    assert!(run("swapRefundB", r#"{"id":"1"}"#, Some("bob.test.near"), Some(tla + 1)).contains("not in state BOTH"));
}

#[test]
fn swap_abandonment_refunds() {
    let _lock = state_lock();
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");
    let u: u128 = 10u128.pow(18);
    let (mint, amt_a, amt_b) = (1_000_000*u, 100_000*u, 80_000*u);
    let t0: i64 = 1_800_000_000_000_000_000;
    let tla: i64 = t0 + 300_000_000_000;
    let lock: i64 = t0 + 100_000_000_000;
    let tlb: i64 = lock + (tla - lock) / 2;

    run("faMint", &format!(r#"{{"to":"alice.test.near","amount":"{mint}"}}"#), Some("alice.test.near"), Some(t0));
    run("fbMint", &format!(r#"{{"to":"bob.test.near","amount":"{mint}"}}"#), Some("bob.test.near"), Some(t0));

    // swap#1: bob never locks → alice refunds after tlA
    run("swapNew", &format!(r#"{{"amountA":"{amt_a}","amountB":"{amt_b}","timeoutSec":"300","secret":"s2"}}"#), Some("alice.test.near"), Some(t0));
    assert!(run("swapRefundA", r#"{"id":"1"}"#, Some("alice.test.near"), Some(tla - 1)).contains("not yet timed out"));
    assert!(run("swapLockB", r#"{"id":"1"}"#, Some("bob.test.near"), Some(tla + 1)).contains("timed out"));
    assert!(run("swapRefundA", r#"{"id":"1"}"#, Some("alice.test.near"), Some(tla + 1)).contains(&format!("refundedA:{amt_a}")));
    assert!(run("faBalanceOf", r#"{"who":"alice.test.near"}"#, None, Some(tla + 1)).contains(&format!("📄 {mint}")));

    // swap#2: both lock, alice never claims → bob at tlB, then alice at tlA
    run("swapNew", &format!(r#"{{"amountA":"{amt_a}","amountB":"{amt_b}","timeoutSec":"300","secret":"s3"}}"#), Some("alice.test.near"), Some(t0));
    run("swapLockB", r#"{"id":"2"}"#, Some("bob.test.near"), Some(lock));
    assert!(run("swapClaimB", r#"{"id":"2","secret":"s3"}"#, Some("alice.test.near"), Some(tlb + 1)).contains("B window closed"));
    assert!(run("swapRefundB", r#"{"id":"2"}"#, Some("bob.test.near"), Some(tlb + 1)).contains(&format!("refundedB:{amt_b}")));
    assert!(run("swapRefundA", r#"{"id":"2"}"#, Some("alice.test.near"), Some(tla + 1)).contains(&format!("refundedA:{amt_a}")));
    assert!(run("faBalanceOf", r#"{"who":"alice.test.near"}"#, None, Some(tla + 1)).contains(&format!("📄 {mint}")));
    assert!(run("fbBalanceOf", r#"{"who":"bob.test.near"}"#, None, Some(tla + 1)).contains(&format!("📄 {mint}")));
}
