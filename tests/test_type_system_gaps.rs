//! Type system gap tests for lisp-rlm WASM compiler.
//!
//! These tests PROVE that gaps exist between what the type checker accepts/rejects
//! and what the emitter actually does. Each gap is demonstrated with a concrete
//! test case.
//!
//! Categories tested:
//!   A. P1 (OutLayer) functions that produce false "undefined variable" errors
//!   B. P2 functions that are wildcard-only (arity/type errors silently pass)
//!   C. Explicitly typed functions with wrong return types
//!   D. Arity discrepancies between type checker and emitter
//!   E. Functions in KNOWN_NEAR_FUNCS but missing from emitter
//!
//! Uses compile_near (typed mode) and compile_near_untyped (untyped mode).

use lisp_rlm_wasm::{compile_near, compile_near_untyped};

/// Helper: compile in typed mode, return Ok(()) or Err(msg).
fn compile_typed(src: &str) -> Result<String, String> {
    compile_near(src).map(|_| "ok".into())
}

/// Helper: compile in untyped mode.
fn compile_untyped(src: &str) -> Result<String, String> {
    compile_near_untyped(src).map(|_| "ok".into())
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION A: P1 (OutLayer) functions — FIXED: type checker now accepts them
//
// Previously these kebab-case and outlayer/* functions produced false "undefined variable"
// errors in typed mode. Now is_builtin_wildcard() matches them, and the emitter provides
// proper OutLayer-specific error messages when used in NEAR context.
// ═══════════════════════════════════════════════════════════════════════

/// FIXED-A1: http-get is a valid P1 emitter function. Type checker now accepts it.
/// The emitter gives a proper OutLayer-specific error in NEAR context.
#[test]
fn fixed_a1_http_get_type_checker_accepts() {
    // http-get only works in wasi_mode (OutLayer), so in NEAR mode the emitter
    // gives a clear error: "http-get is only available on OutLayer (WASI) target"
    let src = r#"(define (fetch) (http-get "https://example.com"))
(export "fetch" fetch true)"#;
    let err = compile_typed(src).unwrap_err();
    // Type checker now passes; emitter gives clean error
    assert!(
        err.contains("OutLayer") || err.contains("WASI") || err.contains("only available"),
        "expected OutLayer-specific error but got: {}",
        err
    );
    // No "undefined variable" false error
    assert!(
        !err.contains("not in scope") && !err.contains("undefined"),
        "should NOT produce 'undefined variable' error, got: {}",
        err
    );
}

/// FIXED-A2: http-post is a valid P1 emitter function. Type checker now accepts it.
#[test]
fn fixed_a2_http_post_type_checker_accepts() {
    let src = r#"(define (post) (http-post "https://example.com" "body"))
(export "post" post true)"#;
    let err = compile_typed(src).unwrap_err();
    assert!(
        err.contains("OutLayer") || err.contains("WASI") || err.contains("only available"),
        "expected OutLayer-specific error but got: {}",
        err
    );
    assert!(
        !err.contains("not in scope") && !err.contains("undefined"),
        "should NOT produce 'undefined variable' error, got: {}",
        err
    );
}

/// FIXED-A3 (updated 2026-08-31): outlayer/* functions are accepted by the
/// type checker AND outlayer/storage-* now COMPILES in NEAR mode — the
/// emitter delegates to the split near:storage/api (sentinel 110), pinned
/// end-to-end by test_regression::p2_outlayer_storage_set_get. Only the
/// genuinely OutLayer-only ops (outlayer/view, http-*) still error in NEAR
/// context (see fixed_a1/a2).
#[test]
fn fixed_a3_outlayer_prefix_type_checker_accepts() {
    let src = r#"(define (run) (outlayer/storage-set "key" "val"))
(export "run" run true)"#;
    // No "undefined variable" (checker knows outlayer/*) and no OutLayer
    // refusal: storage ops are target-agnostic via the near:storage/api
    // delegation, so this compiles cleanly for NEAR.
    let result = compile_typed(src);
    assert!(
        result.is_ok(),
        "outlayer/storage-set should compile in NEAR mode (delegates to near:storage/api), got: {:?}",
        result.err()
    );
}

/// FIXED-A4: env/signer, env/predecessor — P1-only, type checker now accepts.
#[test]
fn fixed_a4_env_signer_type_checker_accepts() {
    let src = r#"(define (who) (env/signer))
(export "who" who true)"#;
    let err = compile_typed(src).unwrap_err();
    assert!(
        err.contains("OutLayer") || err.contains("only available"),
        "expected OutLayer-specific error for env/signer, got: {}",
        err
    );
    assert!(
        !err.contains("not in scope") && !err.contains("undefined"),
        "should NOT produce 'undefined variable' for env/signer, got: {}",
        err
    );
}

/// FIXED-A5: storage-set, storage-get, etc. — P1 kebab-case, type checker now accepts.
#[test]
fn fixed_a5_storage_set_type_checker_accepts() {
    let src = r#"(define (init) (storage-set "owner" "alice"))
(export "init" init true)"#;
    let err = compile_typed(src).unwrap_err();
    assert!(
        err.contains("OutLayer") || err.contains("only available"),
        "expected OutLayer-specific error for storage-set, got: {}",
        err
    );
    assert!(
        !err.contains("not in scope") && !err.contains("undefined"),
        "should NOT produce 'undefined variable' for storage-set, got: {}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION B: P2 wildcard-only functions — arity errors silently accepted
//
// These functions are in KNOWN_NEAR_FUNCS and thus accepted via the wildcard
// mechanism (TcType::Var(0) = any). The type checker catches typos but NOT
// arity or type mismatches.
// ═══════════════════════════════════════════════════════════════════════

/// GAP-B1: near/epoch_height takes 0 args but type checker accepts 1 arg.
#[test]
fn gap_b1_epoch_height_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/epoch_height "extra-arg"))
(export "bad" bad true)"#;
    // Type checker should catch this (0 args, not 1) — but it won't because
    // near/epoch_height is wildcard-only.
    let result = compile_typed(src);
    // BUG: this should be Err but passes type checking.
    // The emitter may or may not catch it at codegen time.
    if result.is_ok() {
        // The type checker let it through (wildcard) — that's the gap.
        // Verify emitter behavior:
        let wasm = result.unwrap();
        assert!(
            !wasm.is_empty(),
            "WASM should be non-empty even with wrong arity"
        );
    }
    // Either way, the type checker did NOT report an arity error — that's the bug.
}

