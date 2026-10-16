//! Price oracle — the YIELD pattern (NEAR PromiseYield, hosts 82/83).
//! requestPrice suspends: the callback runs once with a NotReady result
//! ("pending"), then RE-RUNS with the payload when a feeder contract in
//! a DIFFERENT PROCESS calls yieldResume. The handle persists in the
//! state file (\x00yield:<idx>) — one-shot, consumed on resume.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const TAKER: &str = include_str!("../fixtures/oracle_yield_taker.ts");
const FEEDER: &str = include_str!("../fixtures/oracle_yield_feeder.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(state: &str, taker: bool, acct: &str, method: &str, args: &str, view: bool) -> String {
    let _l = lock();
    let compile = |src: &str, tag: &str| {
        let ir = ts_to_lisp_source(src).unwrap();
        let exprs = parse_all(&ir).unwrap();
        lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
        let wasm = compile_near_from_exprs(&exprs).unwrap();
        let p = std::env::temp_dir().join(format!("yld_{}_{}.wasm", tag, std::process::id()));
        std::fs::write(&p, &wasm).unwrap();
        p
    };
    let manifest = if taker {
        format!(
            "oracle.y.test.near={},feeder.y.test.near={}",
            compile(TAKER, "t").display(),
            compile(FEEDER, "f").display()
        )
    } else {
        format!(
            "oracle.y.test.near={},feeder.y.test.near={}",
            compile(TAKER, "t").display(),
            compile(FEEDER, "f").display()
        )
    };
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg(acct)
        .arg(method)
        .arg(args)
        .env("NEAR_MOCK_SIGNER", "tester.test.near")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    if view {
        cmd.arg("--view");
    }
    let out = cmd.output().expect("near-mock");
    format!(
        "{}{}",
        normalize_num(&String::from_utf8_lossy(&out.stdout)),
        normalize_num(&String::from_utf8_lossy(&out.stderr))
    )
}

#[test]
fn yield_oracle_suspend_and_cross_process_resume() {
    let _ = std::fs::remove_file("/tmp/yld-t.bin");
    // 1. suspend: the yield stays pending. Since 9a4bfd94 (2026-10-14,
    //    real-chain semantics) promise_yield_create no longer runs an
    //    optimistic NotReady callback pass — onPrice first executes when
    //    the feeder calls yieldResume. The tx succeeds with no result
    //    line; the suspension itself is proven by the unset price below.
    let req = run(
        "/tmp/yld-t.bin",
        true,
        "oracle.y.test.near",
        "requestPrice",
        "{}",
        false,
    );
    assert!(req.contains("✅ Success"), "suspend: {req}");
    let price0 = run(
        "/tmp/yld-t.bin",
        true,
        "oracle.y.test.near",
        "getPrice",
        "{}",
        true,
    );
    assert!(
        price0.contains("unset"),
        "price unset before feed: {price0}"
    );

    // 2. cross-process resume: feeder delivers 4210
    let feed = run(
        "/tmp/yld-t.bin",
        false,
        "feeder.y.test.near",
        "feed",
        r#"{"yid":"yd:0","price":"4210"}"#,
        false,
    );
    assert!(
        feed.contains("priced:4210"),
        "callback re-ran with payload: {feed}"
    );
    assert!(feed.contains("fed"), "feeder ok: {feed}");

    // 3. the taker now has the price
    let price1 = run(
        "/tmp/yld-t.bin",
        true,
        "oracle.y.test.near",
        "getPrice",
        "{}",
        true,
    );
    assert!(price1.contains("4210"), "price stored: {price1}");

    // 4. one-shot: the consumed handle cannot be resumed again
    let again = run(
        "/tmp/yld-t.bin",
        false,
        "feeder.y.test.near",
        "feed",
        r#"{"yid":"yd:0","price":"9999"}"#,
        false,
    );
    assert!(
        again.contains("no such yield") || again.contains("not a yield"),
        "one-shot guard: {again}"
    );
    let price2 = run(
        "/tmp/yld-t.bin",
        true,
        "oracle.y.test.near",
        "getPrice",
        "{}",
        true,
    );
    assert!(
        price2.contains("4210"),
        "price unchanged after failed re-resume: {price2}"
    );
}

/// The near-mock's \u{1f4c4} printer comma-groups digit runs (human_num,
/// 7d2b3cb1). Strip the grouping so asserts written against raw digits keep
/// working. Commas inside non-digit contexts are preserved.
fn normalize_num(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b',' {
            let before = out.as_bytes().last().map_or(false, |c| c.is_ascii_digit());
            let after = bytes.get(i + 1).map_or(false, |c| c.is_ascii_digit());
            if before && after {
                i += 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
