//! Resource flow gate (2026-10-08, `check_resource_flow`).
//!
//! Tier 1 (compile-time, both pipelines): arity validation on the promise/
//! transfer family (slot maps transcribed from the emitter arms), literal
//! gas sanity (per-tx 300 Tgas cap; per-defn closure literal sum), raw
//! arithmetic on u128 money values, i64-truncated u128 at money slots,
//! non-decimal literals at money slots, one attached_deposit value attached
//! twice, and storage money stamps (u128 write marks the key; reads seed
//! u128). Model-dependent findings (undefined same-program callback, gas
//! floor vs attached) are WARNINGS, never errors — the gate only rejects
//! what is provably wrong.
//!
//! These tests shell out to the `compile` binary the same way
//! tests/test_promise_single_use.rs does (CARGO_TARGET_DIR else ./target).

use std::process::Command;

fn compile_bin() -> std::path::PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "./target".into());
    std::path::PathBuf::from(dir).join("release").join("compile")
}

/// Returns (exit_ok, stderr) for compiling `src`.
fn compile_src(src: &str, tag: &str) -> (bool, String) {
    let tmp = std::env::temp_dir().join(format!("__res_gate_{}_{}.lisp", tag, std::process::id()));
    std::fs::write(&tmp, src).unwrap();
    let out = Command::new(compile_bin())
        .arg(&tmp)
        .arg("-o")
        .arg(std::env::temp_dir().join(format!("__res_gate_{}.wasm", tag)))
        .output()
        .expect("compile binary missing (cargo build --release --bin compile)");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn arity_violations_are_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near" "extra")))
    (near/promise_return p)))
"#;
    let (ok, err) = compile_src(src, "arity");
    assert!(!ok, "wrong arity must be rejected");
    assert!(
        err.contains("takes 1 argument, got 2"),
        "arity error must name expected and actual counts, got: {err}"
    );
}

#[test]
fn gas_over_tx_cap_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r (near/promise_batch_action_function_call p "go" "{}" "0" 400000000000000)))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "gascap");
    assert!(!ok, "400 Tgas attach must be rejected");
    assert!(
        err.contains("400 Tgas") && err.contains("300 Tgas"),
        "cap error must name the attach and the cap, got: {err}"
    );
}

#[test]
fn raw_arithmetic_on_u128_is_rejected() {
    let src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128))
         (s (u128/add d "100")))
    (let* ((bad (+ d s)))
      (near/log "done"))))
"#;
    let (ok, err) = compile_src(src, "arith");
    assert!(!ok, "money-op-money must be rejected");
    assert!(
        err.contains("raw arithmetic on a u128 money value") && err.contains("u128/add"),
        "error must give the u128-family fix, got: {err}"
    );
}

#[test]
fn i64_truncated_u128_at_money_slot_is_rejected() {
    let src = r#"(define (main)
  (near/transfer_u128 "recv.test.near" (u128/to-i64 (near/attached_deposit_u128))))
"#;
    let (ok, err) = compile_src(src, "trunc");
    assert!(!ok, "i64-truncated u128 at a money slot must be rejected");
    assert!(
        err.contains("truncated to i64"),
        "error must name the truncation, got: {err}"
    );
}

#[test]
fn non_decimal_literal_at_money_slot_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r (near/promise_batch_action_transfer p "not-a-number")))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "badlit");
    assert!(!ok, "non-decimal money literal must be rejected");
    assert!(
        err.contains("not a decimal string"),
        "error must name the literal, got: {err}"
    );
}

#[test]
fn one_deposit_attached_twice_is_rejected() {
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near"))
         (q (near/promise_batch_create "b.test.near"))
         (d (near/attached_deposit)))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" d 10000000000000))
           (r2 (near/promise_batch_action_function_call q "go" "{}" d 10000000000000)))
      (near/promise_return r1))))
"#;
    let (ok, err) = compile_src(src, "dep2x");
    assert!(!ok, "one deposit attached twice must be rejected");
    assert!(
        err.contains("attached to two promise operations"),
        "error must name the double attach, got: {err}"
    );
}

#[test]
fn storage_money_stamps_flow() {
    // Write a u128 value to a literal key, read it back, do raw arithmetic
    // on the read → reject (the stamp flows through the read).
    let src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128)))
    (let* ((_w (near/storage_write "ledger:balance" d)))
      (let* ((bal (near/storage_read "ledger:balance")))
        (let* ((bad (+ bal "1")))
          (near/log "done"))))))
"#;
    let (ok, err) = compile_src(src, "stamp");
    assert!(!ok, "stamped-key read feeding raw arithmetic must be rejected");
    assert!(
        err.contains("raw arithmetic on a u128 money value"),
        "got: {err}"
    );

    // The same read feeding a money sink directly is legal.
    let ok_src = r#"(define (main)
  (let* ((d (near/attached_deposit_u128)))
    (let* ((_w (near/storage_write "ledger:balance" d)))
      (let* ((bal (near/storage_read "ledger:balance")))
        (let* ((t (near/transfer_u128 "recv.test.near" bal)))
          (near/log "paid"))))))
