//! Hot-loop gas optimizations (2026-09-14) — two emitter improvements,
//! measured on a 1000-iteration numeric sum loop:
//!
//! 1. While-cond fast path: a numeric comparison cond ((< a b) etc.)
//!    branches DIRECTLY on the raw i32 compare. The generic path re-tagged
//!    the result as Bool and ran the triple-tag truthiness dispatch on it
//!    (~16 dead instrs per iteration of every hot loop). (< a b) is
//!    numeric-only semantics anyway (cmp untags both operands).
//! 2. Numeric-provenance `+`: emit_poly_add skips the runtime string
//!    dispatch when all operands are syntactically guaranteed TAG_NUM
//!    (literals, numeric locals, numeric-binop results, str->num, ...).
//!    Tracking at let/set! sites with shadow save/restore; unknown shapes
//!    keep the safe polymorphic dispatch.
//!
//! Combined: 0.111 → 0.066 Tgas per 1000-iteration sum (-41%).
//! Concat correctness for string/host operands is pinned here too (the
//! bug class that made `+` polymorphic in the first place).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function sumFor(n: number): string {
  let s = 0;
  for (let i = 0; i < n; i = i + 1) { s = s + i; }
  return toStr(s);
}
export function sumWhile(n: number): string {
  let s = 0;
  let i = 0;
  while (i < n) { s = s + i; i = i + 1; }
  return toStr(s);
}
// concat shapes that MUST keep the polymorphic dispatch
export function ss(): string { const a = "x"; const b = "y"; return a + b; }
export function sn(): string { const a = "n="; const n = 5; return a + n; }
export function ns(): string { const n = 7; const s = "s"; return n + s; }
export function chain(): string {
  let r = "";
  for (let i = 0; i < 3; i = i + 1) { r = r + toStr(i); }
  return r;
}
// numeric local demoted mid-function: s is numeric, then reassigned to a
// host result — the `+` AFTER the demotion must dispatch again
export function demote(): string {
  let s = 0;
  s = near.storageGet("nope") ?? "str";
  return s + "!";
}
export function eqNil(): string {
  const v = near.jsonGetInt("nope");
  if (v == near.jsonGetInt("nope2")) { return "eq"; }
  return "ne";
}
// input-cache (2026-09-14): getters share ONE input read per tx; the
// 2-arg jsonGetStr scans JSON_SCAN_BUF and must NOT clobber the cache
export function cacheMix(): string {
  const o = near.jsonGetStr("outer") ?? "{}";
  const inner = near.jsonGetStr("inner", o);
  const after = near.jsonGetStr("k") ?? "MISS";
  const n1 = near.jsonGetInt("n") ?? -1;
  return after + ":" + toStr(n1) + ":" + inner;
}
export function manyArgs(): string {
  let s = 0;
  s = s + (near.jsonGetInt("a1") ?? 0);
  s = s + (near.jsonGetInt("a2") ?? 0);
  s = s + (near.jsonGetInt("a3") ?? 0);
  s = s + (near.jsonGetInt("a4") ?? 0);
  s = s + (near.jsonGetInt("a5") ?? 0);
  s = s + (near.jsonGetInt("a6") ?? 0);
  return toStr(s);
}
export function eqNum(n: number): string {
  if (n == 42) { return "forty-two"; }
  return "other";
}
export function neNum(n: number): string {
  if (n != 0) { return "nonzero"; }
  return "zero";
}
export function bareTruthy(n: number): string {
  if (n) { return "truthy"; }
  return "falsy";
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

struct Out {
    value: String,
    gas: f64,
}

fn run(method: &str, args: &str) -> Out {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("lgas_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("lgas_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("lgas.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("lgas.t.near")
        .arg(method)
        .arg(args)
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let value = all
        .lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&all)
        .to_string();
    let gas = all
        .lines()
        .find(|l| l.contains("Tgas burnt"))
        .and_then(|l| l.split("gas:").nth(1))
        .and_then(|g| g.split("Tgas").next())
        .and_then(|g| g.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    Out { value, gas }
}

#[test]
fn sum_for_correct() {
    let r = run("sumFor", r#"{"n":100}"#);
    assert!(r.value.contains("4950"), "sumFor: {}", r.value);
}

#[test]
fn sum_while_correct() {
    let r = run("sumWhile", r#"{"n":100}"#);
    assert!(r.value.contains("4950"), "sumWhile: {}", r.value);
}

#[test]
fn hot_loop_gas_under_budget() {
    // 2026-09-14 baseline (pre-optimization): 0.111 Tgas / 1000 iters.
    // Optimized: 0.066. Budget with margin — a regression past this means
    // the fast paths stopped applying (e.g. provenance tracking broke).
    let r = run("sumFor", r#"{"n":1000}"#);
    assert!(r.value.contains("499500"), "sumFor value: {}", r.value);
    assert!(
        r.gas < 0.085,
        "sumFor gas regressed past 0.085 Tgas: {} (pre-opt was 0.111, post-opt 0.066)",
        r.gas
    );
}

#[test]
fn concat_str_str() {
    let r = run("ss", "{}");
    assert!(r.value.contains("xy"), "ss: {}", r.value);
}

#[test]
fn concat_str_num() {
    let r = run("sn", "{}");
    assert!(r.value.contains("n=5"), "sn: {}", r.value);
}

#[test]
fn concat_num_str() {
    let r = run("ns", "{}");
    assert!(r.value.contains("7s"), "ns: {}", r.value);
}

#[test]
fn concat_in_loop_accumulator() {
    let r = run("chain", "{}");
    assert!(r.value.contains("012"), "chain: {}", r.value);
}

#[test]
fn numeric_local_demoted_by_host_result() {
    // s starts numeric (0), gets reassigned a host result — the later +
    // must dispatch (concat), not tagged-add the string descriptor
    let r = run("demote", "{}");
    assert!(r.value.contains("str!"), "demote: {}", r.value);
}

#[test]
fn eq_num_fast_path() {
    let r = run("eqNum", r#"{"n":42}"#);
    assert!(r.value.contains("forty-two"), "eqNum: {}", r.value);
    let r = run("eqNum", r#"{"n":7}"#);
    assert!(r.value.contains("other"), "eqNum: {}", r.value);
}

#[test]
fn ne_num_fast_path() {
    let r = run("neNum", r#"{"n":5}"#);
    assert!(r.value.contains("nonzero"), "neNum: {}", r.value);
    let r = run("neNum", r#"{"n":0}"#);
    assert!(r.value.contains("zero"), "neNum: {}", r.value);
}

#[test]
fn bare_truthy_fast_path() {
    let r = run("bareTruthy", r#"{"n":5}"#);
    assert!(r.value.contains("truthy"), "bareTruthy: {}", r.value);
    let r = run("bareTruthy", r#"{"n":0}"#);
    assert!(r.value.contains("falsy"), "bareTruthy: {}", r.value);
}

#[test]
fn input_cache_survives_string_arg_scan() {
    // jsonGetStr(key, json) scans into JSON_SCAN_BUF — the cached input in
    // INPUT_CACHE_BUF must survive it (k and n read correctly AFTER)
    let r = run(
        "cacheMix",
        r#"{"outer" : { "inner" : "deep" }, "k": "v", "n" : 1}"#,
    );
    assert!(r.value.contains("v:1:deep"), "cacheMix: {}", r.value);
}

#[test]
fn many_getters_gas_under_budget() {
    // 6 int getters: 0.083 Tgas before input caching, 0.024 after. The
    // budget has margin — a regression means the cache stopped hitting
    // (flag/buffer mismatch, clobbering, etc).
    let r = run("manyArgs", r#"{"a1":1,"a2":2,"a3":3,"a4":4,"a5":5,"a6":6}"#);
    assert!(r.value.contains("21"), "manyArgs: {}", r.value);
    assert!(
        r.gas < 0.045,
        "manyArgs gas regressed past 0.045 Tgas: {} (pre-cache 0.083, post 0.024)",
        r.gas
    );
}

// ── equality/truthiness fast-path traps (2026-09-14) ──────────────────
// The eq fast path (tagged-direct i64.eq) and bare-truthiness fast path
// (i64.eqz) each had a correctness trap, both caught by suites:
// 1. i64.eq yields i32 — emit_tag_bool shifts an i64 payload (missing
//    extend broke validation in exit-mode conds)
// 2. u128Lt/u128Gt/u128Eq produce TAGGED BOOLS (falsy == 1, not 0) —
//    eqz-truthiness read bool-false as truthy (multisig threshold guard)

#[test]
fn eq_nil_nil_is_true() {
    // both sides nilable (bare jsonGetInt) — tagged 4 == 4
    let r = run("eqNil", "{}");
    assert!(r.value.contains("eq"), "eqNil: {}", r.value);
}

#[test]
fn u128_bool_false_is_falsy() {
    // u128Lt("1","2") is TRUE (abort taken); u128Lt("2","2") is FALSE
    // (abort skipped) — a bool-false read as truthy inverts both
    let src = r#"
export function thr(v: string): string {
  if (u128Lt(v, "2")) { return "below"; }
  return "at-or-above";
}"#;
    let ir = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("lgas2_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("lgas2_{}.bin", std::process::id()));
    for (v, want) in [("1", "below"), ("2", "at-or-above"), ("5", "at-or-above")] {
        let _ = std::fs::remove_file(&st);
        let out = std::process::Command::new("./target/release/near-mock")
            .arg("cross")
            .arg(st.to_str().unwrap())
            .arg(format!("lgas2.t.near={}", p.display()))
            .arg("lgas2.t.near")
            .arg("thr")
            .arg(format!("{{\"v\":\"{v}\"}}"))
            .output()
            .unwrap();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let val = all.lines().rev().find(|l| l.contains('📄')).unwrap_or(&all);
        assert!(val.contains(want), "thr({v}): {val}");
    }
}
