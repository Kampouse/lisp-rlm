//! Portfolio aggregator — PARALLEL cross-contract fan-out via promise_and
//! (2026-09-01). One callback reads BOTH sub-call results (flattened in
//! dep order) and u128-adds them. Found bug #16: promise_result aliased
//! TEMP_MEM — two live results overwrote each other (250K+250K=500K
//! instead of 950K). Fixed with a forced heap copy.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const TOKEN_SRC: &str = include_str!("../fixtures/token_view.ts");
const PF_SRC: &str = include_str!("../fixtures/portfolio_fanout.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn w(src: &str, tag: &str) -> String {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("lower: {e}"));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {e}"));
    let p = std::env::temp_dir().join(format!("pf_{}_{}.wasm", tag, std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    p.to_str().unwrap().into()
}

fn run(acct: &str, method: &str, args: &str, signer: &str) -> String {
    let _l = lock();
    let tok = w(TOKEN_SRC, "tok");
    let pf = w(PF_SRC, "pf");
    let manifest = format!(
        "toka.pf.test.near={},tokb.pf.test.near={},portfolio.pf.test.near={}",
        tok, tok, pf
    );
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross").arg("/tmp/pf-test-state.bin").arg(&manifest).arg(acct).arg(method).arg(args)
        .env("NEAR_MOCK_SIGNER", signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn portfolio_parallel_fanout() {
    let _ = std::fs::remove_file("/tmp/pf-test-state.bin");
    let u: u128 = 10u128.pow(18);

    // same token wasm, TWO accounts = two independent ledgers
    assert!(run("toka.pf.test.near", "ftMint", &format!(r#"{{"to":"alice.test.near","amount":"{}"}}"#, 700_000*u), "alice.test.near").contains("supply:"));
    assert!(run("tokb.pf.test.near", "ftMint", &format!(r#"{{"to":"alice.test.near","amount":"{}"}}"#, 250_000*u), "alice.test.near").contains("supply:"));

    // THE FAN-OUT: promise_and join, one resume, two promise_results summed
    let out = run("portfolio.pf.test.near", "portfolioTotal", r#"{"user":"alice.test.near"}"#, "alice.test.near");
    assert!(out.contains(&format!("total:{}", 950_000*u)),
        "expected exactly 700K+250K=950K — wrong sum means promise_result aliasing: {out}");

    // asymmetric: bob has A-only balance (B view returns "0")
    assert!(run("toka.pf.test.near", "ftMint", &format!(r#"{{"to":"bob.test.near","amount":"{}"}}"#, 100_000*u), "alice.test.near").contains("supply:"));
    assert!(run("portfolio.pf.test.near", "portfolioTotal", r#"{"user":"bob.test.near"}"#, "bob.test.near").contains(&format!("total:{}", 100_000*u)));

    // empty user: 0+0
    assert!(run("portfolio.pf.test.near", "portfolioTotal", r#"{"user":"carol.test.near"}"#, "carol.test.near").contains("total:0"));

    // re-run alice: same 950K (idempotent view, no double counting)
    assert!(run("portfolio.pf.test.near", "portfolioTotal", r#"{"user":"alice.test.near"}"#, "alice.test.near").contains(&format!("total:{}", 950_000*u)));
}