"#;
    let (ok2, err2) = compile_src(ok_src, "stampok");
    assert!(ok2, "stamped-key read at a money sink must compile, got: {err2}");
}

#[test]
fn legal_resource_program_compiles_and_warns_on_dead_callback() {
    // Legal: u128 family on the string domain, decimal amounts, bounded
    // gas, deposit attached once, in-program callback.
    let src = r#"(define (cb) (near/log "ticked"))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near"))
         (d (near/attached_deposit_u128))
         (s (u128/add d "100")))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" "1000000000000000000" 10000000000000))
           (r2 (near/promise_batch_action_transfer p "5")))
      (let* ((t (near/promise_then r1 (near/current_account_id) "cb" "{}" "0" 10000000000000)))
        (if (u128/lt s "1000") (near/promise_return t) (near/promise_return r1))))))
"#;
    let (ok, err) = compile_src(src, "legal");
    assert!(ok, "legal resource program must compile, got: {err}");

    // Undefined same-program callback: compiles, but MUST warn (the
    // dead-receipt class the gate surfaces without breaking compiles).
    let src2 = r#"(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((t (near/promise_then p (near/current_account_id) "never_defined" "{}" "0" 10000000000000)))
      (near/promise_return t))))
"#;
    let (ok2, err2) = compile_src(src2, "deadc");
    assert!(ok2, "undefined callback must NOT be a hard error (cross-contract ambiguity), got: {err2}");
    assert!(
        err2.contains("warning:") && err2.contains("never_defined") && err2.contains("MethodNotFound"),
        "undefined callback must produce the dead-receipt warning, got: {err2}"
    );
}

// ===== const-bound gas literals (2026-10-09) =====
// Regression: splt3 attached 300 Tgas per receipt via a hoisted const —
// six receipts on one tx — and the gas gate only summed bare literals, so
// the symbol at the gas slot was invisible and the contract sailed through
// to testnet ("Exceeded the prepaid gas" on every receipt). Top-level
// numeric defines are immutable in this language, so resolving a symbol at
// a gas slot is sound; anything else (shadowed, computed) stays opaque and
// unchecked per the gate's hard-reject-only-what-is-provable rule.

#[test]
fn hoisted_gas_const_counts_toward_tx_cap() {
    let src = r#"(define TGAS 300000000000000)
(define (cb) (near/log "ticked"))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" "0" TGAS))
           (r2 (near/promise_batch_action_function_call p "go" "{}" "0" TGAS)))
      (let* ((t (near/promise_then r1 (near/current_account_id) "cb" "{}" "0" TGAS))
             (u (near/promise_then r2 (near/current_account_id) "cb" "{}" "0" TGAS)))
        (near/promise_return u)))))
"#;
    let (ok, err) = compile_src(src, "hoistgas");
    assert!(!ok, "4x300 Tgas via a hoisted const must be rejected");
    assert!(
        err.contains("1200 Tgas") && err.contains("300 Tgas"),
        "error must resolve the const, sum the closure, and name the cap, got: {err}"
    );
}

#[test]
fn small_hoisted_gas_const_compiles() {
    let src = r#"(define G 10000000000000)
(define (cb) (near/log "ticked"))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r1 (near/promise_batch_action_function_call p "go" "{}" "0" G))
           (t (near/promise_then r1 (near/current_account_id) "cb" "{}" "0" G)))
      (near/promise_return t))))
"#;
    let (ok, err) = compile_src(src, "hoistok");
    assert!(ok, "small hoisted consts must compile, got: {err}");
}

#[test]
fn mixed_literal_and_const_gas_sums_across_closure() {
    let src = r#"(define HELPER_GAS 100000000000000)
(define (helper)
  (let* ((p (near/promise_batch_create "b.test.near")))
    (let* ((r (near/promise_batch_action_function_call p "go" "{}" "0" HELPER_GAS)))
      (near/promise_return r))))
(define (main)
  (let* ((p (near/promise_batch_create "a.test.near")))
    (let* ((r (near/promise_batch_action_function_call p "go" "{}" "0" 250000000000000)))
      (let* ((h (helper)))
        (near/promise_return r)))))
"#;
    let (ok, err) = compile_src(src, "mixgas");
    assert!(!ok, "closure sum must include const-bound gas from callees");
    assert!(
        err.contains("350 Tgas"),
        "250T literal + 100T const must sum, got: {err}"
    );
}

