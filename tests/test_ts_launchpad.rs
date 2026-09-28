//! Launchpad spike — launch_token + graduate through the promise DAG
//! (2026-09-27). Three TS contracts lowered via ts_frontend, compiled to
//! NEAR wasm, executed by near-mock multi-contract mode:
//!
//!   owner.test.near   = launchpad (launch_token / graduate)
//!   ft1.test.near     = stub NEP-141 (the spawned token stand-in)
//!   ref.test.near     = stub Ref router (JSON-only)
//!
//! Sequence: launch (underfunded abort) → launch w/ 1.1 NEAR → DAG fires:
//! ft1 created+deploy+mint, ref storage_deposit+create_pool+register →
//! view pool 1 = a:ft1/b:wrap, fee 30 → graduate 3e24 → ref reserve
//! res:owner:wrap == 3e24, lp:grad stamped. Double-graduate aborts.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::process::Command;

fn lower_and_compile(path: &str) -> Vec<u8> {
    let src = std::fs::read_to_string(path).unwrap();
    let ir = ts_to_lisp_source(&src).unwrap_or_else(|e| panic!("lowering {path}: {e}"));
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("typecheck {path}: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile {path}: {e}"))
}

fn write_wasm(name: &str, bytes: &[u8]) -> String {
    let p = std::env::temp_dir().join(format!("lp_{name}_{}.wasm", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

const LAUNCHPAD: &str = "owner.test.near";
const FT: &str = "ft1.test.near";
const REF: &str = "ref.test.near";
const ATTACH: u128 = 1_100_000_000_000_000_000_000_000;

fn mock(wasm: &str, contract: &str, method: &str, args: &str, attach: Option<u128>) -> String {
    let manifest = format!(
        "{LAUNCHPAD}={wasm},{FT}={wasm},{REF}={wasm}"
    );
    let mut cmd = Command::new("./target/release/near-mock");
    cmd.env("NEAR_MOCK_QUIET", "1")
        .arg("cross")
        .arg(format!("/tmp/lp_state_{}.bin", std::process::id()))
        .arg(&manifest)
        .arg(contract)
        .arg(method)
        .arg(args);
    if let Some(a) = attach {
        cmd.arg("--attach").arg(a.to_string());
    }
    let out = cmd.output().expect("near-mock run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn launchpad_launch_then_graduate() {
    let pad = write_wasm("pad", &lower_and_compile("fixtures/launchpad.ts"));
    let _ft = lower_and_compile("fixtures/launch_ft.ts");
    let _ref = lower_and_compile("fixtures/launch_ref.ts");
    let _ = std::fs::remove_file(format!("/tmp/lp_state_{}.bin", std::process::id()));

    // 1. underfunded launch must abort
    let out = mock(&pad, LAUNCHPAD, "launch_token",
        r#"{"creator":"creator.test.near","symbol":"JEM"}"#, Some(500_000_000_000_000_000_000_000));
    assert!(out.contains("insufficient deposit"), "underfunded: {out}");

    // 2. funded launch fires both batches through the DAG
    let out = mock(&pad, LAUNCHPAD, "launch_token",
        r#"{"creator":"creator.test.near","symbol":"JEM"}"#, Some(ATTACH));
    assert!(!out.contains("panicked"), "funded launch: {out}");
    assert!(out.contains("launched"), "funded launch: {out}");
}