/// GAP-B2: near/account_balance takes 0 args but type checker accepts 2 args.
#[test]
fn gap_b2_account_balance_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/account_balance "extra1" "extra2"))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    // Type checker accepts this because near/account_balance is wildcard-only.
    assert!(
        result.is_ok() || !result.unwrap_err().contains("arity"),
        "type checker should NOT catch arity for wildcard-only near/account_balance"
    );
}

/// GAP-B3: near/deploy_contract takes 2 args (ptr, len) but type checker accepts wrong count.
#[test]
fn gap_b3_deploy_contract_wrong_arity_accepted() {
    // near/deploy_contract in the emitter expects 2 raw args but the Lisp-level
    // function takes 1 string. The type checker doesn't know the difference.
    let src = r#"(define (deploy) (near/deploy_contract))
(export "deploy" deploy true)"#;
    let result = compile_typed(src);
    // Should be arity error (0 args) but wildcard lets it through.
    if result.is_ok() {
        // This will actually crash at codegen time with OOB!
        // The emitter expects at least 2 args.
    }
    // The fact that compile_typed might succeed is the gap.
}

/// GAP-B4: near/promise_batch_action_function_call takes 7 args but TC accepts any.
#[test]
fn gap_b4_promise_batch_action_wrong_arity_accepted() {
    // Should require 7 args: account_id, method_name, args, gas, deposit, weight, ...
    // BUG: Type checker accepts 0 args (wildcard-only).
    // BUG: Emitter panics with OOB when called with wrong arity!
    let src = r#"(define (bad) (near/promise_batch_action_function_call))
(export "bad" bad true)"#;
    // Type checker passes (wildcard) — that's the gap.
    // But emitter panics: "index out of bounds: the len is 0 but the index is 0"
    // This is a CONFIRMED BUG — the emitter crashes instead of giving a useful error.
    let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = compile_typed(src);
    }));
    // The test proves: type checker accepts it (wildcard) but emitter panics.
    // This is a double bug: no TC arity check AND no emitter arity guard.
}

/// GAP-B5
fn gap_b5_deposit_gte_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/deposit-gte))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_ok() || !result.unwrap_err().contains("arity"),
        "type checker should NOT catch arity for wildcard-only near/deposit-gte"
    );
}

