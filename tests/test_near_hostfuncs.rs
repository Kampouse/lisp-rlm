//! Tests for NEAR host function Lisp forms — typed near-mode surface
//! (rewritten 2026-10-12 after the fail-fast census exposed the file as
//! never-run: it predated the typed near/ layer and fuzz-mode split, so
//! every fixture used pre-typing raw ABIs and `compile_fuzz`, which has
//! no `near/*` scope at all — 23 tests that could never pass).
//!
//! Current surface (emitter = source of truth, verified by probe):
//!   near/ed25519_verify : str → str → str → int            (3 args)
//!   near/ecrecover      : str → str → int → int → str      (4 args)
//!   near/promise_yield_create : str → str → int → int     (4 args)
//!   near/promise_yield_resume : str → str                 (2 args)
//!   near/promise_batch_action_function_call_weight :
//!         int → str → str → int → int → int               (6 args)
//!   zero-arg context getters : → int (storage_usage, signer_account_pk,
//!         account_balance[_high], account_locked_balance[_high],
//!         epoch_height, validator_total_stake); → str (signer_account_pk)
//!
//! Retired ops (never shipped as near/* forms — phantoms from a
//! pre-typing aspiration; the v3 TS API covers the need):
//!   near/json_return_bool / _null / _raw / _u128, near/json_get_bool
//!
//! Each test compiles via the real NEAR pipeline (compile_near — the
//! fuzz pipeline has no near/ scope by design), validates WASM structure,
//! and checks that the correct imports are present.

use lisp_rlm_wasm::near_validate::validate_near_wasm;
use lisp_rlm_wasm::wasm_emit::compile_near;

/// Helper: compile near-mode source to WASM, validate, return bytes.
fn compile_and_validate(source: &str) -> Result<Vec<u8>, String> {
    let wasm = compile_near(source)?;
    let _ = wasmtime::Module::from_binary(&wasmtime::Engine::default(), &wasm)
        .map_err(|e| format!("wasmtime validation: {}", e))?;
    Ok(wasm)
}

/// Helper: check that a WASM binary imports a specific function from "env".
fn has_import(wasm: &[u8], name: &str) -> bool {
    let wat = wasmprinter::print_bytes(wasm).unwrap_or_default();
    for line in wat.lines() {
        let trimmed = line.trim();
        if trimmed.contains("(import \"env\"") && trimmed.contains(&format!("\"{}\"", name)) {
            return true;
        }
    }
    false
}

// ── ed25519_verify (typed: sig msg pk — hex strings, not raw ptr/len) ──

#[test]
fn test_ed25519_verify_compiles() {
    let source = r#"
        (define (verify-sig sig msg pk)
            (near/ed25519_verify sig msg pk))
    "#;
    let wasm = compile_and_validate(source).expect("ed25519_verify should compile");
    assert!(
        has_import(&wasm, "ed25519_verify"),
        "WASM should import ed25519_verify"
    );
}

// ── promise_yield_create (typed: method args gas weight) ──

#[test]
fn test_promise_yield_create_compiles() {
    let source = r#"
        (define (do-yield method args gas weight)
            (near/promise_yield_create method args gas weight))
    "#;
    let wasm = compile_and_validate(source).expect("promise_yield_create should compile");
    assert!(
        has_import(&wasm, "promise_yield_create"),
        "WASM should import promise_yield_create"
    );
}

// ── promise_yield_resume (typed: data tail) ──

#[test]
fn test_promise_yield_resume_compiles() {
    let source = r#"
        (define (do-resume data tail)
            (near/promise_yield_resume data tail))
    "#;
    let wasm = compile_and_validate(source).expect("promise_yield_resume should compile");
    assert!(
        has_import(&wasm, "promise_yield_resume"),
        "WASM should import promise_yield_resume"
    );
}

// ── ecrecover (typed: hash r v malleability — 4 args, s ignored) ──

#[test]
fn test_ecrecover_compiles() {
    let source = r#"
        (define (do-ecrecover hash r v mal)
            (near/ecrecover hash r v mal))
    "#;
    let wasm = compile_and_validate(source).expect("ecrecover should compile");
    assert!(
        has_import(&wasm, "ecrecover"),
        "WASM should import ecrecover"
    );
}

