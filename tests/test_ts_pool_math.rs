//! Pool math + u128 dispatch regressions (2026-09-27, lisp-rlm launchpad).
//!
//! Two compiler bugs were found building the launchpad pool (both invisible
//! in near-mock single-wasm mode until these exact shapes ran):
//!
//! 1. `!u128IsZero(x)` — statically_bool listed u128/gt|lt|gte|lte|eq but NOT
//!    u128/is-zero, so `!` took the (= x 0) path: a TAG_BOOL compared against
//!    TAG_NUM 0 never matches → the branch never fired (ft_resolve_transfer
//!    silently skipped its credit/refund writes; live receipts logged correct
//!    values but persisted nothing).
//! 2. `out = out + u128Div(...)` with `let out = ""` — the `+` u128 dispatch
//!    (stringy_nonnumeric) had no Identifier arm: a string local fell through
//!    to u128/add → parse("") trap. Workaround shape in pool.ts binds call
//!    results to locals first; the fix routes string locals (that are not
//!    bigint locals) to str-cat.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
function trimZeros(s: string): string {
  let i = 0;
  while (i < strLength(s) - 1) {
    if (strSlice(s, i, i + 1) == "0") { i = i + 1; } else { break; }
  }
  return strSlice(s, i, strLength(s));
}

export function notIsZero(): string {
  const used = u128Sub("1000000000000000000000000000", "0");
  if (!u128IsZero(used)) { return "NZ:" + used; }
  return "Z";
}

export function isZero(): string {
  const used = u128Sub("5", "5");
  if (!u128IsZero(used)) { return "NZ"; }
  return "Z";
}

// string-local accumulator fed by u128 call results — the exact shape that
// trapped before the stringy_nonnumeric identifier fix
function bigDiv(x: string, y: string): string {
  let out = "";
  let rem = "0";
  let i = 0;
  const n = strLength(x);
  while (i < n) {
    const r10 = u128Mul(rem, "10");
    const cur = u128Add(r10, strSlice(x, i, i + 1));
    const q = u128Div(cur, y);
    rem = u128Mod(cur, y);
    out = out + q;
    i = i + 1;
  }
  if (strLength(out) == 0) { return "0"; }
  return trimZeros(out);
}

export function dd(): string {
  const x = "100000000000000000000000000000000000000000000000000";
  return bigDiv(x, "1100000000000000000000000");
}

// bigint local + u128 op must stay ARITHMETIC (the exclusion half of the fix)
export function accAdd(): string {
  let acc = "0";
  acc = acc + u128Add(acc, "5");
  return acc;
}
"#;

fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let m = LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn run(method: &str, input: &str) -> String {
    let _g = state_lock();
    let ir = ts_to_lisp_source(SRC).unwrap_or_else(|e| panic!("lowering: {}", e));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {}", e));
    let tmp = std::env::temp_dir().join(format!("nm_poolmath_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .arg("--state")
        .arg(std::env::temp_dir().join(format!("nm_poolmath_{}.bin", std::process::id())))
        .output()
        .expect("near-mock");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in combined.lines() {
        if let Some(v) = line.strip_prefix("📄 ") {
            return v.trim().to_string();
        }
    }
    panic!("no result line for {method}: {combined}");
}

#[test]
fn not_u128_is_zero_branches_on_nonzero() {
    // before the statically_bool fix this returned "Z" (branch never fired)
    assert_eq!(run("notIsZero", "{}"), "NZ:1000000000000000000000000000");
}

#[test]
fn not_u128_is_zero_branches_on_zero() {
    assert_eq!(run("isZero", "{}"), "Z");
}

#[test]
fn big_div_exact_over_u128_boundary() {
    // 1e50 / 1.1e24 — numerator exceeds u128; result must be the exact floor
    // (before the stringy fix this trapped in __h_u128_parse)
    assert_eq!(run("dd", "{}"), "90909090909090909090909090");
}

#[test]
fn bigint_local_plus_u128_call_stays_arithmetic() {
    // a "0"-inited accumulator is BOTH a string local and a bigint local —
    // the bigint side must win so + remains u128 arithmetic: acc = 0 + (0+5)
    // = 5 (concat would have produced "05", the pre-fix behavior)
    assert_eq!(run("accAdd", "{}"), "5");
}