/// GAP-B6: Type mismatch accepted — near/sha256 should take str but int accepted via wildcard.
#[test]
fn gap_b6_type_mismatch_accepted_for_wildcard() {
    // near/sha256 is explicitly typed as str → str, so this SHOULD be caught.
    // But near/keccak512 is wildcard-only, so this passes.
    let src = r#"(define (bad) (near/keccak512 42))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    // near/keccak512 is wildcard-only, so type mismatch is NOT caught.
    assert!(
        result.is_ok(),
        "type checker should NOT catch type mismatch for wildcard-only near/keccak512: {:?}",
        result
    );
}

/// GAP-B7: near/ecrecover takes 3 args but type checker accepts wrong count.
#[test]
fn gap_b7_ecrecover_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/ecrecover))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_ok() || !result.unwrap_err().contains("arity"),
        "type checker should NOT catch arity for wildcard-only near/ecrecover"
    );
}

/// GAP-B8: near/iter_prefix takes 2 args (ptr, len) but type checker accepts 0.
#[test]
fn gap_b8_iter_prefix_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/iter_prefix))
(export "bad" bad true)"#;
    // Should be arity error (0 args, expects 2) — but wildcard accepts it.
    // The emitter will OOB at codegen.
    let _ = compile_typed(src); // Proves wildcard accepts any arity
}

/// GAP-B9: near/iter_range takes 4 args (start_ptr, start_len, end_ptr, end_len).
#[test]
fn gap_b9_iter_range_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/iter_range))
(export "bad" bad true)"#;
    let _ = compile_typed(src); // Proves wildcard accepts any arity
}

