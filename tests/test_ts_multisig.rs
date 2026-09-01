//! MULTI-SIG — threshold governance + deferred batch function-call.
//!   1. request(target bump, {step:5}) → tx:1 (proposer auto-approves)
//!   2. execute with 1 approval → threshold rejection
//!   3. approve by a second owner → 2 approvals
//!   4. execute → batch FUNCTION-CALL dispatches bump(5) on the TARGET
//!      contract; the target REJECTS direct users (predecessor guard)
//!      but accepts the msig contract; resume burns the request
//!   5. double-execute rejected; direct bump on target rejected
//! Exercises: batch action function_call to a THIRD contract, promise
//! chain target→resume, predecessorAccountId guard, string-roster
//! dedup (substring discipline), u128 counter on the target.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const MSIG: &str = include_str!("../fixtures/multisig.ts");
const TARGET: &str = include_str!("../fixtures/msig_target.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

struct Call<'a> { acct: &'a str, method: &'a str, args: &'a str, signer: &'a str, view: bool }

fn run(state: &str, c: Call) -> String {
    let _l = lock();
    let compile = |src: &str, tag: &str| {
        let ir = ts_to_lisp_source(src).unwrap();
        let exprs = parse_all(&ir).unwrap();
        lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
        let wasm = compile_near_from_exprs(&exprs).unwrap();
        let p = std::env::temp_dir().join(format!("msig_{}_{}.wasm", tag, std::process::id()));
        std::fs::write(&p, &wasm).unwrap();
        p
    };
    let manifest = format!("msig.b.test.near={},target.b.test.near={}",
        compile(MSIG, "m").display(), compile(TARGET, "t").display());
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross").arg(state).arg(&manifest).arg(c.acct).arg(c.method).arg(c.args)
        .env("NEAR_MOCK_SIGNER", c.signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    if c.view { cmd.arg("--view"); }
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn multisig_threshold_and_deferred_call() {
    let st = "/tmp/msig-t.bin";
    let _ = std::fs::remove_file(st);

    // 1. alice proposes: bump(5)
    let r = run(st, Call { acct: "msig.b.test.near", method: "request", args: r#"{"target":"target.b.test.near","method":"bump","args":"{\"step\":\"5\"}"}"#, signer: "alice.test.near", view: false });
    assert!(r.contains("tx:1"), "request: {r}");

    // 2. one approval can't execute
    let e1 = run(st, Call { acct: "msig.b.test.near", method: "execute", args: r#"{"txId":"1"}"#, signer: "alice.test.near", view: false });
    assert!(e1.contains("threshold not met"), "1-approval execute rejected: {e1}");

    // double-approve rejected
    let dup = run(st, Call { acct: "msig.b.test.near", method: "approve", args: r#"{"txId":"1"}"#, signer: "alice.test.near", view: false });
    assert!(dup.contains("already approved"), "dup approve: {dup}");

    // 3. bob approves → 2
    let a2 = run(st, Call { acct: "msig.b.test.near", method: "approve", args: r#"{"txId":"1"}"#, signer: "bob.test.near", view: false });
    assert!(a2.contains("approvals:2"), "second approval: {a2}");

    // 4. execute → target bumped to 5, predecessor guard satisfied
    let ex = run(st, Call { acct: "msig.b.test.near", method: "execute", args: r#"{"txId":"1"}"#, signer: "carol.test.near", view: false });
    assert!(ex.contains("executed:"), "execute resume ran: {ex}");
    assert!(ex.contains("count:5"), "target bumped: {ex}");
    let cnt = run(st, Call { acct: "target.b.test.near", method: "getCount", args: "{}", signer: "anyone.test.near", view: true });
    assert!(cnt.contains("5"), "target state: {cnt}");

    // 5. double-execute rejected; direct bump rejected (predecessor guard)
    let again = run(st, Call { acct: "msig.b.test.near", method: "execute", args: r#"{"txId":"1"}"#, signer: "carol.test.near", view: false });
    assert!(again.contains("already executed"), "double execute: {again}");
    let direct = run(st, Call { acct: "target.b.test.near", method: "bump", args: r#"{"step":"1"}"#, signer: "mallory.test.near", view: false });
    assert!(direct.contains("only the multisig"), "predecessor guard: {direct}");
    let cnt2 = run(st, Call { acct: "target.b.test.near", method: "getCount", args: "{}", signer: "anyone.test.near", view: true });
    assert!(cnt2.contains("5"), "count unchanged after rejects: {cnt2}");
}
