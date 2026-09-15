//! u128 chunked to_str (2026-09-15) — the serialize was ~95% of every u128
//! op's gas. The old __h_u128_to_str divided by 10 via 128-step binary long
//! division PER DECIMAL DIGIT (39 × 128 × ~15 ≈ 75k instrs ≈ 75 Ggas;
//! measured acc(100) at 8.04 Tgas ≈ 80 Ggas per u128Add). The rewrite
//! divides by 10^18 per chunk (10^18 fits SIGNED i64 — 10^19 overflows
//! i64::MAX and would break i64 div/rem on the chunk): ≤3 chunks for a
//! 39-digit value, one 128-step division each + cheap i64 digit
//! formatting. ~700 instrs total.
//!
//! Measured (near-mock, exact same outputs):
//!   acc(100)  8.036 → 1.015 Tgas  (-87%)
//!   paramReuse (3 adds, 30-digit) 0.341 → 0.049 (-86%)
//!   chain2 (mul+add) 0.264 → 0.025 (-91%)
//!   per-u128Add ≈ 80 Ggas → ≈ 9 Ggas
//!
//! The padding rule is the subtle part (pinned here): a chunk is
//! zero-padded to 18 digits iff the quotient after its division is
//! nonzero — the 10^36 value (two full zero chunks) and the 10^18 ± 1
//! boundaries are the kill cases for naive implementations.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function max(): string { return u128Add("340282366920938463463374607431768211454", "0"); }
export function chunkEdge(): string { return u128Add("1000000000000000000", "1"); }
export function chunkEdge2(): string { return u128Add("999999999999999999", "1"); }
export function interiorZeros(): string { return u128Mul("1000000000000000000", "1000000000000000000"); }
export function threeChunks(): string { return u128Add("100000000000000000000000000000000000000", "1"); }
export function subMax(): string { return u128Sub("340282366920938463463374607431768211455", "1"); }
export function mulBig(): string { return u128Mul("18446744073709551616", "18446744073709551615"); }
export function divBig(): string { return u128Div("340282366920938463463374607431768211455", "123456789"); }
export function acc(n: number): string {
  let a = "0";
  const delta = "1000000000000000000000";
  for (let i = 0; i < n; i = i + 1) { a = u128Add(a, delta); }
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
    let p = std::env::temp_dir().join(format!("u128c_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("u128c_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("u128c.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("u128c.t.near")
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

#[test]
fn chunk_boundaries_exact() {
    // 10^18 ± 1: the chunk-boundary padding kill cases
    let (v, _) = run("chunkEdge", "{}");
    assert!(v.contains("1000000000000000001"), "{v}");
    let (v, _) = run("chunkEdge2", "{}");
    assert!(v.contains("1000000000000000000"), "{v}");
}

#[test]
fn interior_zero_chunks_exact() {
    // 10^36 = two full zero chunks between the leading 1 and trailing 0
    let (v, _) = run("interiorZeros", "{}");
    assert!(v.contains("1000000000000000000000000000000000000"), "{v}");
    // 39-digit + 1: three chunks with the top chunk being "100"
    let (v, _) = run("threeChunks", "{}");
    assert!(v.contains("100000000000000000000000000000000000001"), "{v}");
}

#[test]
fn extremes_exact() {
    let (v, _) = run("max", "{}");
    assert!(v.contains("340282366920938463463374607431768211454"), "{v}");
    let (v, _) = run("subMax", "{}");
    assert!(v.contains("340282366920938463463374607431768211454"), "{v}");
    // 2^64 × (2^64 - 1) = 2^128 - 2^64 — the largest product of two
    // 2^64-range factors that still fits u128 (2^64 × 2^64 itself is
    // u128_max + 1 → legitimate overflow trap)
    let (v, _) = run("mulBig", "{}");
    assert!(v.contains("340282366920938463444927863358058659840"), "{v}");
    let (v, _) = run("divBig", "{}");
    assert!(v.contains("2756287197141815048043851257396"), "{v}");
}

#[test]
fn accumulator_gas_under_budget() {
    // 100-iteration u128 accumulator: 8.04 Tgas before chunked to_str,
    // 1.02 after. Budget with margin — a regression past this means the
    // per-digit division path came back.
    let (v, _) = run("acc", r#"{"n":100}"#);
    assert!(v.contains("100000000000000000000000"), "{v}");
    let gas: f64 = v.split('|').nth(1).unwrap().parse().unwrap();
    assert!(
        gas < 1.6,
        "acc(100) gas regressed past 1.6 Tgas: {gas} (pre-fix 8.04, post-fix 1.02)"
    );
}
