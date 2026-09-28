//! Loop-body/if-branch declaration scoping (2026-09-13) — two fixes:
//!
//! 1. FRONTEND: `let j = 0;` inside for/for-of bodies and if-branches used
//!    to lower to a dead `(let ((j 0)) 0)` — the binding vanished, so any
//!    later statement in the same block referencing j (nested whiles,
//!    closures, pushes) failed with "undefined variable 'j'". While bodies
//!    already hoisted (bind nil outside, set! re-init at source position);
//!    for/for-of bodies and loop_body_expr (if branches) now do the same.
//!
//! 2. EMITTER: local slot cross-type reuse. local_idx_i32 could reuse a
//!    freed i64 slot and OVERWRITE the type map to i32 — but the previous
//!    tenant's already-emitted i64 code is immutable, so the final locals
//!    declaration (built from the FINAL type map) mismatched: the whole
//!    module failed wasmtime validation. Found via vec-push inside a
//!    for-loop (the let-bound loop var's freed i64 slot became an i32
//!    vec-nth pointer). Slots now keep their first-allocated type; cross-
//!    type reuse leaks the slot instead of invalidating the module.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(src: &str, method: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("scope_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("scope_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("scope.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("scope.t.near")
        .arg(method)
        .arg("{}")
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

const FOR_OF_DECL: &str = r#"
export function t(): string {
  let s = "";
  const xs = ["x", "y"];
  for (const x of xs) {
    let i = 0;
    while (i < 2) {
      s = s + x;
      i = i + 1;
    }
  }
  return s; // "xxyy"
}
"#;

const FOR_DECL: &str = r#"
export function t(): string {
  let s = "";
  for (let k = 0; k < 2; k++) {
    let j = 0;
    while (j < 2) {
      s = s + "x";
      j = j + 1;
    }
  }
  return s; // "xxxx"
}
"#;

const IF_BRANCH_DECL: &str = r#"
export function t(): string {
  let s = "";
  let k = 0;
  while (k < 2) {
    if (k === 0) {
      let j = 0;
      while (j < 2) {
        s = s + "x";
        j = j + 1;
      }
    }
    k = k + 1;
  }
  return s; // "xx"
}
"#;

const ELSE_BRANCH_DECL: &str = r#"
export function t(): string {
  let s = "";
  let k = 0;
  while (k < 4) {
    if (k % 2 === 0) {
      const tag = "e";
      s = s + tag;
    } else {
      const tag = "o";
      s = s + tag;
    }
    k = k + 1;
  }
  return s; // "eoeo"
}
"#;

const DECL_WITH_CONTINUE: &str = r#"
export function t(): string {
  let s = "";
  const xs = ["a", "b", "c"];
  for (const x of xs) {
    const up = x + "!";
    if (x === "b") { continue; }
    s = s + up;
  }
  return s; // "a!c!"
}
"#;

const VEC_PUSH_IN_FOR: &str = r#"
export function t(): string {
  const out: string[] = [];
  for (let k = 0; k < 3; k++) {
    const d = k * 2;
    if (k === 1) {
      out.push(toStr(d));
    }
  }
  return out[0]; // "2"
}
"#;

const VEC_PUSH_PLAIN_FOR: &str = r#"
export function t(): string {
  const out: string[] = [];
  for (let k = 0; k < 3; k++) {
    out.push(toStr(k));
  }
  return out[0]; // "0"
}
"#;

#[test]
fn for_of_body_decl_visible_to_nested_while() {
    let r = run(FOR_OF_DECL, "t");
    assert!(r.contains("xxyy"), "for-of: {r}");
}

#[test]
fn for_body_decl_visible_to_nested_while() {
    let r = run(FOR_DECL, "t");
    assert!(r.contains("xxxx"), "for: {r}");
}

#[test]
fn if_branch_decl_visible_in_branch() {
    let r = run(IF_BRANCH_DECL, "t");
    assert!(r.contains("xx"), "if-branch: {r}");
}

#[test]
fn else_branch_decl_visible_in_branch() {
    let r = run(ELSE_BRANCH_DECL, "t");
    assert!(r.contains("eoeo"), "else-branch: {r}");
}

#[test]
fn decl_with_continue_in_body() {
    let r = run(DECL_WITH_CONTINUE, "t");
    assert!(r.contains("a!c!"), "continue: {r}");
}

#[test]
fn vec_push_with_decl_in_for_body() {
    let r = run(VEC_PUSH_IN_FOR, "t");
    assert!(r.contains("2"), "vec-push+decl: {r}");
}

#[test]
fn vec_push_plain_in_for_body() {
    // regression for the emitter slot-type fix alone (no declarations)
    let r = run(VEC_PUSH_PLAIN_FOR, "t");
    assert!(r.contains("0"), "vec-push plain: {r}");
}
