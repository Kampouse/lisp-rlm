//! Loop-carried state + in-loop returns — regression suite for the 2026-09-11
//! fp254 fixes (see fixtures/loop_returns.ts for the full history):
//!
//! - hoist-order: while-body decls re-init per iteration in SOURCE order
//!   (the old insert(1,…) reversed them; `let C = …reads ai…` saw nil→0 /
//!   stale values — the "values vanish" report)
//! - nested returns propagate through function-level __fn_done/__fn_res;
//!   post-return statements are guarded (no storage commits after return)
//!
//! All expectations are python-verified simulations of the fixture code.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const LR: &str = include_str!("../fixtures/loop_returns.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn call(state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(LR).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("lr_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("lr.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("lr.t.near")
        .arg(method)
        .arg(args)
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default()
}

#[test]
fn fp254_array_writes_nested_whiles() {
    let st = "/tmp/lr-arr.bin";
    let _ = std::fs::remove_file(st);
    // THE fp254 repro: inner-loop array writes + dynamic reads.
    let r = call(st, "repro_arr", "{}");
    assert!(r.contains("📄 100,49"), "repro_arr: {r}");
    let r = call(st, "repro_arr2", "{}");
    assert!(r.contains("📄 2400,1200"), "repro_arr2: {r}");
    let r = call(st, "repro_arr3", "{}");
    assert!(r.contains("📄 65534,2"), "repro_arr3: {r}");
    // native-int-array version of the same algorithm — proves TAG_ARRAY
    // elements are tagged values, not strings: no strToNum/toStr hot loop
    let r = call(st, "repro_arr_int", "{}");
    assert!(r.contains("📄 100,49"), "repro_arr_int: {r}");
}

#[test]
fn cios_shape_mul() {
    let st = "/tmp/lr-mul3.bin";
    let _ = std::fs::remove_file(st);
    // (a=0x123456, b=0x9abcde)
    let r = call(st, "mul3", r#"{"a":1193046,"b":10133662}"#);
    assert!(r.contains("📄 281993103348"), "mul3 #1: {r}");
    let r = call(st, "mul3", r#"{"a":65535,"b":65535}"#);
    assert!(r.contains("📄 655340001"), "mul3 #2: {r}");
    let r = call(st, "mul3", r#"{"a":16777215,"b":16777215}"#);
    assert!(r.contains("📄 6554150240001"), "mul3 #3: {r}");
    let r = call(st, "mul3", r#"{"a":0,"b":12345}"#);
    assert!(r.contains("📄 0"), "mul3 #4: {r}");
    // early returns in the entry (before the loops)
    let r = call(st, "mul3", r#"{"a":-1,"b":5}"#);
    assert!(r.contains("📄 -1"), "mul3 neg a: {r}");
    let r = call(st, "mul3", r#"{"a":5,"b":-2}"#);
    assert!(r.contains("📄 -2"), "mul3 neg b: {r}");
}

#[test]
fn nested_loop_returns() {
    let st = "/tmp/lr-ret.bin";
    let _ = std::fs::remove_file(st);
    // return inside a nested while — before the fix this fell through to
    // `return total` (the value 77 vanished)
    let r = call(st, "nested_return", "{}");
    assert!(r.contains("📄 77"), "nested_return: {r}");
    // return inside a for nested in a while
    let r = call(st, "for_nested_return", "{}");
    assert!(r.contains("📄 55"), "for_nested_return: {r}");
    // three-level loop-carried accumulation
    let r = call(st, "three_deep", "{}");
    assert!(r.contains("📄 8"), "three_deep: {r}");
    // early return in a mid-function if
    let r = call(st, "if_return_early", r#"{"x":1}"#);
    assert!(r.contains("📄 7"), "if_return_early hit: {r}");
    let r = call(st, "if_return_early", r#"{"x":5}"#);
    assert!(r.contains("📄 105"), "if_return_early miss: {r}");
    // inner break: outer body statements still run, carry persists
    let r = call(st, "carry_over_break", "{}");
    assert!(r.contains("📄 6"), "carry_over_break: {r}");
}

#[test]
fn nested_return_skips_later_writes() {
    let st = "/tmp/lr-skip.bin";
    let _ = std::fs::remove_file(st);
    let r = call(st, "skip_write", "{}");
    assert!(r.contains("📄 99"), "skip_write returns 99: {r}");
    let r = call(st, "read_sk", "{}");
    assert!(
        r.contains("📄 before"),
        "storage after a nested return must stay 'before': {r}"
    );
}
