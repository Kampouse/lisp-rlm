//! Surface Tour 2 — every under-tested TS surface item, one battery (2026-09-01).
//!
//! Two fixtures:
//! - `surface_tour2.ts` — hosts present in the near-mock: lowered, typechecked,
//!   compiled, AND executed. Every export must run and return shape-correct
//!   values (fail-closed on the mock's deterministic env).
//! - `surface_tour2_exotic.ts` — crypto hosts absent from the mock:
//!   compile-gated only (lower + typecheck + codegen). Real-value testing for
//!   these lands with protocol #16 (BLS threshold multisig) on testnet.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const TOUR_SRC: &str = include_str!("../fixtures/surface_tour2.ts");
const EXOTIC_SRC: &str = include_str!("../fixtures/surface_tour2_exotic.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Lower + typecheck + compile to WASM. Fails loud on any stage.
fn compile(src: &str, what: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("{what}: lowering: {e}"));
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("{what}: parse: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("{what}: typecheck: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("{what}: codegen: {e}"))
}

/// Compile + instantiate in near-mock + call an exported method.
fn run(src: &str, what: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let wasm = compile(src, what);
    let tmp = std::env::temp_dir().join(format!("nm_tour2_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock binary (cargo build --release first)");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "{what}/{method}: near-mock failed:\n{s}"
    );
    // Extract the result line: mock prints `📄 <value>` for the exported
    // method's return (other lines = host-call trace + storage dump).
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return rest.trim_end().to_string();
        }
    }
    panic!("{what}/{method}: no 📄 result line in mock output:\n{s}")
}

fn run_raw(src: &str, what: &str, method: &str, input: &str) -> String {
    // Like run(), but returns the FULL mock output — for tests asserting on
    // trap/panic paths where no 📄 result line exists by design.
    let _l = lock();
    let wasm = compile(src, what);
    let tmp = std::env::temp_dir().join(format!("nm_tour2_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock binary (cargo build --release first)");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// ── compile-gated: the exotic crypto surface ────────────────────────────

#[test]
fn exotic_compiles_and_typechecks() {
    let _l = lock();
    compile(EXOTIC_SRC, "exotic");
}

#[test]
fn exotic_panic_guard_aborts_on_zero() {
    // panicGuard takes a named bigint arg — the mock feeds it as JSON.
    // ok=0 must abort loudly (run_raw: no 📄 line exists on the trap path);
    // ok=1 must survive.
    let abort = run_raw(EXOTIC_SRC, "exotic", "panicGuard", r#"{"ok":0}"#);
    assert!(
        abort.contains("PANIC") || abort.contains("panic") || abort.contains("not ok"),
        "panic path should abort loudly: {abort}"
    );
    let ok = run(EXOTIC_SRC, "exotic", "panicGuard", r#"{"ok":1}"#);
    assert!(ok.contains("survived"), "ok path should survive: {ok}");
}

// ── executed: every mock-backed host ────────────────────────────────────

#[test]
fn string_method_surface() {
    let out = run(TOUR_SRC, "tour", "strMethods", "{}");
    // 14 + "SEI" + "6" + "e" + "hello" + "hello lisp-rlm!"
    let acc = out.trim();
    assert!(acc.contains("SEI"), "string methods ran: {out}");
    assert!(acc.contains("hello lisp-rlm!"), "concat applied: {out}");
}

#[test]
fn syntax_for_of_and_incr() {
    let out = run(TOUR_SRC, "tour", "syntaxTour", "{}");
    // abc:31 — for-of joins (via .concat), 3×10 + 1 = 31
    assert!(
        out.contains("abc:31"),
        "for-of + i++ + += + template literal: {out}"
    );
}

#[test]
fn ctx_hosts_shapes() {
    // randomSeed → 64 hex chars (32B); signerAccountPk → mock's 51-char
    // ed25519 string; gas/usage counters ≥ 0; depositGte(0,0) on 0-deposit = 1.
    let out = run(TOUR_SRC, "tour", "ctx", "{}");
    let parts: Vec<&str> = out.trim().split(':').collect();
    assert_eq!(parts.len(), 7, "ctx returned 7 fields: {out}");
    assert_eq!(parts[0], "64", "randomSeed is 64 hex chars: {out}");
    assert_eq!(parts[1], "51", "signerAccountPk mock is 51 chars: {out}");
    for (i, p) in parts[2..6].iter().enumerate() {
        assert_eq!(
            *p, "true",
            "field {i} (gas/prepaid/usage/depHi) true: {out}"
        );
    }
    // depositGte returns TAG_BOOL; `${ok == 1}` is bool-vs-num → false
    // (interpreter parity: (= #t 1) is false — different tags). The
    // fixture's `// 1` comment means "true", not the numeral.
    assert_eq!(
        parts[6], "false",
        "depositGte(0,0) is bool true; == 1 compares across tags → false: {out}"
    );
}

#[test]
fn input_echo() {
    let out = run(TOUR_SRC, "tour", "inputEcho", r#"{"a":1}"#);
    assert!(
        out.contains(r#""a":1"#),
        "input() returns raw args JSON: {out}"
    );
}

#[test]
fn iter_round_trip_via_mock_storage() {
    // The mock's storage_iter_* are host noops (real iteration is a bytecode-
    // VM feature), but the string-arg surface must instantiate and run
    // without trapping — that's the surface guarantee this battery pins.
    let out = run(TOUR_SRC, "tour", "iterProbe", "{}");
    assert!(
        out.contains("iter:"),
        "iterPrefix/iterNext ran clean: {out}"
    );
}

#[test]
fn u128_storage_round_trip() {
    let out = run(TOUR_SRC, "tour", "numStorage", "{}");
    assert_eq!(
        out.trim(),
        "340282366920938463463374607431768211455",
        "storeU128/loadU128 u128::MAX round trip: {out}"
    );
}

#[test]
fn money_noops_run_clean() {
    // promiseBatchActionCreateAccount + transfer + transferU128 on the mock's
    // noop batch hosts — must not trap.
    let out = run(TOUR_SRC, "tour", "money", "{}");
    assert!(out.contains("money-ok"), "batch create/transfer ran: {out}");
}

#[test]
fn json_array_args() {
    // {"ks":["x","y"]} → first element "x" (fixture returns `jsonArr:${first}`)
    let out = run(TOUR_SRC, "tour", "jsonArr", r#"{"ks":["x","y"]}"#);
    assert!(
        out.contains("jsonArr:x"),
        "jsonArr first element: {out}"
    );
}

#[test]
fn every_export_lowered() {
    // Belt-and-braces: every exported fn in BOTH fixtures lowers+compiles.
    let _l = lock();
    compile(TOUR_SRC, "tour");
    compile(EXOTIC_SRC, "exotic");
}