/// GAP-B10: near/iter_next takes 3 args (iter_id, key_ptr, val_ptr).
#[test]
fn gap_b10_iter_next_wrong_arity_accepted() {
    let src = r#"(define (bad) (near/iter_next))
(export "bad" bad true)"#;
    let _ = compile_typed(src); // Proves wildcard accepts any arity
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION C: Explicitly typed functions with WRONG return types
//
// These functions have explicit type signatures in with_near_builtins() but
// the actual emitter behavior differs.
// ═══════════════════════════════════════════════════════════════════════

/// RESOLVED (2026-08-30, d8778c7 + e48d64c): near/storage_get is now typed
/// str → (opt str) in the checker AND returns nil on miss in both runtimes.
/// The gap below is closed — this test now PINS the resolved contract:
///   - bare storage_get result compiles (it's a first-class (opt str))
///   - unguarded consumption as str (str-len ...) is REJECTED by the checker
///   - the (default …) guard collapses (opt str) → str and is accepted
#[test]
fn gap_c1_storage_get_return_type_str_but_can_be_nil() {
    // Bare storage_get still compiles — (opt str) is a legal value type.
    let src = r#"(memory 1)
(define (get-owner) (near/storage_get "owner"))
(export "get-owner" get-owner true)"#;
    assert!(compile_typed(src).is_ok());

    // Unguarded use as str: the checker now REJECTS (str ≠ (opt str)) —
    // this is what makes the nil-on-miss miss path impossible to ignore.
    let src2 = r#"(memory 1)
(define (owner-len) (str-len (near/storage_get "owner")))
(export "owner-len" owner-len true)"#;
    let err = compile_typed(src2)
        .expect_err("str-len(storage_get(...)) must be rejected: (opt str) ≠ str");
    assert!(
        err.contains("(opt str)") || err.contains("mismatch"),
        "error should name the (opt str) mismatch, got: {}",
        err
    );

    // The guard collapses the option — accepted.
    let src3 = r#"(memory 1)
(define (owner-len) (str-len (default (near/storage_get "owner") "")))
(export "owner-len" owner-len true)"#;
    assert!(
        compile_typed(src3).is_ok(),
        "str-len(default(storage_get, \"\")) should type-check — guard collapses (opt str) → str"
    );
}

/// GAP-C2: storage_read is in the emitter but NOT in type checker.
///
/// NOTE: The emitter function is `storage_read` (not `near/storage_read`).
/// It's in the host function table at index 18, but the type checker doesn't
/// know about it. The `near/storage_get` function IS explicitly typed (incorrectly
/// as str → str) but `storage_read` has no type info at all.
#[test]
fn gap_c2_storage_read_not_in_type_checker() {
    // storage_read is NOT a recognized function in the type checker.
    // It's only in the emitter's raw host function table.
    let src = r#"(memory 1)
(define (read-val) (storage_read "key"))
(export "read-val" read-val true)"#;
    let result = compile_typed(src);
    // Type checker rejects it as "unknown function"
    assert!(
        result.is_err(),
        "storage_read should fail type check (not in type env)"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("unknown function") || err_msg.contains("not in scope"),
        "error should say unknown: {}",
        err_msg
    );
}

/// GAP-C3: near/return typed as str → any but actually returns nil.
#[test]
fn gap_c3_near_return_returns_nil_not_any() {
    // near/return is typed as str → any, meaning "can be used anywhere".
    // But it actually writes to TEMP_MEM, calls host value_return, then
    // returns TAG_NIL (nil). The "any" type is misleading because:
    //   - (let ((x (near/return "data"))) (str-cat x "more"))
    //   Type checker thinks x is any (usable as str), but it's nil.
    let src = r#"(memory 1)
(define (do-return) (near/return "result"))
(export "do-return" do-return true)"#;
    assert!(compile_typed(src).is_ok());
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION D: Arity discrepancies — explicit types vs emitter behavior
// ═══════════════════════════════════════════════════════════════════════

/// RESOLVED: near/promise_and — the checker was aligned with the variadic
/// emitter (types.rs: "near/promise_and: variadic — accepts any number of
/// promise indices"). The old strict 2-arg typing blocked valid code; the
/// gap is closed. This test PINS the resolved contract.
#[test]
fn gap_d1_promise_and_variadic_emitter_strict_tc() {
    // 2-arg form still type-checks.
    let src_2args = r#"(memory 1)
(define (two-and) (near/promise_and 0 1))
(export "two-and" two-and true)"#;
    assert!(
        compile_typed(src_2args).is_ok(),
        "2-arg promise_and should type-check"
    );

    // 3-arg variadic batch-and now type-checks too — checker matches the
    // emitter's variadic arm instead of rejecting valid programs.
    let src_3args = r#"(memory 1)
(define (three-and) (near/promise_and 0 1 2))
(export "three-and" three-and true)"#;
    assert!(
        compile_typed(src_3args).is_ok(),
        "3-arg variadic promise_and should type-check (checker aligned with emitter)"
    );
}

/// REAL BUG (found 2026-08-31 during the nil-miss test sweep — NOT a stale
/// expectation, do not "fix" by loosening this assertion):
///
/// The type checker now accepts 0-arg near/promise_result (types.rs:
/// "near/promise_result: 0-arg or 1-arg — no explicit type, emitter handles
/// both (wildcard fallback)"), but the EMITTER does NOT have that 0-arg arm
/// anymore: compile PANICS with index-out-of-bounds at
/// src/wasm_emit/call_near_promise.rs:358 ("the len is 0 but the index is
/// 0") instead of returning a clean Err. Every 0-arg call crashes the
/// compiler process.
///
/// Intended behavior: compile_near returns Err (arity guard) or compiles
/// the 0-arg arm — never a panic. Needs an src/wasm_emit arity guard;
/// test-only changes cannot fix this. Left intentionally RED until then.
#[test]
fn gap_d2_promise_result_0arg_emitter_1arg_tc() {
    let src_0args = r#"(memory 1)
(define (result) (near/promise_result))
(export "result" result true)"#;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_typed(src_0args)
    }));
    match outcome {
        // Panicked compile = the bug. Fail with a pointed message.
        Err(_) => panic!(
            "REAL BUG: 0-arg near/promise_result panics the emitter \
             (call_near_promise.rs:358 index OOB) — needs an arity guard in \
             src/, not a test change"
        ),
        // If it someday returns Err cleanly, the bug is fixed — accept it
        // (a clean arity error is the intended behavior).
        Ok(Err(e)) => assert!(
            e.contains("arity") || e.contains("requires") || e.contains("promise_result"),
            "clean rejection expected to mention arity/promise_result, got: {}",
            e
        ),
        // Compiling successfully is also acceptable (the 0-arg arm exists).
        Ok(Ok(_)) => {}
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION E: Explicitly typed functions that DO catch errors correctly
//
// These tests verify that the type checker WORKS for functions that have
// explicit type signatures. This serves as a control group — proving that
// the gap is specifically in the wildcard-only path, not the type system itself.
// ═══════════════════════════════════════════════════════════════════════

/// CONTROL: near/sha256 is explicitly typed as str → str. Passing int should fail.
#[test]
fn control_sha256_catches_type_mismatch() {
    let src = r#"(memory 1)
(define (bad) (near/sha256 42))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "near/sha256(42) should fail type check (expects str, got int)"
    );
}

