//! Logical && / || VALUE semantics (2026-10-04): operands keep their types
//! and the RESULT is one of the operand values (JS semantics), replacing the
//! old always-0/1 coercion. Each left operand evaluated at most once via a
//! let binding; short-circuit preserved. Truthiness mirrors `if` exactly
//! (tag-aware: false/nil/0 falsy; strings — including "" — truthy, the
//! documented M2 boundary).
//!
//! Run on near-mock; stateful single-eval cases use the cross-mode state
//! file (bump counters in contract storage).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = include_str!("../fixtures/logical_values.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn compile(src: &str, what: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("{what}: lowering: {e}"));
    if std::env::var("TS_LOGICAL_DEBUG").is_ok() {
        eprintln!("── lowered IR ──\n{ir}\n────────────────");
    }
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("{what}: parse: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("{what}: typecheck: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("{what}: codegen: {e}"))
}

fn strip_mock_debug(s: &str) -> String {
    // near-mock decorates ambiguous 8-byte returns: `7 (raw i64, untagged: 0)`
    // (raw-i64 twin dispatch for all-number exports) or `"abcd…" (8-byte str |
    // i64 view: N)`. The decoration is a mock VIEW artifact, not contract
    // output — strip it so raw-number exports assert on the bare value.
    if let Some(i) = s.find(" (raw i64") {
        s[..i].to_string()
    } else if let Some(i) = s.find(" (8-byte str") {
        s[..i].trim_matches('"').to_string()
    } else {
        s.to_string()
    }
}

fn run_plain(what: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let wasm = compile(SRC, what);
    let tmp = std::env::temp_dir().join(format!("nm_lv_{}.wasm", std::process::id()));
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
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return strip_mock_debug(rest.trim_end());
        }
    }
    panic!("{what}/{method}: no 📄 result line in mock output:\n{s}")
}

fn run_stateful(what: &str, state: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let wasm = compile(SRC, what);
    let p = std::env::temp_dir().join(format!("lv_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("lv.test.near={}", p.to_str().unwrap());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("lv.test.near")
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "{what}/{method}: near-mock failed:\n{s}"
    );
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return strip_mock_debug(rest.trim_end());
        }
    }
    panic!("{what}/{method}: no 📄 result line in mock output:\n{s}")
}

/// read the "__lv:c" bump counter out of the state file via the exported reader
fn read_counter(state: &str) -> i64 {
    run_stateful("logical_values", state, "readC", "{}")
        .parse::<i64>()
        .expect("counter numeric")
}

#[test]
fn compiles_and_typechecks() {
    let _l = lock();
    compile(SRC, "logical_values");
}

#[test]
fn or_returns_operand_value() {
    assert_eq!(run_plain("lv", "orDefault", r#"{"a":0,"b":7}"#), "7");
    assert_eq!(run_plain("lv", "orDefault", r#"{"a":5,"b":7}"#), "5");
    assert_eq!(run_plain("lv", "orDefault", r#"{"a":-3,"b":7}"#), "-3");
}

#[test]
fn and_returns_operand_value() {
    assert_eq!(run_plain("lv", "andGuard", r#"{"a":0,"b":9}"#), "0");
    assert_eq!(run_plain("lv", "andGuard", r#"{"a":3,"b":9}"#), "9");
}

#[test]
fn bool_operands_stay_boolean() {
    // bool && bool → Bool value; interpolation renders true/false
    assert_eq!(run_plain("lv", "boolChain", r#"{"x":2}"#), "true|true");
    assert_eq!(run_plain("lv", "boolChain", r#"{"x":0}"#), "false|false");
}

#[test]
fn string_or_follows_if_truthiness() {
    // "" is TRUTHY in this dialect (same as `if (s)` — string truthiness
    // is M2). Deliberate divergence from JS, documented in the fixture.
    // Bracket-wrapped return: bare "" would print no 📄 line in the mock.
    assert_eq!(run_plain("lv", "strOr", r#"{"s":"","d":"x"}"#), "[]");
    assert_eq!(run_plain("lv", "strOr", r#"{"s":"a","d":"x"}"#), "[a]");
}

#[test]
fn left_operand_evaluated_once() {
    let _ = std::fs::remove_file("/tmp/lv-t1.bin");
    assert_eq!(run_stateful("lv", "/tmp/lv-t1.bin", "onceLeft", "{}"), "42");
    assert_eq!(read_counter("/tmp/lv-t1.bin"), 1, "bump ran exactly once");
}

#[test]
fn and_short_circuits_right_operand() {
    let _ = std::fs::remove_file("/tmp/lv-t2.bin");
    // falsy left → right NOT evaluated, falsy VALUE returned
    assert_eq!(
        run_stateful("lv", "/tmp/lv-t2.bin", "onceRight", r#"{"a":0}"#),
        "0"
    );
    assert_eq!(
        read_counter("/tmp/lv-t2.bin"),
        0,
        "no bump when short-circuited"
    );
    // truthy left → right evaluated exactly once, its value returned
    assert_eq!(
        run_stateful("lv", "/tmp/lv-t2.bin", "onceRight", r#"{"a":1}"#),
        "9"
    );
    assert_eq!(read_counter("/tmp/lv-t2.bin"), 1, "exactly one bump");
}

#[test]
fn logical_in_if_condition_unchanged() {
    assert_eq!(run_plain("lv", "guardCall", r#"{"x":50}"#), "in");
    assert_eq!(run_plain("lv", "guardCall", r#"{"x":0}"#), "out");
    assert_eq!(run_plain("lv", "guardCall", r#"{"x":500}"#), "out");
}

#[test]
fn chains_flow_values() {
    assert_eq!(run_plain("lv", "orChain", r#"{"a":0,"b":0,"c":4}"#), "4");
    assert_eq!(run_plain("lv", "orChain", r#"{"a":0,"b":6,"c":4}"#), "6");
    assert_eq!(run_plain("lv", "andChain", r#"{"a":2,"b":3,"c":4}"#), "4");
    assert_eq!(run_plain("lv", "andChain", r#"{"a":2,"b":0,"c":4}"#), "0");
}
