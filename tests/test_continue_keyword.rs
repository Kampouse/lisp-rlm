//! `continue` in while/for/for-of loops (2026-09-13) + two pre-existing
//! loop-exit bugs fixed along the way:
//!   1. `while (true)` with exits never compiled — the flag-guarded cond
//!      paired an int literal test (`true` → 1) with a bool false branch.
//!      false now type-matches the test (int 0 vs bool (= 1 0)).
//!   2. `while (true) { if (...) return; <more stmts> }` as the function's
//!      LAST statement never compiled — seen_fn_exit emitted __fn_done
//!      guards without the fn flags being bound. Guards now gate on
//!      fn_bound (an unbound-context return sets __wl_done, which the
//!      plain exit guard already honors).
//!
//! Design: `continue` sets __wl_done (kills the rest of the iteration —
//! the existing guards skip the tail) but NOT __wl_brk (the loop-exit
//! flag the cond checks). The body start re-arms __wl_done each iteration
//! so a conditional continue doesn't trip the guards forever. For-loop
//! update clauses (i++) guard on __wl_brk so they still run after a
//! continue (break/return skip them, as before).

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

fn run_src(src: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("cont_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("cont_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("cont.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("cont.t.near")
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

const WHILE: &str = r#"
export function odds(): string {
  let s = 0;
  let i = 0;
  while (i < 10) {
    i = i + 1;
    if (i % 2 === 0) { continue; }
    s = s + i;
  }
  return toStr(s); // 25
}
"#;

const FOR: &str = r#"
export function odds(): string {
  let s = 0;
  for (let i = 0; i < 10; i++) {
    if (i % 2 === 0) { continue; }
    s = s + i;
  }
  return toStr(s); // 25
}
"#;

const FOR_OF: &str = r#"
export function skipSome(): string {
  let s = 0;
  const xs = [1, 2, 3, 4, 5];
  for (const x of xs) {
    if (x === 2 || x === 4) { continue; }
    s = s + x;
  }
  return toStr(s); // 9
}
"#;

const TAIL: &str = r#"
export function tailRuns(): string {
  let s = 0;
  let i = 0;
  while (i < 6) {
    i = i + 1;
    if (i === 3) { continue; }
    s = s + 100; // runs for i = 1,2,4,5,6
  }
  return toStr(s); // 500
}
"#;

const BRK: &str = r#"
export function withBreak(): string {
  let s = 0;
  let i = 0;
  while (true) {
    i = i + 1;
    if (i > 10) { break; }
    if (i % 2 === 0) { continue; }
    s = s + i;
  }
  return toStr(s); // 25
}
"#;

const RET: &str = r#"
export function withReturn(): string {
  let s = 0;
  let i = 0;
  while (true) {
    i = i + 1;
    if (i > 8) { return toStr(s); }
    if (i % 2 === 0) { continue; }
    s = s + i;
  }
}
"#;

#[test]
fn continue_in_while() {
    let r = run_src(WHILE, "odds", "{}");
    assert!(r.contains("25"), "while: {r}");
}

#[test]
fn continue_in_for() {
    let r = run_src(FOR, "odds", "{}");
    assert!(r.contains("25"), "for: {r}");
}

#[test]
fn continue_in_for_of() {
    let r = run_src(FOR_OF, "skipSome", "{}");
    assert!(r.contains("9"), "for-of: {r}");
}

#[test]
fn statements_after_continue_run_next_iteration() {
    let r = run_src(TAIL, "tailRuns", "{}");
    assert!(r.contains("500"), "tail: {r}");
}

#[test]
fn continue_with_break() {
    // also covers the while(true) int/bool cond fix
    let r = run_src(BRK, "withBreak", "{}");
    assert!(r.contains("25"), "brk: {r}");
}

#[test]
fn continue_with_return_last_stmt() {
    // also covers the unbound __fn_done fix (return as last statement)
    let r = run_src(RET, "withReturn", "{}");
    assert!(r.contains("16"), "ret: {r}");
}
