//! Comprehensive P2 / NEAR host function test suite.
//!
//! Maps every function across all NEAR subsystems and tests each for:
//!   1. Compile correctness (wasm-tools validate passes)
//!   2. WASM binary size (detects gross bloat)
//!   3. WAT round-trip (structurally sound, no stack issues)
//!
//! Uses `compile_near_untyped` (no type checker) to test the full emitter surface.
//! Bugs found are tagged with [BUG] in comments.

use lisp_rlm_wasm::compile_near_untyped;
use std::process::Command;

/// Compile Lisp source to NEAR WASM and validate with wasm-tools.
fn validate_wasm(src: &str) -> Result<(Vec<u8>, String), String> {
    let wasm_bytes = compile_near_untyped(src).map_err(|e| format!("compile error: {}", e))?;

    if wasm_bytes.is_empty() {
        return Err("compile returned empty WASM".to_string());
    }

    // Write to temp file for wasm-tools validation
    // Use PID + thread ID to avoid race conditions in multi-threaded test runner
    let thread_id = format!("{:?}", std::thread::current().id());
    let tmp = std::env::temp_dir().join(format!(
        "p2_test_{}_{}.wasm",
        std::process::id(),
        &thread_id
    ));
    std::fs::write(&tmp, &wasm_bytes).map_err(|e| format!("write temp: {}", e))?;

    let output = Command::new("wasm-tools")
        .args(["validate", tmp.to_str().unwrap()])
        .output()
        .map_err(|e| format!("wasm-tools spawn: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("wasm-tools validate failed:\n{}", err));
    }

    // WAT for diagnostics (via wasm-tools print)
    let print = Command::new("wasm-tools")
        .args(["print", tmp.to_str().unwrap()])
        .output()
        .map_err(|e| format!("wasm-tools print spawn: {}", e))?;

    let _ = std::fs::remove_file(&tmp);
    let wat = String::from_utf8_lossy(&print.stdout).to_string();
    Ok((wasm_bytes, wat))
}

/// Test macro: compile, validate, report size + WAT length.
macro_rules! near_host_test {
    ($name:ident, $code:expr) => {
        #[test]
        fn $name() {
            let src = concat!(
                "(memory 1)\n",
                "(define (test) ",
                $code,
                ")\n",
                r#"(export "test" test true)"#
            );
            let (wasm, wat) =
                validate_wasm(src).unwrap_or_else(|e| panic!("{}\nsource:\n{}\n", e, src));
            eprintln!(
                "  [PASS] {} — {} bytes, WAT: {} chars",
                stringify!($name),
                wasm.len(),
                wat.len()
            );
            // Sanity: WASM should be under 100KB for single-function tests
            assert!(
                wasm.len() < 100_000,
                "WASM suspiciously large: {} bytes",
                wasm.len()
            );
        }
    };
}

// ════════════════════════════════════════════════════════════════
// A. NEAR Host Functions — systematic mapping
// ════════════════════════════════════════════════════════════════

// A1. Registers: read_register(0), register_len(1), write_register(2)
near_host_test!(near_read_register, "(let ((len (near/input))) len)");
near_host_test!(near_write_register, "(near/return (near/input))");

// A2. Context: current_account_id(3), signer_account_id(4), signer_account_pk(5),
//           predecessor_account_id(6), input(7)
near_host_test!(near_current_account_id, "(near/current_account_id)");
near_host_test!(near_signer_account_id, "(near/signer_account_id)");
near_host_test!(near_signer_account_pk, "(near/signer_account_pk)");
near_host_test!(near_predecessor_account_id, "(near/predecessor_account_id)");
near_host_test!(near_input, "(near/input)");

// A3. Block: block_index(8), block_timestamp(9), epoch_height(10)
near_host_test!(near_block_index, "(near/block_index)");
near_host_test!(near_block_timestamp, "(near/block_timestamp)");
near_host_test!(near_epoch_height, "(near/epoch_height)");

// A4. Economics: storage_usage(11), account_balance(12), account_locked_balance(13),
//              attached_deposit(14), prepaid_gas(15), used_gas(16)
near_host_test!(near_storage_usage, "(near/storage_usage)");
near_host_test!(near_account_balance, "(near/account_balance)");
near_host_test!(near_account_locked_balance, "(near/account_locked_balance)");
near_host_test!(near_attached_deposit, "(near/attached_deposit)");
near_host_test!(near_prepaid_gas, "(near/prepaid_gas)");
near_host_test!(near_used_gas, "(near/used_gas)");

// A5. Storage: storage_write(17), storage_read(18), storage_remove(19), storage_has_key(20)
//    Aliases: near/store, near/load, near/has_key, near/remove
near_host_test!(near_store, r#"(near/store "key" "value")"#);
near_host_test!(near_load, r#"(near/load "key")"#);
near_host_test!(near_remove, r#"(near/remove "key")"#);
near_host_test!(near_has_key, r#"(near/has_key "key")"#);
near_host_test!(near_storage_set, r#"(near/storage_set "key" "value")"#);
near_host_test!(near_storage_get, r#"(near/storage_get "key")"#);
near_host_test!(near_storage_has, r#"(near/storage_has "key")"#);
near_host_test!(near_storage_remove, r#"(near/storage_remove "key")"#);

// A6. Crypto: sha256(21), keccak256(22), random_seed(23), ed25519_verify(24), p256_verify(55)
near_host_test!(near_sha256, r#"(near/sha256 "hello")"#);
near_host_test!(near_keccak256, r#"(near/keccak256 "hello")"#);
near_host_test!(near_random_seed, "(near/random_seed)");
near_host_test!(
    near_ed25519_verify,
    r#"(near/ed25519_verify "sig" "msg" "pk")"#
);
near_host_test!(near_p256_verify, r#"(near/p256_verify "sig" "msg" "pk")"#);
near_host_test!(near_keccak512, r#"(near/keccak512 "hello")"#);
near_host_test!(near_ripemd160, r#"(near/ripemd160 "hello")"#);
near_host_test!(
    near_ecrecover,
    r#"(near/ecrecover "hash" "sig" "v" "r" "s")"#
);

// A7. Value return: value_return(25), panic(26), panic_utf8(27)
near_host_test!(near_return_str, r#"(near/return_str "hello")"#);
near_host_test!(near_panic, r#"(near/panic "oops")"#);

// A8. Logging: log_utf8(28), log_utf16(29)
near_host_test!(near_log, r#"(near/log "hello")"#);
near_host_test!(near_log_utf16, r#"(near/log_utf16 "hello")"#);

// A9. Promises: promise_create(30), promise_then(31), promise_and(32),
//             promise_results_count(33), promise_result(34), promise_return(35)
near_host_test!(
    near_promise_create,
    r#"(near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)"#
);
near_host_test!(
    near_promise_then,
    r#"(let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
       (near/promise_then p "callback.contract" "on_result" (near/input) 0 0))"#
);
near_host_test!(
    near_promise_and,
    r#"(let ((p1 (near/promise_create "a.near" "method1" (near/input) 0 0))
               (p2 (near/promise_create "b.near" "method2" (near/input) 0 0)))
       (near/promise_and p1 p2))"#
);
near_host_test!(near_promise_results_count, "(near/promise_results_count)");
near_host_test!(near_promise_result, "(near/promise_result 0)");
near_host_test!(
    near_promise_return,
    r#"(let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
        (near/promise_result 0)
       (near/promise_return p))"#
);

// A10. Combined: cross-contract call with result
near_host_test!(
    near_cc_full_flow,
    r#"(let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
       (near/promise_result 0))"#
);

// A11. Alt BN128: 56–58
near_host_test!(
    near_alt_bn128_g1_multiexp,
    r#"(near/alt_bn128_g1_multiexp "data")"#
);
near_host_test!(near_alt_bn128_g1_sum, r#"(near/alt_bn128_g1_sum "data")"#);
near_host_test!(
    near_alt_bn128_pairing_check,
    r#"(near/alt_bn128_pairing_check "data")"#
);

// A12. BLS12-381: 59–67
near_host_test!(near_bls12381_p1_sum, r#"(near/bls12381_p1_sum "data")"#);
near_host_test!(near_bls12381_p2_sum, r#"(near/bls12381_p2_sum "data")"#);
near_host_test!(
    near_bls12381_g1_multiexp,
    r#"(near/bls12381_g1_multiexp "data")"#
);
near_host_test!(
    near_bls12381_g2_multiexp,
    r#"(near/bls12381_g2_multiexp "data")"#
);
near_host_test!(
    near_bls12381_map_fp_to_g1,
    r#"(near/bls12381_map_fp_to_g1 "data")"#
);
near_host_test!(
    near_bls12381_map_fp2_to_g2,
    r#"(near/bls12381_map_fp2_to_g2 "data")"#
);
near_host_test!(
    near_bls12381_pairing_check,
    r#"(near/bls12381_pairing_check "data")"#
);
near_host_test!(
    near_bls12381_p1_decompress,
    r#"(near/bls12381_p1_decompress "data")"#
);
near_host_test!(
    near_bls12381_p2_decompress,
    r#"(near/bls12381_p2_decompress "data")"#
);

// A13. JSON builtins
near_host_test!(
    near_json_get_str,
    r#"(json-get-str "{\"key\": \"val\"}" "key")"#
);
near_host_test!(near_json_get_int, r#"(json-get "{\"key\": 42}" "key")"#);

// A14. Misc: near/abort, near/current_code_hash, near/deposit-gte
near_host_test!(near_abort, r#"(near/abort 42 "reason")"#);
near_host_test!(near_current_code_hash, "(near/current_code_hash)");
near_host_test!(near_deposit_gte, "(near/deposit-gte 1000000)");

// A15. Shorthand getters (type-checked mode only — skipped in untyped tests)

// ════════════════════════════════════════════════════════════════
// B. [BUG] Known broken functions — these panic at compile time
// ════════════════════════════════════════════════════════════════

/// [BUG] near/iter_prefix: dispatch code expects 2 args (ptr, len) but the
/// function is documented as taking 1 string arg. Crashes with index OOB.
#[test]
#[should_panic]
fn bug_iter_prefix_oob() {
    let src = r#"(memory 1)
(define (test) (near/iter_prefix "prefix"))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/iter_range: dispatch expects 4 args (start_ptr, start_len, end_ptr, end_len)
/// but documented as taking 2 string args.
#[test]
#[should_panic]
fn bug_iter_range_oob() {
    let src = r#"(memory 1)
(define (test) (near/iter_range "start" "end"))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/iter_next: dispatch expects 3 args (iter_id, key_ptr, val_ptr)
/// but documented as taking 1 arg (iter_id).
#[test]
#[should_panic]
fn bug_iter_next_oob() {
    let src = r#"(memory 1)
(define (test) (let ((it (near/iter_prefix "prefix"))) (near/iter_next it)))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/promise_batch_create: dispatch expects 2 args (ptr, len) raw pointer
/// but takes 1 string arg at Lisp level.
#[test]
#[should_panic]
fn bug_promise_batch_create_oob() {
    let src = r#"(memory 1)
(define (test) (near/promise_batch_create "account.near"))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/promise_batch_then: expects 3 raw args (batch_id, ptr, len).
#[test]
#[should_panic]
fn bug_promise_batch_then_oob() {
    let src = r#"(memory 1)
(define (test) (let ((b (near/promise_batch_create "a.near")))
       (near/promise_batch_then b "b.near" "callback")))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/promise_batch_action_function_call: expects 7 raw args.
#[test]
#[should_panic]
fn bug_promise_batch_action_function_call_oob() {
    let src = r#"(memory 1)
(define (test) (let ((b (near/promise_batch_create "a.near")))
       (near/promise_batch_action_function_call b "method" "args" 0 0)))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/promise_batch_action_transfer: expects 3 raw args.
#[test]
#[should_panic]
fn bug_promise_batch_action_transfer_oob() {
    let src = r#"(memory 1)
(define (test) (let ((b (near/promise_batch_create "a.near")))
       (near/promise_batch_action_transfer b 1000000)))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/deploy_contract: dispatch expects 2 args (ptr, len) raw pointer.
#[test]
#[should_panic]
fn bug_deploy_contract_oob() {
    let src = r#"(memory 1)
(define (test) (near/deploy_contract "code"))
(export "test" test true)"#;
    compile_near_untyped(src).unwrap();
}

/// [BUG] near/account_balance_high: not recognized in untyped mode.
#[test]
fn bug_account_balance_high_unknown() {
    let src = r#"(memory 1)
(define (test) (near/account_balance_high))
(export "test" test true)"#;
    let result = compile_near_untyped(src);
    assert!(
        result.is_err(),
        "Expected compile error for unknown near/account_balance_high"
    );
    assert!(
        result.unwrap_err().contains("unknown function"),
        "Expected 'unknown function' error"
    );
}

/// [BUG] register_len: not recognized as a standalone function.
#[test]
fn bug_register_len_unknown() {
    let src = r#"(memory 1)
(define (test) (register_len 0))
(export "test" test true)"#;
    let result = compile_near_untyped(src);
    assert!(
        result.is_err(),
        "Expected compile error for unknown register_len"
    );
}

/// [BUG] near/log_num: actually compiles fine — just a numeric log variant.
#[test]
fn bug_log_num_actually_works() {
    let src = r#"(memory 1)
(define (test) (near/log_num 42))
(export "test" test true)"#;
    let result = compile_near_untyped(src);
    assert!(result.is_ok(), "near/log_num should compile");
}

/// [BUG] near/deposit-gte: actually compiles fine.
#[test]
fn bug_deposit_gte_actually_works() {
    let src = r#"(memory 1)
(define (test) (near/deposit-gte 1000000))
(export "test" test true)"#;
    let result = compile_near_untyped(src);
    assert!(result.is_ok(), "near/deposit-gte should compile");
}

// ════════════════════════════════════════════════════════════════
// C. Memory stress tests
// ════════════════════════════════════════════════════════════════

/// Multiple string allocations + host calls — tests heap pointer advancement.
#[test]
fn memory_many_strings() {
    let src = r#"(memory 2)
(define (test)
  (let ((a "hello")
        (b "world")
        (c "foo")
        (d "bar")
        (e (str-cat a b))
        (f (str-cat c d))
        (g (near/sha256 e)))
    (near/return_str (str-cat f g))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] memory_many_strings — {} bytes", wasm.len());
}

/// str-cat multiple times — tests memory reuse after temp strings.
#[test]
fn memory_str_cat_multiple() {
    let src = r#"(memory 2)
(define (test)
  (let ((result (str-cat (str-cat "a" "b") (str-cat "c" "d"))))
    (near/return_str result)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] memory_str_cat_multiple — {} bytes", wasm.len());
}

/// Store then load — tests memory persistence across host calls.
#[test]
fn memory_store_load_cycle() {
    let src = r#"(memory 2)
(define (test)
  (near/store "mykey" "myvalue")
  (let ((v (near/load "mykey")))
    (near/return_str v)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] memory_store_load_cycle — {} bytes", wasm.len());
}

/// Subtraction — tests arithmetic doesn't corrupt memory.
#[test]
fn memory_subtraction() {
    let src = r#"(memory 1)
(define (test)
  (let ((a 100)
        (b 30)
        (c (- a b)))
    (near/return_str (str-cat "result: " (str c)))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] memory_subtraction — {} bytes", wasm.len());
}

/// Large string — tests memory allocation for bigger payloads.
#[test]
fn memory_large_string() {
    let src = r#"(memory 4)
(define (test)
  (let ((big (str-cat "AAAAAAAAAA" "BBBBBBBBBB")))
    (near/sha256 big)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] memory_large_string — {} bytes", wasm.len());
}

// ════════════════════════════════════════════════════════════════
// D. Control flow + host call interaction
// ════════════════════════════════════════════════════════════════

/// cond branching with host calls in both branches.
#[test]
fn control_flow_cond_host_calls() {
    let src = r#"(memory 2)
(define (test)
  (let ((ts (near/block_timestamp)))
    (if (> ts 1000000)
        (near/store "status" "new")
        (near/store "status" "old"))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!(
        "  [PASS] control_flow_cond_host_calls — {} bytes",
        wasm.len()
    );
}

/// Loop with host call inside — tests stack balance after iteration.
#[test]
fn control_flow_loop_host_call() {
    let src = r#"(memory 2)
(define (test)
  (near/store "counter" "0")
  (let ((x 0))
    (while (< x 5)
      (near/log (str-cat "iter: " (str x)))
      (set! x (+ x 1)))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!(
        "  [PASS] control_flow_loop_host_call — {} bytes",
        wasm.len()
    );
}

/// Function calling another function with host result.
#[test]
fn control_flow_fn_chain() {
    let src = r#"(memory 2)
(define (greet name) (str-cat "Hello, " name))
(define (test)
  (let ((account (near/signer_account_id)))
    (near/return_str (greet account))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] control_flow_fn_chain — {} bytes", wasm.len());
}
// ════════════════════════════════════════════════════════════════

/// Multiple exported functions — tests function table + export section.
#[test]
fn multi_export_two_functions() {
    let src = r#"(memory 2)
(define (get_ts) (near/block_timestamp))
(define (get_signer) (near/signer_account_id))
(export "get_ts" get_ts true)
(export "get_signer" get_signer true)"#;
    let (wasm, wat) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    assert!(wat.contains("export"), "WAT should contain exports");
    eprintln!("  [PASS] multi_export_two_functions — {} bytes", wasm.len());
}

/// Kitchen sink: everything combined.
#[test]
fn kitchen_sink() {
    let src = r#"(memory 4)
(define (init)
  (near/store "owner" (near/signer_account_id))
  (near/store "counter" "0"))
(define (increment)
  (near/log "incrementing"))
(define (get_balance_of account)
  (near/promise_create "wrap.near" "ft_balance_of" account 0 0))
(export "init" init true)
(export "increment" increment true)
(export "get_balance_of" get_balance_of true)"#;
    let (wasm, wat) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!(
        "  [PASS] kitchen_sink — {} bytes, {} chars WAT",
        wasm.len(),
        wat.len()
    );
    assert!(wasm.len() > 500, "Kitchen sink WASM should be substantial");
}

// ════════════════════════════════════════════════════════════════
// F. Promise chain stress test
// ════════════════════════════════════════════════════════════════

/// Promise create → result → return chain.
#[test]
fn promise_chain_create_result_return() {
    let src = r#"(memory 2)
(define (test)
  (let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
    (near/promise_result 0)
    (near/promise_return p)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!(
        "  [PASS] promise_chain_create_result_return — {} bytes",
        wasm.len()
    );
}

/// Promise and — two parallel creates.
#[test]
fn promise_and_parallel() {
    let src = r#"(memory 2)
(define (test)
  (let ((p1 (near/promise_create "a.near" "m1" (near/input) 0 0))
        (p2 (near/promise_create "b.near" "m2" (near/input) 0 0)))
    (near/promise_and p1 p2)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] promise_and_parallel — {} bytes", wasm.len());
}

/// Nested let with promise operations — tests stack depth.
#[test]
fn promise_nested_let() {
    let src = r#"(memory 2)
(define (test)
  (let ((caller (near/signer_account_id))
        (deposit (near/attached_deposit))
        (p (near/promise_create "target.near" "method" (near/input) 0 0)))
    (near/promise_return p)))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] promise_nested_let — {} bytes", wasm.len());
}

// ════════════════════════════════════════════════════════════════
// G. Arithmetic edge cases
// ════════════════════════════════════════════════════════════════

#[test]
fn arithmetic_muldiv() {
    let src = r#"(memory 1)
(define (test)
  (let ((a 100)
        (b 3)
        (product (* a b))
        (quotient (/ a b)))
    (str-cat (str product) (str quotient))))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] arithmetic_muldiv — {} bytes", wasm.len());
}

#[test]
fn arithmetic_comparison() {
    let src = r#"(memory 1)
(define (test)
  (let ((a 10)
        (b 20))
    (if (> b a) "bigger" "smaller")))
(export "test" test true)"#;
    let (wasm, _) = validate_wasm(src).unwrap_or_else(|e| panic!("{}", e));
    eprintln!("  [PASS] arithmetic_comparison — {} bytes", wasm.len());
}