#[test]
fn shadowed_gas_name_stays_unchecked_not_misresolved() {
    // A binder shadows the const name at the gas slot: the slot value is
    // the binder's (10 Tgas, legal). Resolving it to the const (400 Tgas)
    // would wrongly reject — the shadow check must keep the slot opaque.
    let src = r#"(define TGAS 400000000000000)
(define (main)
  (let* ((TGAS 10000000000000))
    (let* ((p (near/promise_batch_create "a.test.near")))
      (let* ((r (near/promise_batch_action_function_call p "go" "{}" "0" TGAS)))
        (near/promise_return r)))))
"#;
    let (ok, err) = compile_src(src, "shadowgas");
    assert!(
        ok,
        "shadowed name must not resolve to the const's value, got: {err}"
    );
}


// ---------------------------------------------------------------------------
// Compile-time gas fill (`(gas default)` → static floor, 2026-10-09).
//
// The promise graph is fully static, so the compiler can pick callback gas
// itself: fill = (CB_FLOOR_BASE + CB_FLOOR_PER_HOST * hosts_closure(cb)) * 2.
// Only SAME-PROGRAM callbacks are filled (we own that code); `default` on a
// remote account is a hard error — the callee is unknowable (docs: "we do
// not own the callee"), and a silent guess could underfund a receipt.
// Explicit numeric gas remains the override (today's behavior, unchanged).

/// u64 → unsigned LEB128, as wasm encodes i64.const immediates.
fn leb128(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break out;
        }
    }
}

fn wasm_bytes(src: &str, tag: &str) -> Vec<u8> {
    let (ok, err) = compile_src(src, tag);
    assert!(ok, "fixture must compile: {err}");
    // compile_src emits the artifact at this exact path (see above).
    let tmp = std::env::temp_dir().join(format!("__res_gate_{}.wasm", tag));
    std::fs::read(&tmp).expect("wasm artifact")
}

const CB_1HOST_SRC: &str = r#"(define (cb) (near/log "done"))
(define (main)
  (let* ((p (near/promise_create "self" "noop" "{}" "0" 0)))
    (let* ((r (near/promise_then p (near/current_account_id) "cb" "0" "0" (gas default))))
      (near/promise_return r))))
"#;

#[test]
fn gas_default_fills_same_program_callback_floor() {
    // cb closure = 1 host op (log): floor = 1T + 0.01T = 1.01T; x2 = 2.02T.
    let bytes = wasm_bytes(CB_1HOST_SRC, "fill1");
    let want = leb128(2_020_000_000_000);
    assert!(
        bytes.windows(want.len()).any(|w| w == want),
        "emitted wasm must carry the filled gas i64.const 2.02T (LEB128 {:?})",
        want
    );
}

#[test]
fn gas_default_fill_grows_with_callback_closure() {
    // Two host ops in the callback closure: floor = 1.02T; x2 = 2.04T.
    let src = r#"(define (cb) (near/log "a") (near/log "b"))
(define (main)
  (let* ((p (near/promise_create "self" "noop" "{}" "0" 0)))
    (let* ((r (near/promise_then p (near/current_account_id) "cb" "0" "0" (gas default))))
      (near/promise_return r))))
"#;
    let bytes = wasm_bytes(src, "fill2");
    let want = leb128(2_040_000_000_000);
    assert!(
        bytes.windows(want.len()).any(|w| w == want),
        "two host ops must fill 2.04T, got no LEB128 {:?} in wasm",
        want
    );
}

#[test]
fn gas_default_on_remote_account_is_hard_error() {
    // We do not own the remote callee: its floor is unknowable, so a silent
    // fill would be a guess — the gate must reject, not invent.
    let src = r#"(define (main)
  (let* ((p (near/promise_create "self" "noop" "{}" "0" 0)))
    (let* ((r (near/promise_then p "remote.near" "anywhere" "0" "0" (gas default))))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "fillremote");
    assert!(!ok, "remote-callee default must not compile");
    assert!(err.contains("(gas default)"), "error must name the form: {err}");
}

#[test]
fn gas_default_local_callback_undefined_is_hard_error() {
    // Locality is proven via current_account_id, but the method is NOT a
    // define in this file: hosts_closure of nothing = no floor — refuse.
    let src = r#"(define (main)
  (let* ((p (near/promise_create "self" "noop" "{}" "0" 0)))
    (let* ((r (near/promise_then p (near/current_account_id) "ghost" "0" "0" (gas default))))
      (near/promise_return r))))
"#;
    let (ok, err) = compile_src(src, "fillghost");
    assert!(!ok, "undefined local callback default must not compile");
    assert!(err.contains("ghost"), "error must name the method: {err}");
}

#[test]
fn explicit_gas_still_overrides_default() {
    // Explicit numeric gas keeps today's semantics exactly (override, no
    // diagnostic when it covers the floor).
    let bytes = wasm_bytes(CB_1HOST_SRC.replace("(gas default)", "5000000000000").as_str(), "overr");
    let want = leb128(5_000_000_000_000);
    assert!(
        bytes.windows(want.len()).any(|w| w == want),
        "explicit gas must reach the wasm unchanged"
    );
}
