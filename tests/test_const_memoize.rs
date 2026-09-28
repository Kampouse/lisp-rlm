//! Top-level value defines are memoized (2026-09-14). `(define K <expr>)`
//! used to re-run its initializer on EVERY reference (each reference calls
//! the synthesized 0-param fn) — array constants re-allocated per access
//! (the 09-12 Poseidon RP-literal heap-exhaustion trap was this class),
//! and the wasm path diverged from the interpreter (letrec value binding:
//! evaluated once) and from JS module-const semantics.
//!
//! Fix: the 0-param fn gets a guard — unique zeroed data-section slot;
//! 0 → evaluate + cache, non-zero → cached tagged value. Memory is
//! re-initialized per NEAR transaction, so the cache lives exactly one
//! tx. Degenerate: tagged Num(0) == 0 never caches (re-evaluates, cheap).
//!
//! These tests pin VALUES (the perf property — one allocation — is what
//! makes the 256-element loop test survive at all; pre-fix it grew the
//! heap 1024×).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
const SEED = [1, 2, 3];
const DERIVED = 6 * 7;
const MIXED = SEED[0] + SEED[1] + SEED[2];
const NAME = "abc";
export function tseed(i: number): string { return toStr(SEED[i]); }
export function tderived(): string { return toStr(DERIVED); }
export function tmixed(): string { return toStr(MIXED); }
export function tboth(): string { return toStr(MIXED + DERIVED); }
export function tname(): string { return NAME + toStr(strLength(NAME)); }
export function hotLoop(): string {
  let s = 0;
  for (let i = 0; i < 4; i = i + 1) {
    for (let j = 0; j < 3; j = j + 1) { s = s + SEED[j]; }
  }
  return toStr(s);
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
    let p = std::env::temp_dir().join(format!("memo_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("memo_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("memo.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("memo.t.near")
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
fn array_const_indexed() {
    let r = run("tseed", r#"{"i":0}"#);
    assert!(r.contains("1"), "SEED[0]: {r}");
    let r = run("tseed", r#"{"i":2}"#);
    assert!(r.contains("3"), "SEED[2]: {r}");
}

#[test]
fn scalar_const() {
    let r = run("tderived", "{}");
    assert!(r.contains("42"), "DERIVED: {r}");
}

#[test]
fn const_referencing_const() {
    let r = run("tmixed", "{}");
    assert!(r.contains("6"), "MIXED: {r}");
}

#[test]
fn two_consts_in_one_expression() {
    // slots must be UNIQUE (alloc_data dedupe aliased every const to one
    // slot — MIXED+DERIVED returned 6+6 before the unique-slot fix)
    let r = run("tboth", "{}");
    assert!(r.contains("48"), "MIXED+DERIVED: {r}");
}

#[test]
fn string_const() {
    let r = run("tname", "{}");
    assert!(r.contains("abc3"), "NAME+len: {r}");
}

#[test]
fn array_const_in_hot_loop() {
    // 12 SEED references in one call — pre-fix this re-allocated the
    // array on every access; the 256-element version of this trap
    // exhausted the heap. Value pins the semantics; survival pins the
    // single-allocation behavior.
    let r = run("hotLoop", "{}");
    assert!(r.contains("24"), "hotLoop: {r}");
}
