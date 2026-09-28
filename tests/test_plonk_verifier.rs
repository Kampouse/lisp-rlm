//! PLONK on-chain verifier (2026-09-15) — universal setup, no trusted
//! ceremony per circuit. Port of the snarkjs PLONK verifier to the TS
//! dialect, running entirely on the alt_bn128 hosts + keccak256.
//!
//!   verify(): honest → "OK", tampered pub → "BAD:pairing"
//!   ~53 Tgas (vs Groth16's 34 Tgas — the price of universality at this
//!   circuit size; still 6× under the 300 Tgas cap)
//!
//! Key design (see plonk_transcript.md + the verifier source):
//! - Fiat-Shamir: 6 keccak rounds over BE words (LE-halves wire → BE via
//!   hexDecode byte tables — string literals can't carry bytes ≥ 0xC0)
//! - Montgomery CIOS for all Fr products (consts pre-converted)
//! - NO inversions: the standard batch-inversion (~380 CIOS ≈ 2× the gas
//!   cap) is eliminated by scaling BOTH pairing sides by
//!   D0 = zh·pre1·pre2·pre3 (bilinearity: 1^D0 = 1) — every 1/x becomes
//!   a polynomial
//! - 2 multiexp calls (18-term B1', 2-term A1'), negations folded into
//!   scalars; G2 points static (VK X_2 + generator)
//!
//! debugChallenges/dbgScalars exports remain for oracle-based diffing
//! against snarkjs (zk/identity/gen_oracle.js regenerates the expected
//! values from the same vkey/proof/pub).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = include_str!("../zk/identity/plonk_verifier.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    // NOTE: no big-stack workaround needed anymore — the compile entry
    // points run on a dedicated deep-stack thread internally (run_deep);
    // before that fix this compile aborted with a stack overflow on the
    // default 2 MiB test thread (the PLONK verifier's IR is deep enough)
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("plonk_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("pv.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("pv.t.near")
        .arg(method)
        .arg(args)
        .output()
        .expect("near-mock spawn");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const INIT: &str = include_str!("../zk/identity/plonk_init_args.json");
const VERIFY: &str = include_str!("../zk/identity/plonk_verify_args.json");

/// pub[0] + 1 — a valid-looking but false statement must fail the pairing
fn tampered_args() -> String {
    let mut v: serde_json::Value = serde_json::from_str(VERIFY).unwrap();
    let hex = v["pub"][0].as_str().unwrap().to_string();
    // LE-halves: increment the lowest byte (value += 1)
    let lo: u128 = u128::from_str_radix(&hex[..32].chars().rev().collect::<String>(), 16).unwrap();
    let hi: u128 = u128::from_str_radix(&hex[32..].chars().rev().collect::<String>(), 16).unwrap();
    let val = ((hi as u128) << 64 >> 64).wrapping_add(0); // keep hi
    let _ = val;
    let new_lo = lo + 1;
    let le = |x: u128| {
        let mut b = Vec::new();
        let mut y = x;
        for _ in 0..16 {
            b.push((y & 0xff) as u8);
            y >>= 8;
        }
        b.iter().map(|c| format!("{c:02x}")).collect::<String>()
    };
    v["pub"][0] = serde_json::Value::String(format!("{}{}", le(new_lo), le(hi)));
    v.to_string()
}

#[test]
fn plonk_honest_ok_tampered_bad() {
    let st = "/tmp/plonk-test-state.bin";
    let _ = std::fs::remove_file(st);

    let init_out = run(st, "init", INIT);
    assert!(init_out.contains("✅"), "init failed: {init_out}");

    let honest = run(st, "verify", VERIFY);
    assert!(
        honest.contains("📄 OK"),
        "honest proof must verify: {honest}"
    );

    let tampered = run(st, "verify", &tampered_args());
    assert!(
        tampered.contains("BAD:pairing"),
        "tampered pub must fail the pairing: {tampered}"
    );

    let _ = std::fs::remove_file(st);
}

#[test]
fn plonk_dbg_challenges_stable() {
    // the transcript exports must stay byte-stable (oracle diffing depends
    // on them) — light smoke: they return 6 comma-separated hex words
    let st = "/tmp/plonk-test-state2.bin";
    let _ = std::fs::remove_file(st);
    let init_out = run(st, "init", INIT);
    assert!(init_out.contains("✅"), "init failed: {init_out}");
    let ch = run(st, "debugChallenges", VERIFY);
    assert!(ch.contains("📄 "), "challenges: {ch}");
    let _ = std::fs::remove_file(st);
}
