//! Regression: `if` with a missing `else` must NOT lower to a numeric 0
//! value (2026-09-17). `if (c) { return s; }` used to emit `(if c <str> 0)`
//! and the checker rejected it — "if: branch types disagree — str ≠ int" —
//! a false compile error on legal TypeScript (the branch just falls
//! through). Found by the cfg differential fuzz corpus (20/60 files
//! rejected). Fix: the missing-else hole lowers to nil (bottom type —
//! unify unifies it with anything, same idiom as the __fn_res init) in
//! BOTH lower_tail_stmt's if-arm and the mid-function __fn_res capture
//! arm. Also pins runtime semantics: the early return still wins.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function tailIf(n: number): string {
  let s = "";
  if (n == 0) { s = s + "a"; } else { if (n > 5) { return s + "!"; } }
  return s + "/x";
}
export function nestedIf(n: number): string {
  let s = "";
  if (n == 0) { if (n > -5) { return s + "!"; } } else { s = s + "a"; }
  return s + "/x";
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
    let ir = ts_to_lisp_source(SRC).expect("ts_to_lisp_source");
    let exprs = parse_all(&ir).expect("parse_all");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("type_check: if-without-else must not be a branch-type error");
    let wasm = compile_near_from_exprs(&exprs).expect("compile_near_from_exprs");
    let p = std::env::temp_dir().join(format!("ifne_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("ifne_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("ifne.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("ifne.t.near")
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
fn if_no_else_compiles_and_runs() {
    // Early-return branch wins (pre-fix this file didn't even compile)
    let r = run("tailIf", r#"{"n":9}"#);
    assert!(r.contains("!"), "tailIf early return: {r}");
    // Fall-through branch produces the tail value
    let r = run("tailIf", r#"{"n":0}"#);
    assert!(r.contains("a/x"), "tailIf fall-through: {r}");
}

#[test]
fn nested_else_if_no_else_compiles_and_runs() {
    // The `else { if (c) { return v; } }` shape goes through the
    // __fn_res capture arm's own None hole (second patch site).
    // n=9: outer test false → else branch "a", tail "/x". n=0: inner
    // (always-true) test fires the early return "!".
    let r = run("nestedIf", r#"{"n":9}"#);
    assert!(r.contains("a/x"), "nestedIf fall-through: {r}");
    let r = run("nestedIf", r#"{"n":0}"#);
    assert!(r.contains("!"), "nestedIf early return: {r}");
}