// ── promise_batch_action_function_call_weight (typed: idx method args
//    amount gas weight) ──

#[test]
fn test_promise_batch_action_function_call_weight_compiles() {
    let source = r#"
        (define (do-weighted idx method args amount gas weight)
            (near/promise_batch_action_function_call_weight idx method args amount gas weight))
    "#;
    let wasm = compile_and_validate(source)
        .expect("promise_batch_action_function_call_weight should compile");
    assert!(
        has_import(&wasm, "promise_batch_action_function_call_weight"),
        "WASM should import promise_batch_action_function_call_weight"
    );
}

// ── Wrong arg count errors (emitter messages) ──

#[test]
fn test_ed25519_verify_wrong_arg_count() {
    let source = "(define (bad) (near/ed25519_verify 1 2))";
    let result = compile_near(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("arity mismatch: 3 args vs 2 args"));
}

#[test]
fn test_promise_yield_create_wrong_arg_count() {
    let source = "(define (bad) (near/promise_yield_create 1 2 3))";
    let result = compile_near(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("arity mismatch: 4 args vs 3 args"));
}

#[test]
fn test_promise_yield_resume_wrong_arg_count() {
    let source = "(define (bad) (near/promise_yield_resume 1))";
    let result = compile_near(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("arity mismatch: 2 args vs 1 args"));
}

#[test]
fn test_ecrecover_wrong_arg_count() {
    let source = "(define (bad) (near/ecrecover 1 2 3))";
    let result = compile_near(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("arity mismatch: 4 args vs 3 args"));
}

#[test]
fn test_promise_batch_action_function_call_weight_wrong_arg_count() {
    let source = "(define (bad) (near/promise_batch_action_function_call_weight 1 2 3))";
    let result = compile_near(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("promise_batch_action_function_call_weight: need 6 args"));
}

// ── Minor host funcs ──

#[test]
fn test_storage_usage_compiles() {
    let source = r#"
        (define (get-usage)
            (near/storage_usage))
    "#;
    let wasm = compile_and_validate(source).expect("storage_usage should compile");
    assert!(
        has_import(&wasm, "storage_usage"),
        "WASM should import storage_usage"
    );
}

#[test]
fn test_account_locked_balance_compiles() {
    // typed surface entry added 2026-10-12 — emitter had host 13 wired
    // but the type env never listed it ("undefined variable" on use)
    let source = r#"
        (define (get-locked)
            (near/account_locked_balance))
    "#;
    let wasm = compile_and_validate(source).expect("account_locked_balance should compile");
    assert!(
        has_import(&wasm, "account_locked_balance"),
        "WASM should import account_locked_balance"
    );
}

#[test]
fn test_signer_account_pk_compiles() {
    let source = r#"
        (define (get-pk)
            (near/signer_account_pk))
    "#;
    let wasm = compile_and_validate(source).expect("signer_account_pk should compile");
    assert!(
        has_import(&wasm, "signer_account_pk"),
        "WASM should import signer_account_pk"
    );
}

// ── Retired ops: phantom pins (tombstones, 2026-10-12) ──
//
// The original file pinned near/json_return_bool, near/json_return_null,
// near/json_return_raw, near/json_return_u128 and near/json_get_bool —
// none ever shipped as near/* forms (no emitter arm, no type entry; the
// probes here were how we found out). The need they addressed is covered
// by the v3 API: typed getters + `??` + near.returnStr/returnRaw.

#[test]
fn test_json_return_family_never_existed() {
    for op in [
        "near/json_return_bool",
        "near/json_return_null",
        "near/json_return_raw",
        "near/json_return_u128",
        "near/json_get_bool",
    ] {
        let src = format!("(define (bad) ({} 1))", op);
        let err = compile_near(&src).unwrap_err();
        assert!(
            err.contains("undefined variable"),
            "{op} must stay undefined (phantom pin tombstoned 2026-10-12): {err}"
        );
    }
}