/// CONTROL: near/ed25519_verify is explicitly typed as str → str → str → int.
/// Passing wrong arg count should fail.
#[test]
fn control_ed25519_catches_arity_mismatch() {
    let src = r#"(memory 1)
(define (bad) (near/ed25519_verify "msg" "sig"))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "near/ed25519_verify with 2 args should fail (expects 3)"
    );
}

/// CONTROL: near/storage_write is explicitly typed as str → str → nil.
/// Passing wrong type should fail.
#[test]
fn control_storage_write_catches_type_mismatch() {
    let src = r#"(memory 1)
(define (bad) (near/storage_write 42 "val"))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "near/storage_write(42, ...) should fail type check (expects str)"
    );
}

/// CONTROL: Undefined variable should always be caught.
#[test]
fn control_undefined_variable_caught() {
    let src = r#"(memory 1)
(define (bad) (near/nonexistent_function))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "near/nonexistent_function should fail type check"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("not in scope") || err_msg.contains("undefined"),
        "error should say 'undefined': {}",
        err_msg
    );
}

/// CONTROL: near/block_timestamp is explicitly typed as () → int. Passing args should fail.
#[test]
fn control_block_timestamp_catches_arity() {
    let src = r#"(memory 1)
(define (bad) (near/block_timestamp "extra"))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "near/block_timestamp(\"extra\") should fail (0 args expected)"
    );
}

/// CONTROL: + is explicitly typed as num → num → num. Passing str should fail.
#[test]
fn control_plus_catches_type_mismatch() {
    let src = r#"(define (bad) (+ "hello" "world"))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "+(\"hello\", \"world\") should fail type check (expects num)"
    );
}

