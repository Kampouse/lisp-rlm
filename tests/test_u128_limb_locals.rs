//! u128 Level 1 — limb locals (2026-09-15). Locals whose every store is
//! u128-pure (u128 arith results, ≤u128::MAX digit literals, copies of
//! other limb locals) compile to a (lo, hi) i64 pair: the parse/render
//! round-trip (~7 Ggas of every ~9 Ggas op after the chunked to_str win)
//! vanishes for values that stay in limb form.
//!
//! Measured (near-mock, exact same outputs):
//!   fib(185)  1.6 Tgas (chunked to_str alone) → ~0.1 Tgas (limb)
//!
//! Pinned semantics:
//! - mixed functions (any non-pure store) demote the name function-wide;
//! - generic reads (strcat/return/storage) materialize lazily;
//! - comparisons run on the locals directly;
//! - params stay tagged (one-i64 calling convention);
//! - overflow under try stays catchable (the _ck helper path).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function fib(): string {
  let a = 0n;
  let b = 1n;
  let i = 0;
  while (i < 185) {
    const t = a + b;
    a = b;
    b = t;
    i = i + 1;
  }
  return a;
}
export function accrue(): string {
  const bal = near.jsonGetStr("bal") ?? "123456789";
  let rps = "500000";
  let out = u128Mul(bal, rps);
  out = u128Div(out, "1000");
  return out;
}
export function demote(): string {
  let x = u128Add("1", "2");
  x = strCat("v", x);
  return x;
}
export function mat(): string {
  let x = u128Add("123", "456");
  const s = `v:${x}`;
  return s;
}
export function cmp(): string {
  let a = u128Add("1000000000000000000000", "1");
  let b = "999999999999999999999";
  if (a > b) { return "gt"; }
  return "le";
}
export function eq(): string {
  let a = u128Add("2", "3");
  let b = "5";
  if (a === b) { return "eq"; }
  return "ne";
}
export function shadowLoop(): string {
  let x = u128Add("10", "5");
  let j = 0;
  while (j < 3) {
    let x2 = u128Add(x, "1");
    x = x2;
    j = j + 1;
  }
  return x;
}
export function withParam(): string {
  const amt = near.jsonGetStr("amt") ?? "5";
  let acc = 0n;
  let i = 0;
  while (i < 10) {
    acc = u128Add(acc, amt);
    i = i + 1;
  }
  return acc;
}
export function subMul(): string {
  let a = u128Mul("18446744073709551616", "18446744073709551615");
  a = u128Sub(a, "18446744073709551615");
  return a;
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(method: &str, args: &str) -> (String, bool) {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("u128ll_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("u128ll_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("u128ll.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("u128ll.t.near")
        .arg(method)
        .arg(args)
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let val = all
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
    (format!("{val}|{gas}"), all.contains("unreachable"))
}

fn fib_u128(n: u64) -> u128 {
    let (mut a, mut b) = (0u128, 1u128);
    for _ in 0..n {
        let t = a.wrapping_add(b);
        a = b;
        b = t;
    }
    a
}

#[test]
fn fib_185_exact_and_cheap() {
    let (v, trap) = run("fib", "{}");
    let expected = fib_u128(185).to_string();
    assert!(!trap, "fib trapped: {v}");
    assert!(v.contains(&expected), "fib(185) wrong: {v} want {expected}");
    let gas: f64 = v.split('|').nth(1).unwrap().parse().unwrap();
    // 8.0 Tgas pre-chunked-to_str, 1.6 with chunked to_str alone, ~0.1
    // with limb locals. Budget with generous margin: a regression past
    // this means values are stringifying inside the loop again.
    assert!(
        gas < 0.5,
        "fib(185) gas regressed past 0.5 Tgas: {gas} (limb target ~0.1)"
    );
}

#[test]
fn mixed_generic_operand_accrues_exactly() {
    // bal is a json getter (never limb-eligible) — parsed per use, exact
    // math through limb rps/out locals: 123456789 * 500000 / 1000
    let (v, trap) = run("accrue", r#"{"bal":"123456789"}"#);
    assert!(!trap, "{v}");
    assert!(v.contains("61728394500"), "accrue wrong: {v}");
}

#[test]
fn generic_store_demotes_function_wide() {
    // x has a strcat store → tagged everywhere; the u128Add feeds a
    // tagged set! through the normal (to_str) op exit
    let (v, trap) = run("demote", "{}");
    assert!(!trap, "{v}");
    assert!(v.contains("v3"), "demote wrong: {v}");
}

#[test]
fn generic_read_materializes() {
    let (v, trap) = run("mat", "{}");
    assert!(!trap, "{v}");
    assert!(v.contains("v:579"), "materialize wrong: {v}");
}

#[test]
fn limb_comparison_direct() {
    // NOTE: number-returning exports print as raw 8-byte LE ints in
    // near-mock's 📄 line (pre-existing, on main) — assert on strings so
    // the values are observable
    let (v, trap) = run("cmp", "{}");
    assert!(!trap, "{v}");
    assert!(v.contains("gt"), "cmp wrong: {v}");
    let (v, trap) = run("eq", "{}");
    assert!(!trap, "{v}");
    assert!(v.contains("eq"), "eq wrong: {v}");
}

#[test]
fn loop_shadow_and_copy() {
    let (v, trap) = run("shadowLoop", "{}");
    assert!(!trap, "{v}");
    assert!(v.contains("18"), "shadowLoop wrong: {v}");
}

#[test]
fn param_operand_stays_correct() {
    let (v, trap) = run("withParam", r#"{"amt":"7"}"#);
    assert!(!trap, "{v}");
    assert!(v.contains("70"), "withParam wrong: {v}");
}

#[test]
fn u128_max_boundary_math() {
    // 2^64 × (2^64-1) − (2^64-1) = 2^128 − 2^65 + 1
    // = 340282366920938463426481119284349108225
    let (v, trap) = run("subMul", "{}");
    assert!(!trap, "{v}");
    assert!(
        v.contains("340282366920938463426481119284349108225"),
        "subMul wrong: {v}"
    );
}

#[test]
fn zz_print_gas_actuals() {
    // informational: fib limb-locals gas (asserted < 0.5 in the test above)
    let (v, _) = run("fib", "{}");
    eprintln!("FIB185_ACTUAL: {v}");
}
