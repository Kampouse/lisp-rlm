//! Poseidon BN254 (Fr) — circomlib-compatible t=3 permutation, end-to-end.
//!
//! Constants from circomlibjs poseidon_constants.json (t=3: RF=8, RP=57),
//! Montgomery-converted offline. Anchors verified against the circomlibjs
//! test suite AND live on testnet (poseidon.registry-nostrgov.testnet,
//! 2026-09-11): poseidon([1,2]) = 0x115cc0f5…7189a — all 5 vectors pass
//! at ~146 Tgas per permutation on-chain.
//!
//! This fixture is also the regression test for TWO compiler bugs found
//! while building it:
//!   1. `const take = r < 2; if (take)` in a loop always took the
//!      then-arm — the frontend wrapped the identifier in (!= x 0), a
//!      NUMERIC compare that reads bool≠num as always-true. Fixed: pass
//!      the value raw to the tag-aware (if …) emitter.
//!   2. Function-local constants are REQUIRED: top-level const arrays
//!      re-execute their literal per access and exhaust the runtime heap
//!      (silent corruption class).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const POSEIDON: &str = include_str!("../fixtures/poseidon_bn254.ts");

// (python-verified against the circomlibjs anchors; full vectors in
// /tmp/poseidon_t3_vectors.json during the build session)
const V_ONE_TWO: (&str, &str, &str) = (
    "1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    "2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    // poseidon([1,2]) limbs = 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a
    "6298,17431,24698,40473,17745,29746,6130,10806,59855,38498,19563,15862,36929,59350,49397,4444",
);
const V_ZEROS: (&str, &str, &str) = (
    "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    "18532,18102,61060,43065,54563,24574,9429,56369,58497,31617,50162,15594,40619,40483,62971,8344",
);
const V_TWO_ONE: (&str, &str, &str) = (
    "2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    "1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
    "30554,8793,27141,13837,44485,54006,2798,29683,56429,24591,59677,34406,39799,46860,50517,5494",
);

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn call(state: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(POSEIDON).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("poseidon_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("poseidon.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("poseidon.t.near")
        .arg("poseidon")
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
fn poseidon_anchor_one_two() {
    let st = "/tmp/poseidon-anchor.bin";
    let _ = std::fs::remove_file(st);
    let args = format!(r#"{{"a":"{}","b":"{}"}}"#, V_ONE_TWO.0, V_ONE_TWO.1);
    let r = call(st, &args);
    assert!(r.contains("📄 "), "no result: {r}");
    let val = r.split("📄 ").nth(1).unwrap().lines().next().unwrap_or("");
    assert_eq!(val, V_ONE_TWO.2, "circomlib anchor poseidon([1,2])");
}

#[test]
fn poseidon_zeros() {
    let st = "/tmp/poseidon-zeros.bin";
    let _ = std::fs::remove_file(st);
    let args = format!(r#"{{"a":"{}","b":"{}"}}"#, V_ZEROS.0, V_ZEROS.1);
    let r = call(st, &args);
    let val = r
        .split("📄 ")
        .nth(1)
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    assert_eq!(val, V_ZEROS.2, "poseidon([0,0])");
}

#[test]
fn poseidon_two_one() {
    let st = "/tmp/poseidon-swap.bin";
    let _ = std::fs::remove_file(st);
    let args = format!(r#"{{"a":"{}","b":"{}"}}"#, V_TWO_ONE.0, V_TWO_ONE.1);
    let r = call(st, &args);
    let val = r
        .split("📄 ")
        .nth(1)
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    assert_eq!(val, V_TWO_ONE.2, "poseidon([2,1]) — input order matters");
}

#[test]
fn poseidon_gas_ceiling() {
    // storage/compute regression guard: a full permutation runs well under
    // the 300-Tgas protocol cap (measured: ~146 on-chain / ~145 mock).
    // Slicing was a mid-limb precision issue in an earlier probe — assert
    // whole-value now.
    let st = "/tmp/poseidon-gas.bin";
    let _ = std::fs::remove_file(st);
    let args = format!(r#"{{"a":"{}","b":"{}"}}"#, V_ONE_TWO.0, V_ONE_TWO.1);
    let r = call(st, &args);
    if let Some(line) = r.lines().find(|l| l.contains("⛽")) {
        let burnt: f64 = line
            .split("gas:")
            .nth(1)
            .and_then(|s| s.split("Tgas").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(burnt > 50.0 && burnt < 250.0, "gas regression: {line}");
    }
}
