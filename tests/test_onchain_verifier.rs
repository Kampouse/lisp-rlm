//! On-chain Groth16 verifier (zk/groth16_verify.ts) — compilation test.
//!
//! Verifies the TS→Lisp→WASM pipeline succeeds for the production verifier,
//! including the Bug 2 fix (early-return guards on impure const initializers).
//! The old fixture in fixtures/ uses near.abort(); this version uses `return`
//! for error handling, which was broken before the VariableDeclaration
//! hoist+guard fix.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::typing::type_check_program;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const ONCHAIN_VERIFIER: &str = include_str!("../zk/groth16_verify.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

#[test]
fn onchain_verifier_compiles() {
    let _l = lock();
    let ir = ts_to_lisp_source(ONCHAIN_VERIFIER).expect("TS→Lisp should succeed");
    eprintln!(
        "LOWERED IR (first 4000 chars):\n{}",
        &ir[..ir.len().min(4000)]
    );
    let exprs = parse_all(&ir).expect("Lisp parse should succeed");
    type_check_program(&exprs, true).expect("Type check should pass");
    let wasm = compile_near_from_exprs(&exprs).expect("WASM emit should succeed");
    assert!(
        wasm.len() > 1000,
        "WASM should be non-trivial: {} bytes",
        wasm.len()
    );

    // Verify key functions exist in the lowered IR
    for name in &["init", "verify", "isInitialized", "debugMxLen"] {
        assert!(
            ir.contains(&format!("(define ({} ", name)),
            "function '{}' not found in lowered IR",
            name
        );
    }

    // Verify altBn128 host calls are present
    assert!(
        ir.contains("near/alt_bn128_g1_multiexp"),
        "multiexp host not found"
    );
    assert!(
        ir.contains("near/alt_bn128_pairing_check"),
        "pairing host not found"
    );

    // Verify __fn_done guards appear (Bug 2 fix: impure const initializers
    // after an early return must be guarded). The verify function has early
    // returns like `if (...) { return "BAD:..." }` followed by
    // `const pub = near.altBn128G1Multiexp(mx)`.
    let guard_count = ir.matches("__fn_done").count();
    assert!(
        guard_count > 0,
        "expected __fn_done guards for early-return protection, found none"
    );

    eprintln!(
        "✅ on-chain verifier compiled: {}B IR, {}B WASM, {} __fn_done guards",
        ir.len(),
        wasm.len(),
        guard_count
    );
}
