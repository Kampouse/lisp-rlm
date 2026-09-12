
//! Compiler regression tests for the two g16v-verifier bugs (2026-09-12):
//!
//! 1. `+` concat dispatch missed string-valued identifiers:
//!    - top-level `const K = "..."` (CONST_FOLDS) — the dispatch ran before
//!      fold-substitution and saw a bare Identifier → numeric + → decimal
//!      garbage (multiexp buffer 198B instead of 288B)
//!    - locals seeded by `(storageGet(k) ?? "")` — expr_is_stringy didn't
//!      look through parens or recognize nullish-with-string-fallback
//! 2. M2 with_return treated VariableDeclarations as pure bindings — an
//!    initializer containing host calls (storage writes) EXECUTED even
//!    after an early return fired → post-return state corruption.
//!    Fix: expr_has_call initializers hoist to nil + guarded set!.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const CONCAT_SRC: &str = r###"
const ONE_HEX: string = "0100000000000000000000000000000000000000000000000000000000000000";
const LABEL: string = "mx:";

export function concat(): string {
  const base = near.storageGet("some:key") ?? "";
  const a = base + ONE_HEX;
  const b = "lit" + ONE_HEX;
  const c = LABEL + ONE_HEX;
  return `${strLength(a)}:${strLength(b)}:${strLength(c)}`;
}
"###;

const M2_SRC: &str = r###"
export function repro(): string {
  const a = near.jsonGetInt("a") ?? 0;
  if (a < 0) { return "EARLY"; }
  const b = writeAndReturn(a);
  return `LATE:${b}`;
}
function writeAndReturn(v: number): number {
  near.storageSet("se", `${v}`);
  return v * 2;
}
export function checkSE(): string {
  return near.storageGet("se") ?? "never";
}
"###;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn call(src: &str, state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("cmpfix_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("cmpfix.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("cmpfix.t.near")
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
fn plus_dispatch_string_consts_and_nullish() {
    let st = "/tmp/cmpfix-concat.bin";
    let _ = std::fs::remove_file(st);
    // base = "" (missing key) + 64-char hex = 64; "lit"+64 = 67; "mx:"+64 = 67
    let r = call(CONCAT_SRC, st, "concat", "{}");
    assert!(r.contains("64:67:67"), "concat dispatch: {r}");
}

#[test]
fn m2_impure_declaration_guarded_after_return() {
    let st = "/tmp/cmpfix-m2.bin";
    let _ = std::fs::remove_file(st);
    // early return: the storage write inside writeAndReturn must NOT run
    let r = call(M2_SRC, st, "repro", r#"{"a":-5}"#);
    assert!(r.contains("EARLY"), "early: {r}");
    let r = call(M2_SRC, st, "checkSE", "{}");
    assert!(r.contains("never"), "side effect must not run after return: {r}");

    // normal path: the write runs
    let st2 = "/tmp/cmpfix-m2b.bin";
    let _ = std::fs::remove_file(st2);
    let r = call(M2_SRC, st2, "repro", r#"{"a":5}"#);
    assert!(r.contains("LATE:10"), "normal: {r}");
    let r = call(M2_SRC, st2, "checkSE", "{}");
    assert!(r.contains("5"), "side effect must run on the normal path: {r}");
}
