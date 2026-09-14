//! JSON v2 (2026-09-14) — Tier 1+2 improvements, all verified on near-mock:
//! 1. jsonGetStr(key, json) 2-arg form: scans the GIVEN buffer (was a
//!    silent footgun — compiled fine, scanned the tx input, ignored arg 2)
//! 2. Object/array spanning: {"o": {...}} returns the FULL balanced span
//!    as raw JSON text (literal path returned just "{" before — the quote
//!    branch didn't check the string flag; dynamic path already spanned)
//! 3. jsonGetInt on found-but-non-numeric → nil (?? fires; was silent 0,
//!    indistinguishable from a real zero). Prefix rule: "12x" → 12.
//! 4. jsonExtract(...keys): single-pass multi-key extraction from tx
//!    input (TS surface for the lisp json-extract/__json_extract_N
//!    machinery) — 2.3× cheaper than N individual getters (measured).
//! 5. jsonArr: nested elements (objects/arrays as full spans) + cap
//!    raised 64 → 512.
//! 6. (deferred) dynamic keys for jsonArr — needs buffer parameterization
//!    of the whole json_get_arr scanner; documented in GAPS.md.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function twoArg(): string { const o = near.jsonGetStr("outer") ?? "{}"; return near.jsonGetStr("inner", o) ?? "MISS"; }
export function objSpan(): string { return near.jsonGetStr("outer") ?? "MISS"; }
export function arrSpan(): string { return near.jsonGetStr("nums") ?? "MISS"; }
export function spanChain(): string {
  const o = near.jsonGetStr("outer") ?? "{}";
  const inner = near.jsonGetStr("inner", o) ?? "MISS";
  return inner;
}
export function intGarbage(): string { return toStr(near.jsonGetInt("g") ?? -99); }
export function intRealZero(): string { return toStr(near.jsonGetInt("z") ?? -99); }
export function intPrefix(): string { return toStr(near.jsonGetInt("px") ?? -99); }
export function intMiss(): string { return toStr(near.jsonGetInt("nope") ?? -99); }
export function extAll(): string {
  const r = jsonExtract("who", "missing", "n", "outer");
  return toStr(r.length) + ":" + r[0] + "|" + r[1] + "|" + r[2] + "|" + r[3];
}
export function arrNested(): string {
  const a = near.jsonArr("rows");
  return toStr(a.length) + ":" + a[0] + "|" + a[1];
}
export function arrFlat(): string {
  const a = near.jsonArr("ids");
  return toStr(a.length) + ":" + a[0] + "/" + a[1] + "/" + a[2];
}
export function arrStrings(): string {
  const a = near.jsonArr("names");
  return toStr(a.length) + ":" + a[0] + "/" + a[1];
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("jv2_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("jv2_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("jv2.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("jv2.t.near")
        .arg(method)
        .arg(args)
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all.lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&all)
        .to_string()
}

const ARGS: &str = r#"{"outer" : { "inner" : "deep" }, "nums" : [1, 2, 3], "g" : "abc", "z" : 0, "px" : "12x", "who": "jp", "n" : 42, "rows": [{"id": 1}, {"id": 2}], "ids": [10, 20, 30], "names": ["alice", "bob"]}"#;

#[test]
fn two_arg_form_scans_given_json() {
    let r = run("twoArg", ARGS);
    assert!(r.contains("deep"), "twoArg: {r}");
}

#[test]
fn object_value_spans() {
    let r = run("objSpan", ARGS);
    assert!(r.contains("{ \"inner\" : \"deep\" }"), "objSpan: {r}");
}

#[test]
fn array_value_spans() {
    let r = run("arrSpan", ARGS);
    assert!(r.contains("[1, 2, 3]"), "arrSpan: {r}");
}

#[test]
fn span_then_two_arg_chain() {
    let r = run("spanChain", ARGS);
    assert!(r.contains("deep"), "spanChain: {r}");
}

#[test]
fn int_non_numeric_is_nil() {
    let r = run("intGarbage", ARGS);
    assert!(r.contains("-99"), "intGarbage: {r}");
}

#[test]
fn int_real_zero_survives() {
    let r = run("intRealZero", ARGS);
    assert!(r.contains("0"), "intRealZero: {r}");
    assert!(!r.contains("-99"), "intRealZero must not be nil: {r}");
}

#[test]
fn int_prefix_digits_parse() {
    let r = run("intPrefix", ARGS);
    assert!(r.contains("12"), "intPrefix: {r}");
}

#[test]
fn int_missing_is_nil() {
    let r = run("intMiss", ARGS);
    assert!(r.contains("-99"), "intMiss: {r}");
}

#[test]
fn extract_single_pass_all_shapes() {
    let r = run("extAll", ARGS);
    assert!(
        r.contains("4:jp||42|{ \"inner\" : \"deep\" }"),
        "extAll: {r}"
    );
}

#[test]
fn arr_nested_objects() {
    let r = run("arrNested", ARGS);
    assert!(r.contains("2:{\"id\": 1}|{\"id\": 2}"), "arrNested: {r}");
}

#[test]
fn arr_flat_numbers_regression() {
    let r = run("arrFlat", ARGS);
    assert!(r.contains("3:10/20/30"), "arrFlat: {r}");
}

#[test]
fn arr_strings_regression() {
    let r = run("arrStrings", ARGS);
    assert!(r.contains("2:alice/bob"), "arrStrings: {r}");
}
