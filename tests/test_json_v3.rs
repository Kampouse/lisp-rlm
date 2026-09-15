//! JSON API v3 (2026-09-15) — the JS-like layer.
//!
//! The oddness this replaces (pinned by the audit): `o.prop ?? fb` was a
//! COMPILE ERROR ("value is not maybe-nil" — property reads typed str with
//! the "" contract), `o.prop || fb` also errored, and the workaround was a
//! 4-line strLength check. Meanwhile near.jsonGetStr(k) ?? fb worked. Two
//! APIs, two missing-contracts, one rejected idiom.
//!
//! v3:
//! - `const o = near.input()` — input HANDLE: property reads rewrite at
//!   compile time to the cached-input getters (zero copies, shared
//!   scanner, input cache). nil-ON-MISS on handles (legacy o.k on plain
//!   strings keeps the "" contract).
//! - `o.prop ?? fb` — dispatches on the fallback's literal type:
//!   number fb → the INT getter (typed read, no strToNum ceremony),
//!   string fb → the STR getter.
//! - `o.a.b` nested — top key via the input getter, rest via the buffer
//!   dot-path scanner ("" contract; ?? on nested paths rejected with a
//!   pointer until the str-nil buffer op exists).
//! - `const {a, n} = near.args<{a: string, n: number}>()` — typed
//!   single-pass binding: one jsonExtract call for all keys, number
//!   fields wrapped str->num.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function readName(): string {
  const o = near.input();
  return o.name ?? "anon";
}
export function readAmt(): string {
  const o = near.input();
  const amt = o.amount ?? 0;
  return toStr(amt * 2);
}
export function nested(): string {
  const o = near.input();
  const u = o.user.name;
  if (strLength(u) == 0) { return "nobody"; }
  return u;
}
export function bound(): string {
  const { who, count } = near.args<{ who: string, count: number }>();
  return who + ":" + toStr(count + 1);
}
export function bareMiss(): string {
  // bare handle read on a miss yields nil — visible, not silent
  const o = near.input();
  return toStr(strLength(o.absent));
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
    let p = std::env::temp_dir().join(format!("jv3_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("jv3_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("jv3.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("jv3.t.near")
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

#[test]
fn handle_str_fallback_fires_on_miss() {
    let r = run("readName", r#"{"name": "jp"}"#);
    assert!(r.contains("jp"), "hit: {r}");
    let r = run("readName", "{}");
    assert!(r.contains("anon"), "miss fires ???: {r}");
}

#[test]
fn handle_number_fallback_is_typed_read() {
    let r = run("readAmt", r#"{"amount" : 21}"#);
    assert!(r.contains("42"), "typed numeric ?? — no strToNum: {r}");
    let r = run("readAmt", "{}");
    assert!(r.contains("0"), "numeric miss → 0 fallback: {r}");
}

#[test]
fn handle_nested_path_reads() {
    let r = run("nested", r#"{"user" : { "name" : "bob" }}"#);
    assert!(r.contains("bob"), "nested hit: {r}");
    let r = run("nested", "{}");
    assert!(r.contains("nobody"), "nested miss handled by guard: {r}");
}

#[test]
fn args_destructuring_typed_single_pass() {
    let r = run("bound", r#"{"who": "wd", "count" : 41}"#);
    assert!(r.contains("wd:42"), "typed fields: {r}");
    // missing fields: str→"", num→0 (extract spans are ""-on-miss)
    let r = run("bound", "{}");
    assert!(r.contains(":1"), "missing → defaults: {r}");
}

#[test]
fn bare_handle_read_miss_is_nil_not_empty_string() {
    // strLength(nil) == 0 — but the SEMANTIC pin: the read itself is nil
    // (renders "nil" in concat). The ?? layer is the sanctioned access.
    let r = run("bareMiss", r#"{"x" : 1}"#);
    assert!(r.contains("0"), "strLength of nil-miss: {r}");
}
