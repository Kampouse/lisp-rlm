//! Lending v4 — u128 + interest + LIQUIDATIONS (2026-09-01).
//!
//! Deterministic guards: NEAR_MOCK_BLOCK_TS pins time, NEAR_MOCK_SIGNER
//! impersonates the liquidator. Full sequence: deposit → max borrow
//! (health exactly 10000 = at the line) → at-line liquidation REFUSED
//! (the >= boundary bug lived here) → self-liquidation refused →
//! close-factor refused → alice liquidates 2e24: seizes 2.1e24 (5%
//! bonus), debt drops 5.137e24 → 3.137e24, health 9733 → 12591.

use std::process::Command;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/lending.ts");

fn run(method: &str, input: &str, ts_ns: &str, signer: Option<&str>) -> String {
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {}", e));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {}", e));
    let tmp = std::env::temp_dir().join(format!("nm_l4_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let mut cmd = Command::new("./target/release/near-mock");
    cmd.env("NEAR_MOCK_BLOCK_TS", ts_ns);
    if let Some(s) = signer {
        cmd.env("NEAR_MOCK_SIGNER", s);
    }
    let out = cmd.arg(&tmp).arg(method).arg(input).output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

const TS0: &str = "1800000000000000000";
const T100D: &str = "1808640000000000000"; // TS0 + 8_640_000s (machine-derived)

#[test]
fn lending_liquidations_full_guard_battery() {
    let _ = std::fs::remove_file("/tmp/near-mock-state.bin");

    let out = run("deposit", r#"{"amt":"10000000000000000000000000"}"#, TS0, None);
    assert!(out.contains(r#""dep":10000000000000000000000000"#), "{out}");
    // NOTE: json-set writes string values UNQUOTED (own:owner.test.near) —
// lenient round-trip works (self-guard matched it); strict-JSON quirk
// tracked separately.
    assert!(out.contains(r#""own":owner.test.near"#), "first deposit must stamp owner: {out}");

    // max borrow: debt lands EXACTLY on 5e24 (fee ceiled) → health 10000
    let out = run("borrow", r#"{"amt":"4761904761904761904761904"}"#, TS0, None);
    assert!(out.contains(r#""bor":5000000000000000000000000"#), "{out}");

    let out = run("health", "{}", TS0, None);
    assert!(out.contains("📄 10000"), "{out}");

    // at-the-line (health == LIQ_LINE): `>=` must INCLUDE equality → refuse
    let out = run("liquidate", r#"{"victim":"owner.test.near","amt":"1000000000000000000000000"}"#, TS0, Some("alice.test.near"));
    assert!(out.contains("account healthy"), "at-line account must not be liquidatable: {out}");

    // +100d: interest 136986301369863013698630 → bor 5136986301369863013698630, health 9733
    // borrower cannot liquidate themselves
    let out = run("liquidate", r#"{"victim":"owner.test.near","amt":"2000000000000000000000000"}"#, T100D, None);
    assert!(out.contains("cannot liquidate yourself"), "{out}");

    // close factor: 3e24 > bor/2 → refused
    let out = run("liquidate", r#"{"victim":"owner.test.near","amt":"3000000000000000000000000"}"#, T100D, Some("alice.test.near"));
    assert!(out.contains("close factor"), "{out}");

    // alice liquidates 2e24: seizes 2.1e24 (5% bonus), bor → 3136986301369863013698630
    let out = run("liquidate", r#"{"victim":"owner.test.near","amt":"2000000000000000000000000"}"#, T100D, Some("alice.test.near"));
    assert!(out.contains(r#""bor":3136986301369863013698630"#), "{out}");
    assert!(out.contains(r#""dep":7900000000000000000000000"#), "{out}");

    // health restored above the line: 9733 → 12591
    let out = run("health", "{}", T100D, None);
    assert!(out.contains("📄 12591"), "{out}");

    // ── withdraw (v4.1): boundary at health == 10000 exactly ──
    // post-liquidation: dep 7.9e24, bor 3136986301369863013698630
    // over-line withdraw refused
    let out = run("withdraw", r#"{"amt":"2000000000000000000000000"}"#, T100D, None);
    assert!(out.contains("undercollateralize"), "{out}");

    // MAX withdraw: dep-amt == bor*2 → dep'*5000 == bor*10000 exactly —
    // allowed (guard is strict <, consistent with borrow). health → 10000.
    let out = run("withdraw", r#"{"amt":"1626027397260273972602740"}"#, T100D, None);
    assert!(out.contains(r#""dep":6273972602739726027397260"#), "{out}");

    // one yocto more must abort
    let out = run("withdraw", r#"{"amt":"1"}"#, T100D, None);
    assert!(out.contains("undercollateralize"), "{out}");

    // repay the rest, then drain to zero — full lifecycle closes
    let out = run("repay", r#"{"amt":"3136986301369863013698630"}"#, T100D, None);
    assert!(out.contains(r#""bor":0"#), "{out}");
    let out = run("withdraw", r#"{"amt":"6273972602739726027397260"}"#, T100D, None);
    assert!(out.contains(r#""dep":0"#), "{out}");
}
