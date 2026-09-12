//! BN254 field multiply — 16-limb CIOS Montgomery, ALL state in native
//! int arrays + locals (zero storage writes in the hot path).
//!
//! out = a·b·R⁻¹ mod p — feed Montgomery-form operands for field-mul.
//! Vectors (python-verified against a·b·R⁻¹ mod p):
//!   identity    : 1m × 1m = 1m
//!   pm1_squared : (p-1)m × (p-1)m = 1m   — (p-1)² = 1 in M-domain
//!   raw_2x3     : 2 × 3 → 6·R⁻¹ mod p
//!   big_rand    : 254-bit pseudo-random operands (seed 254)
//!   zero        : 0 × x = 0
//!
//! GAS (near-mock, 2026-09-11): ~0.0033 Tgas per mul vs 254 Tgas for the
//! storage-slot version — ~76,000× cheaper, comfortably on-chain. The
//! 1-Tgas ceiling assertion below is a storage-regression guard: any
//! storage write reintroduced into the loop blows straight past it.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const FP: &str = include_str!("../fixtures/fp254_cios.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn call(state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(FP).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("fp254_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("fp254.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("fp254.t.near")
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
fn cios_identity() {
    let st = "/tmp/fp254-cios.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594","b":"3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594"}"#;
    let r = call(st, "mulmod", &args);
    assert!(r.contains("📄 3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594"), "identity: {r}");
    // storage-regression guard: compute-only mul must stay far below 1 Tgas
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt < 1.0, "identity gas regression: {line}");
    }
}

#[test]
fn cios_pm1_squared() {
    let st = "/tmp/fp254-cios.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"61354,4845,18569,26819,48975,29354,32616,36104,4657,2312,41543,20961,49402,18217,54961,8793","b":"61354,4845,18569,26819,48975,29354,32616,36104,4657,2312,41543,20961,49402,18217,54961,8793"}"#;
    let r = call(st, "mulmod", &args);
    assert!(r.contains("📄 3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594"), "pm1_squared: {r}");
    // storage-regression guard: compute-only mul must stay far below 1 Tgas
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt < 1.0, "pm1_squared gas regression: {line}");
    }
}

#[test]
fn cios_raw_2x3() {
    let st = "/tmp/fp254-cios.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0","b":"3,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0"}"#;
    let r = call(st, "mulmod", &args);
    assert!(r.contains("📄 60135,52560,30025,25720,40723,1180,47687,36153,46919,50196,7757,17094,33930,46442,63593,9332"), "raw_2x3: {r}");
    // storage-regression guard: compute-only mul must stay far below 1 Tgas
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt < 1.0, "raw_2x3 gas regression: {line}");
    }
}

#[test]
fn cios_big_rand() {
    let st = "/tmp/fp254-cios.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"9094,24563,49313,32832,39577,16446,53319,26874,51112,43384,49061,22278,55689,14243,4920,58","b":"9062,47063,22385,32787,30842,46399,46914,63968,25388,21263,2905,48010,65058,31722,1894,11073"}"#;
    let r = call(st, "mulmod", &args);
    assert!(r.contains("📄 42441,45104,47084,611,18988,22481,53354,49820,36939,33998,59317,55140,34935,11805,29936,3756"), "big_rand: {r}");
    // storage-regression guard: compute-only mul must stay far below 1 Tgas
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt < 1.0, "big_rand gas regression: {line}");
    }
}

#[test]
fn cios_zero() {
    let st = "/tmp/fp254-cios.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0","b":"9062,47063,22385,32787,30842,46399,46914,63968,25388,21263,2905,48010,65058,31722,1894,11073"}"#;
    let r = call(st, "mulmod", &args);
    assert!(r.contains("📄 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0"), "zero: {r}");
    // storage-regression guard: compute-only mul must stay far below 1 Tgas
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt < 1.0, "zero gas regression: {line}");
    }
}

// ── array-aliasing regression (2026-09-11): array literals used fixed
// compile-time addresses — ciosMul(a, ciosMul(a, b)) aliased the two t[]
// blocks and silently zeroed its own input. Fixed by runtime-heap alloc;
// verified on-chain (fp254.registry-nostrgov.testnet). ──
#[test]
fn cios_mul_twice_big() {
    let st = "/tmp/fp254-cios-mt.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"9094,24563,49313,32832,39577,16446,53319,26874,51112,43384,49061,22278,55689,14243,4920,58","b":"9062,47063,22385,32787,30842,46399,46914,63968,25388,21263,2905,48010,65058,31722,1894,11073"}"#;
    let r = call(st, "mulTwice", &args);
    assert!(r.contains("📄 27694,7888,49106,29860,42354,47780,37256,7429,13619,31661,19243,55295,58146,63260,4847,1144"), "mulTwice big: {r}");
}

#[test]
fn cios_roundtrip() {
    let st = "/tmp/fp254-cios-rt.bin";
    let _ = std::fs::remove_file(st);
    let args = r#"{"a":"3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594","b":"3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594"}"#;
    let r = call(st, "roundtrip", &args);
    assert!(r.contains("📄 3485,50575,17293,54109,2877,62919,60200,2680,17964,30841,41839,26222,57135,39431,30657,3594"), "roundtrip: {r}");
}
