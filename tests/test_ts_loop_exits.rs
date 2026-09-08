//! Loop-exit regression battery (2026-09-08).
//!
//! The break/return flag machinery conflated `break` with `return`:
//! __wl_done was set by BOTH, but only `return` ever set __wl_res —
//! so a `break` out of a loop fell into the value-selector and yielded
//! the never-assigned __wl_res = nil. Two user-visible failures:
//!   1. `while (...) { if (m) { r = 1; break; } ... } return r;`
//!      returned NIL on match (fell through to 0 only on no-match).
//!   2. Mid-function, any statement AFTER a breaking loop was skipped
//!      (the continuation guard keyed on __wl_done, not the return flag).
//! Fix: separate __wl_ret, set only by in-loop `return`; selectors and
//! continuation guards key on __wl_ret; `break` = loop-exit only.
//!
//! This battery pins all four exit shapes: break+post-return, break
//! mid-function continuation, in-loop return, and value-position break.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
// 1. the charMatches shape: break on match, value accumulated in a var,
//    returned AFTER the loop. Bug: returned nil when a match was found.
export function scan_match(target: string, set: string, m: number): number {
  let r = 0;
  let j = 0;
  while (j < m) {
    if (target === strSlice(set, j, j + 1)) {
      r = 1;
      break;
    }
    j = j + 1;
  }
  return r;
}

// 2. mid-function continuation: statements after a breaking loop MUST run.
export function break_then_continue(n: number): number {
  let acc = 0;
  let i = 0;
  while (i < n) {
    i = i + 1;
    if (i === 3) {
      break;
    }
  }
  acc = i * 100; // must execute after the break
  return acc;
}

// 3. in-loop return still wins: value carried through __wl_res.
export function return_inside(n: number): number {
  let i = 0;
  while (i < n) {
    if (i === 2) {
      return 777;
    }
    i = i + 1;
  }
  return 1;
}

// 4. value-position loop with break (no post-loop code): undefined → 0,
//    never nil.
export function value_break(n: number): number {
  let i = 0;
  while (i < n) {
    i = i + 1;
    if (i === 2) {
      break;
    }
  }
}

// 5. for-of with break + post-loop accumulation (same machinery).
export function forof_break_post(k: string): number {
  let hit = 0;
  let items = ["a", ",", "c"];
  let acc = 0;
  for (const p of items) {
    if (p === ",") {
      hit = 1;
      break;
    }
    acc = acc + 1;
  }
  return hit * 10 + acc; // hit=1, acc=1 → 11
}

// 6. no-exit loop sanity: plain fall-through value.
export function plain_sum(n: number): number {
  let s = 0;
  let i = 0;
  while (i < n) {
    s = s + i;
    i = i + 1;
  }
  return s;
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run_num(method: &str, input: &str) -> String {
    let r = run(method, input);
    // mock annotates raw i64 returns: "300 (raw i64, untagged: 37)"
    r.split_whitespace().next().unwrap_or(&r).to_string()
}

fn run(method: &str, input: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).expect("codegen");
    let tmp = std::env::temp_dir().join(format!("nm_loopexit_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock binary");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{method}: mock failed:\n{s}");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return rest.trim_end().to_string();
        }
    }
    panic!("{method}: no result line:\n{s}");
}

#[test]
fn break_post_loop_returns_match_not_nil() {
    // match at j=0 — the exact charMatches shape that returned nil.
    assert_eq!(run_num("scan_match", r#"{"target":"a","set":"abc","m":3}"#), "1");
    // match mid-string
    assert_eq!(run_num("scan_match", r#"{"target":"b","set":"abc","m":3}"#), "1");
    // match at last position
    assert_eq!(run_num("scan_match", r#"{"target":"c","set":"abc","m":3}"#), "1");
    // no match — fall-through r=0 (was already correct pre-fix)
    assert_eq!(run_num("scan_match", r#"{"target":"z","set":"abc","m":3}"#), "0");
}

#[test]
fn mid_function_break_runs_continuation() {
    // break at i=3, then acc = 3*100 must execute → 300.
    // Pre-fix: continuation was skipped, function returned nil.
    assert_eq!(run_num("break_then_continue", r#"{"n":10}"#), "300");
    // n=2: i reaches 2, cond (2<2) fails, no break → acc=200
    assert_eq!(run_num("break_then_continue", r#"{"n":2}"#), "200");
}

#[test]
fn in_loop_return_carries_value() {
    assert_eq!(run_num("return_inside", r#"{"n":10}"#), "777");
    // return never fires → post-loop 1
    assert_eq!(run_num("return_inside", r#"{"n":1}"#), "1");
}

#[test]
fn value_position_break_is_zero_not_nil() {
    assert_eq!(run_num("value_break", r#"{"n":10}"#), "0");
}

#[test]
fn forof_break_post_code_runs() {
    assert_eq!(run_num("forof_break_post", r#"{"k":"x"}"#), "11");
}

#[test]
fn plain_loops_unchanged() {
    assert_eq!(run_num("plain_sum", r#"{"n":5}"#), "10");
}
