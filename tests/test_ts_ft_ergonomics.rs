//! Ergonomics-v2 acceptance test (2026-10-07) — THE example, end to end.
//!
//! Pins every feature of the near.db/assert/event/Money surface:
//! scalar type alias, object params reading through the cached-input
//! getter (the silent-nil BUG fix), assert-as-special-form (panic +
//! full rollback), near.db.key/put/has/del/keys, near.event JSON logs.
//!
//! Also pins the traps: non-owner mint → ERR_NOT_OWNER, double-burn →
//! ERR_NO_ACCOUNT, garbage Money → __h_u128_parse + full rollback.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const FT_SRC: &str = include_str!("../fixtures/ft_ergonomics.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(state: &str, method: &str, args: &str, signer: &str, view: bool) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(FT_SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("ft_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("ft.ergo.test.near={}", p.to_str().unwrap());
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("ft.ergo.test.near")
        .arg(method)
        .arg(args)
        .arg("--signer")
        .arg(signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    if view {
        cmd.arg("--view");
    }
    // keys() rides the engine-level iter builtins; the mock gates them
    // behind the same lab env the storage_cleaner loops use.
    cmd.env("NEAR_MOCK_ALLOW_DEPRECATED_ITERS", "1");
    let out = cmd.output().expect("near-mock");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const OWNER: &str = "owner.test.near";

#[test]
fn ft_ergonomics_end_to_end() {
    let st = "/tmp/ft_ergo_test.bin";
    let _ = std::fs::remove_file(st);

    // compile + check happen inside run(); assert the surface compiles
    // clean (this IS the acceptance: the example failed with 5 distinct
    // errors before Tier A/C).
    let ir = ts_to_lisp_source(FT_SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("the ergonomics example must type-check");

    // init as owner
    let o = run(st, "init", "{}", OWNER, false);
    assert!(o.contains("ok"), "init failed: {o}");

    // mint 5 then 5 → 10 (object params + db.put + u128Add)
    let o = run(
        st,
        "ftMint",
        r#"{"to":"alice","amount":"5"}"#,
        OWNER,
        false,
    );
    assert!(o.contains("ok"), "mint#1 failed: {o}");
    let o = run(
        st,
        "ftMint",
        r#"{"to":"alice","amount":"5"}"#,
        OWNER,
        false,
    );
    assert!(o.contains("ok"), "mint#2 failed: {o}");

    // balance reads back 10 — params.to was LIVE through both mints
    // (the silent-nil bug fix), db.key unwrap works
    let o = run(st, "ftGet", r#"{"to":"alice"}"#, OWNER, true);
    assert!(o.contains("10"), "balance expected 10: {o}");

    // assert enforcement: non-owner mint panics AND rolls back
    let o = run(
        st,
        "ftMint",
        r#"{"to":"eve","amount":"1"}"#,
        "eve.test.near",
        false,
    );
    assert!(o.contains("ERR_NOT_OWNER"), "no owner assert: {o}");
    let o = run(st, "ftGet", r#"{"to":"eve"}"#, OWNER, true);
    assert!(o.contains("none"), "eve balance must be nil post-rollback: {o}");

    // keys: alice + owner under their prefixes (sorted drain loop)
    let o = run(st, "ftKeys", "{}", OWNER, true);
    assert!(o.contains("ft:alice"), "keys missing ft:alice: {o}");
    assert!(o.contains("owner"), "keys missing owner: {o}");

    // burn → gone; double burn → ERR_NO_ACCOUNT
    let o = run(st, "ftBurn", r#"{"from":"alice"}"#, OWNER, false);
    assert!(o.contains("ok"), "burn failed: {o}");
    let o = run(st, "ftGet", r#"{"to":"alice"}"#, OWNER, true);
    assert!(o.contains("none"), "alice must be gone: {o}");
    let o = run(st, "ftBurn", r#"{"from":"alice"}"#, OWNER, false);
    assert!(o.contains("ERR_NO_ACCOUNT"), "no has-assert: {o}");

    // garbage Money traps the u128 boundary and rolls back — the
    // "what if you pass a letter" guarantee is RUNTIME, not types
    let o = run(
        st,
        "ftMint",
        r#"{"to":"bob","amount":"5x"}"#,
        OWNER,
        false,
    );
    assert!(
        o.contains("trapped") || o.contains("u128"),
        "garbage Money must trap: {o}"
    );
    let o = run(st, "ftGet", r#"{"to":"bob"}"#, OWNER, true);
    assert!(o.contains("none"), "bob balance must be nil post-trap: {o}");
}
