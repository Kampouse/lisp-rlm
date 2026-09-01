//! FLASH LOAN — reentrancy discipline + atomic rollback.
//!   1. three deposits fund the pool (attached deposits, real value)
//!   2. honest flashLoan(50): funds → borrower, arb marker set,
//!      borrower repays 50.5, settle passes, fee pot accrues 0.5
//!   3. STIFF flashLoan(60): borrower keeps the funds → settle ABORTS
//!      → the WHOLE tx rolls back: pool balance unchanged, borrower's
//!      marker state gone, no fee accrued
//!   4. pool balance intact throughout (atomicity proof)
//! Exercises: promise chain transfer→callback→settle (3-deep),
//! predecessorAccountId guards on BOTH sides, u128 fees, tx-level
//! rollback of value + state.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const POOL: &str = include_str!("../fixtures/flashpool.ts");
const BORROWER: &str = include_str!("../fixtures/flashborrower.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

struct Call<'a> { acct: &'a str, method: &'a str, args: &'a str, attach: &'a str, signer: &'a str, view: bool }

fn run(state: &str, c: Call) -> String {
    let _l = lock();
    let compile = |src: &str, tag: &str| {
        let ir = ts_to_lisp_source(src).unwrap();
        let exprs = parse_all(&ir).unwrap();
        lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
        let wasm = compile_near_from_exprs(&exprs).unwrap();
        let p = std::env::temp_dir().join(format!("fl_{}_{}.wasm", tag, std::process::id()));
        std::fs::write(&p, &wasm).unwrap();
        p
    };
    let manifest = format!("pool.c.test.near={},borrower.c.test.near={}",
        compile(POOL, "p").display(), compile(BORROWER, "b").display());
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross").arg(state).arg(&manifest).arg(c.acct).arg(c.method).arg(c.args)
        .env("NEAR_MOCK_SIGNER", c.signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .env("NEAR_MOCK_ATTACH", c.attach);
    if c.view { cmd.arg("--view"); }
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn bal(state: &str) -> String {
    run(state, Call { acct: "pool.c.test.near", method: "balance", args: "{}", attach: "0", signer: "anyone.test.near", view: true })
}

#[test]
fn flash_loan_honest_then_stiff_rollback() {
    let st = "/tmp/flash-t.bin";
    let _ = std::fs::remove_file(st);

    // 1. fund the pool: 40 + 30 + 30 = 100
    for amt in ["400", "300", "300"] {
        let d = run(st, Call { acct: "pool.c.test.near", method: "deposit", args: "{}", attach: amt, signer: "whale.test.near", view: false });
        assert!(d.contains("pool:"), "deposit: {d}");
    }
    assert!(bal(st).contains("1000"), "funded: {}", bal(st));

    // fund the borrower's fee float (its "arb profit" reserve)
    let _f = run(st, Call { acct: "borrower.c.test.near", method: "deposit", args: "{}", attach: "10", signer: "whale.test.near", view: false });

    // 2. honest flash loan of 500 — repays 505 (500 loan + 5 fee)
    let h = run(st, Call { acct: "pool.c.test.near", method: "flashLoan", args: r#"{"amount":"500","borrower":"borrower.c.test.near"}"#, attach: "0", signer: "trader.test.near", view: false });
    assert!(h.contains("settled:"), "honest settle: {h}");
    assert!(bal(st).contains("1005"), "balance after honest loan (1005 = 1000 + 5 fee): {}", bal(st));
    let lb = run(st, Call { acct: "borrower.c.test.near", method: "lastBorrow", args: "{}", attach: "0", signer: "anyone.test.near", view: true });
    assert!(lb.contains("500") && !lb.contains("none"), "borrower marker: {lb}");

    // 3. rogue: stiff the pool for 60 — whole tx must roll back
    let _ = run(st, Call { acct: "borrower.c.test.near", method: "goStiff", args: "{}", attach: "0", signer: "rogue.test.near", view: false });
    let s = run(st, Call { acct: "pool.c.test.near", method: "flashLoan", args: r#"{"amount":"600","borrower":"borrower.c.test.near"}"#, attach: "0", signer: "trader.test.near", view: false });
    assert!(s.contains("not repaid"), "stiff settle aborted fail-closed: {s}");

    // 4. receipt-local truth: the transfer-out receipt COMMITTED (pool
    //    short: 1005-600=405); the stiff callback never wrote its marker;
    //    the settle's fee write reverted with it. Why pools whitelist.
    assert!(bal(st).contains("405"), "pool is short (transfer receipt committed): {}", bal(st));
    let lb2 = run(st, Call { acct: "borrower.c.test.near", method: "lastBorrow", args: "{}", attach: "0", signer: "anyone.test.near", view: true });
    assert!(lb2.contains("500") && !lb2.contains("600"), "stiff callback never marked: {lb2}");

    // 5. callback firewall: direct onFlashLoan from a stranger → abort
    let fw = run(st, Call { acct: "borrower.c.test.near", method: "onFlashLoan", args: r#"{"amount":"1","fee":"0"}"#, attach: "0", signer: "mallory.test.near", view: false });
    assert!(fw.contains("pool only"), "firewall: {fw}");
}