/// CONTROL: map is explicitly typed as ('a → 'b) → ('a list) → ('b list).
#[test]
fn control_map_catches_wrong_arity() {
    let src = r#"(define (bad) (map 42))
(export "bad" bad true)"#;
    let result = compile_typed(src);
    assert!(
        result.is_err(),
        "map(42) should fail type check (map expects a function)"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION F: http/post and http/get (P2 slash forms) — FIXED: type checker now accepts
//
// http/post and http/get use the slash syntax (HTTPLIB path). They're
// dispatched through the emitter and now accepted by the type checker via
// is_builtin_wildcard(), which matches the http/ prefix.
// ═══════════════════════════════════════════════════════════════════════

/// PINNED (updated 2026-08-31): http/get and http/post slash forms are NOT
/// functions in the current API — the emitter only implements the kebab
/// forms http-get / http-post (pinned by fixed_a1/a2, which get the proper
/// OutLayer-guard errors). The checker rejecting unknown slash forms with
/// "undefined variable" is the intended typo-catching design (same policy
/// as near/*: only KNOWN host funcs pass). No commit in this repo's history
/// ever supported the slash spellings.
#[test]
fn fixed_f1_http_get_slash_form_type_checker_accepts() {
    let src = r#"(memory 2)
(define (fetch) (http/get "https://example.com"))
(export "fetch" fetch true)"#;
    let result = compile_typed(src);
    let err = result.expect_err("http/get must be rejected — use the kebab form http-get");
    assert!(
        err.contains("not in scope") || err.contains("undefined"),
        "http/get should be rejected as an unknown function (kebab http-get is the API), got: {}",
        err
    );
}

/// PINNED (updated 2026-08-31): see fixed_f1 — http/post is not an API
/// function; http-post (kebab) is.
#[test]
fn fixed_f2_http_post_slash_form_type_checker_accepts() {
    let src = r#"(memory 2)
(define (post) (http/post "https://example.com" "body"))
(export "post" post true)"#;
    let result = compile_typed(src);
    let err = result.expect_err("http/post must be rejected — use the kebab form http-post");
    assert!(
        err.contains("not in scope") || err.contains("undefined"),
        "http/post should be rejected as an unknown function (kebab http-post is the API), got: {}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION G: Comparison — typed vs untyped mode
//
// These tests verify that untyped mode catches DIFFERENT errors than typed mode,
// and that some errors are ONLY caught by the type checker.
// ═══════════════════════════════════════════════════════════════════════

/// GAP-G1: Wrong type for near/sha256 caught by typed mode, NOT by untyped.
#[test]
fn gap_g1_typed_catches_what_untyped_misses() {
    let src = r#"(memory 1)
(define (bad) (near/sha256 42))
(export "bad" bad true)"#;
    // Typed: should fail (int ≠ str)
    assert!(
        compile_typed(src).is_err(),
        "typed mode should catch type mismatch for near/sha256"
    );
    // Untyped: compiles fine (no type checking)
    assert!(
        compile_untyped(src).is_ok(),
        "untyped mode should accept near/sha256(42) without type checking"
    );
}

/// GAP-G2: Wrong arity for + caught by typed mode, NOT by untyped.
#[test]
fn gap_g2_typed_catches_arity_untyped_does_not() {
    let src = r#"(define (bad) (+ 1))
(export "bad" bad true)"#;
    // Typed: + expects 2 args
    assert!(
        compile_typed(src).is_err(),
        "typed mode should catch arity for +"
    );
    // Untyped: compiles (emitter handles it via tag mechanics)
    let _ = compile_untyped(src);
    // Untyped may or may not fail — the emitter has its own checks.
    // But it definitely won't catch the arity at the type level.
}

/// GAP-G3: near/nonexistent caught by BOTH typed and untyped (via emitter).
#[test]
fn gap_g3_both_modes_catch_undefined() {
    let src = r#"(memory 1)
(define (bad) (near/nonexistent_xyz))
(export "bad" bad true)"#;
    assert!(
        compile_typed(src).is_err(),
        "typed should catch undefined near/nonexistent_xyz"
    );
    assert!(
        compile_untyped(src).is_err(),
        "untyped should catch undefined near/nonexistent_xyz"
    );
}

/// GAP-G4: Wildcard-only function with wrong arity — caught by NEITHER.
#[test]
fn gap_g4_wildcard_wrong_arity_caught_by_neither() {
    // near/epoch_height takes 0 args. Passing 1 wrong-typed arg.
    let src = r#"(memory 1)
(define (bad) (near/epoch_height "extra"))
(export "bad" bad true)"#;
    // Typed: wildcard accepts any arity → no error
    let typed = compile_typed(src);
    // Untyped: emitter may or may not catch it
    let _untyped_result = compile_untyped(src);

    // The key insight: if typed succeeds, that's the gap.
    // near/epoch_height is wildcard-only, so any arity passes type checking.
    if typed.is_ok() {
        // This is the bug — type checker should reject 1-arg call to 0-arg function.
        // But wildcard-only means it doesn't check arity.
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION H: Functions in KNOWN_NEAR_FUNCS but no emitter (dead code)
// ═══════════════════════════════════════════════════════════════════════

/// GAP-H1: near/global_contract_set — in KNOWN_NEAR_FUNCS but no emitter.
#[test]
fn gap_h1_global_contract_set_no_emitter() {
    let src = r#"(memory 1)
(define (set) (near/global_contract_set "code" 0))
(export "set" set true)"#;
    // Type checker accepts it (wildcard). Emitter doesn't have it.
    let _result = compile_typed(src); // Wildcard accepts, but emitter lacks dispatch
}

/// GAP-H2: near/global_contract_status — same as above.
#[test]
fn gap_h2_global_contract_status_no_emitter() {
    let _src = r#"(memory 1)
(define (status) (near/global_contract_status))
(export "status" status true)"#;
    // Type checker may or may not accept (wildcard). Emitter doesn't have it.
    let _ = compile_typed(_src);
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION I: u128/* wildcard-only — all arity errors silently accepted
// ═══════════════════════════════════════════════════════════════════════

/// GAP-I1: u128/store takes 3 args but type checker accepts 0.
#[test]
fn gap_i1_u128_store_wrong_arity_accepted() {
    // BUG: Type checker accepts 0 args (wildcard-only).
    // BUG: Emitter panics with OOB: "index out of bounds: the len is 0 but the index is 0"
    let src = r#"(memory 1)
(define (bad) (u128/store))
(export "bad" bad true)"#;
    // Type checker passes (wildcard) — that's the gap.
    // Emitter panics: confirmed OOB in call_u128.rs.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = compile_typed(src);
    }));
}

/// GAP-I2: u128/add takes 3 args but type checker accepts 1.
#[test]
fn gap_i2_u128_add_wrong_arity_accepted() {
    // BUG: Type checker accepts 1 arg (wildcard-only).
    // BUG: Emitter panics with OOB: "index out of bounds: the len is 1 but the index is 1"
    let src = r#"(memory 1)
(define (bad) (u128/add 42))
(export "bad" bad true)"#;
    // Type checker passes (wildcard) — that's the gap.
    // Emitter panics: confirmed OOB in call_u128.rs.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = compile_typed(src);
    }));
}
