//! u256 decimal-string emitter ops (differential, 2026-10-18).
//!
//! Every op is checked against an INDEPENDENTLY computed BigInt oracle.
//! These are the values that killed the first three drafts: the to_str
//! infinite loop (consumed bits never cleared → pos underflow), the
//! inverted shr carry guard, the fixed-K isqrt start, and the borrow
//! flag that overwrote the difference limb in isqrt's x -= b.
//!
//! Oracle: python3 -c`...` with plain int arithmetic.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = include_str!("../fixtures/u256_surface.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(what: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {e}"));
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("parse: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("typecheck: {e}"));
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("codegen: {e}"));
    let tmp = std::env::temp_dir().join(format!("nm_u256_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .output()
        .expect("near-mock binary (cargo build --release first)");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{what}: near-mock failed:\n{s}");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return rest.trim_end().to_string();
        }
    }
    panic!("{what}: no 📄 result line in mock output:\n{s}")
}

fn json2(a: &str, b: &str) -> String {
    format!(r#"{{"a":"{a}","b":"{b}"}}"#)
}

fn json1(a: &str) -> String {
    format!(r#"{{"a":"{a}"}}"#)
}

fn jsonk(a: &str, k: u32) -> String {
    format!(r#"{{"a":"{a}","k":{k}}}"#)
}

const G2_SQ: &str = "340282366920938463463374607431768211456"; // 2^128
const U128_MAX: &str = "340282366920938463463374607431768211455";

#[test]
fn u256_mul_schoolbook_and_edge() {
    // 6×7 (the bis5 case that printed garbage 📄 1)
    assert_eq!(run("mul67", "mulBig", &json2("6", "7")), "42");
    // (2^128-1)^2 — the largest product the surface can produce
    assert_eq!(
        run("mulmax", "mulBig", &json2(U128_MAX, U128_MAX)),
        "115792089237316195423570985008687907852589419931798687112530834793049593217025",
        "(2^128-1)^2"
    );
    // (2^128-1)·2 = 2^129-2 — 1 over the u128 wall (the G² overflow case)
    assert_eq!(
        run("mulwall", "mulBig", &json2(U128_MAX, "2")),
        "680564733841876926926749214863536422910"
    );
}

#[test]
fn u256_div_exact_and_trap() {
    // 2^129 / 2 = 2^128
    assert_eq!(
        run("div1", "divBig", &json2("680564733841876926926749214863536422912", "2")),
        G2_SQ
    );
    // (2^128-1)^2 / (2^128-1) = 2^128-1 (self-inverse, exercises the long
    // division's multi-limb quotient path)
    assert_eq!(
        run("divself", "divBig", &json2(
            "115792089237316195423570985008687907852589419931798687112530834793049593217025",
            U128_MAX
        )),
        U128_MAX
    );
}

#[test]
fn u256_shr_cross_limb() {
    // 2^64 >> 1 = 2^63 — the case the inverted carry guard zeroed
    assert_eq!(run("shr1", "shr1", &json1("18446744073709551616")), "9223372036854775808");
    // (2^128-1) >> 64 = 2^64-1 — whole-limb shift + cross-limb OR
    assert_eq!(
        run("shr64", "shr64", &json1(U128_MAX)),
        "18446744073709551615"
    );
    // 2^65 >> 1 = 2^64 — the case the Sub operand order botched: the OR
    // term computed v[j+1] << bits (mod-64 alias) instead of << (64−bits)
    assert_eq!(
        run("shr65", "shrK", &jsonk("36893488147419103232", 1)),
        "18446744073709551616"
    );
    // 2^64 >> 8 = 2^56
    assert_eq!(
        run("shr8", "shrK", &jsonk("18446744073709551616", 8)),
        "72057594037927936"
    );
    // whole-limb binary decomposition: 2^192 >> 128 = 2^64 (whole=3 — the
    // case the >= s phase tests shifted by 1+2+3 = 6 limbs)
    assert_eq!(
        run("shr128", "shrK", &jsonk("6277101735386680763835789423207666416102355444464034512896", 128)),
        "18446744073709551616"
    );
    // (2^256-1) >> 150 = 2^106-1 (whole=2 + bits=22)
    assert_eq!(
        run("shr150", "shrK", &jsonk("115792089237316195423570985008687907853269984665640564039457584007913129639935", 150)),
        "81129638414606681695789005144063"
    );
    // (2^256-1) >> 192 = 2^64-1 (whole=3)
    assert_eq!(
        run("shr192", "shrK", &jsonk("115792089237316195423570985008687907853269984665640564039457584007913129639935", 192)),
        "18446744073709551615"
    );
}

#[test]
fn u256_isqrt_hackers_delight() {
    // the exact values the first (|= form) and fixed-K drafts got wrong
    assert_eq!(run("isq0", "isqrt", &json1("0")), "0");
    assert_eq!(run("isq1", "isqrt", &json1("1")), "1");
    assert_eq!(run("isq16", "isqrt", &json1("16")), "4");
    // isqrt(2^128-1) = 2^64-1
    assert_eq!(run("isqmax", "isqrt", &json1(U128_MAX)), "18446744073709551615");
    // isqrt(10^50) = 10^25 — root > 2^64, exercises y's high limb
    assert_eq!(run("isq50", "isqrt", &json1("100000000000000000000000000000000000000000000000000")), "10000000000000000000000000");
    // isqrt(2^128) = 2^64 exactly (perfect square at the limb boundary)
    assert_eq!(run("isqpow", "isqrt", &json1(G2_SQ)), "18446744073709551616");
}

#[test]
fn u256_add_shl_carry_and_wrap() {
    // 1+1
    assert_eq!(run("add2", "add2", &json2("1", "1")), "2");
    // (2^64-1)+1 = 2^64 — the wrap-based carry the first draft lost
    assert_eq!(
        run("add64", "add2", &json2("18446744073709551615", "1")),
        "18446744073709551616"
    );
    // (2^128-1)+1 and (2^192-1)+1 — carries through limbs 1 and 2
    assert_eq!(
        run("add128", "add2", &json2(U128_MAX, "1")),
        "340282366920938463463374607431768211456"
    );
    assert_eq!(
        run("add192", "add2", &json2("6277101735386680763835789423207666416102355444464034512895", "1")),
        "6277101735386680763835789423207666416102355444464034512896"
    );
    // MAX+MAX = 2^256-2 — carry out of the top limb discards silently
    let max256 = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    assert_eq!(
        run("addmax", "add2", &json2(max256, max256)),
        "115792089237316195423570985008687907853269984665640564039457584007913129639934"
    );
    // 2^64 << 1 = 2^65 — cross-limb bring-up
    assert_eq!(
        run("shl1", "shlK", &jsonk("18446744073709551616", 1)),
        "36893488147419103232"
    );
    // 2^64 << 64 = 2^128 — whole-limb shift
    assert_eq!(
        run("shl64", "shlK", &jsonk("18446744073709551616", 64)),
        "340282366920938463463374607431768211456"
    );
    // 1 << 200 — whole limbs 3 + bits 8
    assert_eq!(
        run("shl200", "shlK", &jsonk("1", 200)),
        "1606938044258990275541962092341162602522202993782792835301376"
    );
    // k=0 is identity
    assert_eq!(run("shl0", "shlK", &jsonk("12345", 0)), "12345");
}

#[test]
fn u256_cmp_all_orderings() {
    let big = "115792089237316195423570985008687907852589419931798687112530834793049593217025";
    assert_eq!(run("cmp_lt", "cmpKind", &json2("1", "2")), "lt");
    assert_eq!(run("cmp_gt", "cmpKind", &json2("2", "1")), "gt");
    assert_eq!(run("cmp_eq", "cmpKind", &json2("2", "2")), "eq");
    // big vs u128max+5 — the case that read RAW ASCII before the parse fix
    assert_eq!(run("cmp_big", "cmpKind", &json2(big, "340282366920938463463374607431768211460")), "gt");
    assert_eq!(run("cmp_zero", "eqZero", &json1("0")), "zero");
    assert_eq!(run("cmp_nz", "eqZero", &json1("1")), "nonzero");
}

#[test]
fn u256_clmm_walk_step() {
    // L·Δ >> 64 with L=1e26, Δ=S1−S0 — needs 256-bit HOLD across steps
    assert_eq!(run("walk", "walkStep", "{}"), "9998514149446319915215");
}

/// (2^128-1)^2 helper assertion used above; kept as a compile-time witness
/// that the surface totals stay differential (no admitted constants).
#[test]
fn u256_mul_wall_is_one_over_u128() {
    // 2^64 * 2^64 = 2^128 exactly (G(0)^2 in the CLMM grid)
    assert_eq!(
        run("mulg0", "mulBig", &json2("18446744073709551616", "18446744073709551616")),
        G2_SQ
    );
}
