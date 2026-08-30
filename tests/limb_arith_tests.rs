//! Limb-arithmetic executed tests: compile lisp programs and run them on
//! the REAL near-vm-runner (Wasmtime backend) — the oracle that caught
//! every limb bug to date (Select operand order, value-selects, i32 conds).
//! All expected values are python-verified.

use near_parameters::vm::VMKind;
use near_parameters::{RuntimeConfigStore, RuntimeFeesConfig};
use near_primitives_core::code::ContractCode;
use near_primitives_core::config::ViewConfig;
use near_vm_runner::logic::VMContext;
use near_vm_runner::logic::mocks::mock_external::MockedExternal;
use near_vm_runner::{Contract, prepare, run};
use std::rc::Rc;
use std::sync::Arc;

/// Run `method` on a freshly compiled contract; returns (aborted, return_value).
fn call(wasm: &[u8], method: &str) -> (Option<String>, Vec<u8>) {
    struct CodeWrap(Arc<ContractCode>);
    impl Contract for CodeWrap {
        fn hash(&self) -> near_primitives_core::hash::CryptoHash { *self.0.hash() }
        fn get_code(&self) -> Option<Arc<ContractCode>> { Some(self.0.clone()) }
    }
    let contract = CodeWrap(Arc::new(ContractCode::new(wasm.to_vec(), None)));
    let context = VMContext {
        current_account_id: "t.test.near".parse().unwrap(),
        signer_account_id: "s.test.near".parse().unwrap(),
        signer_account_pk: vec![0, 1, 2],
        predecessor_account_id: "p.test.near".parse().unwrap(),
        refund_to_account_id: "p.test.near".parse().unwrap(),
        input: Rc::from(b"{}".as_slice()),
        promise_results: Vec::new().into(),
        block_height: 1,
        block_timestamp: 42,
        epoch_height: 0,
        account_balance: near_primitives_core::types::Balance::from_near(100),
        account_locked_balance: near_primitives_core::types::Balance::ZERO,
        storage_usage: 100,
        account_contract: near_primitives_core::account::AccountContract::None,
        attached_deposit: near_primitives_core::types::Balance::ZERO,
        prepaid_gas: near_primitives_core::gas::Gas::from_teragas(300),
        random_seed: vec![0, 1, 2],
        view_config: Some(ViewConfig {
            max_gas_burnt: near_primitives_core::gas::Gas::from_teragas(300),
        }),
        output_data_receivers: vec![],
    };
    let store = RuntimeConfigStore::new(None);
    let runtime_config = store.get_config(u32::MAX);
    let wasm_config = (*runtime_config.wasm_config).clone();
    assert!(matches!(wasm_config.vm_kind, VMKind::Wasmtime));
    let fees = Arc::new(RuntimeFeesConfig::test());
    let gas_counter = context.make_gas_counter(&wasm_config);
    let mut ext = MockedExternal::new();
    let prepared = prepare(&contract, Arc::new(wasm_config), None, gas_counter, method);
    let outcome = run(prepared, &mut ext, &context, fees).expect("runner error");
    (
        outcome
            .aborted
            .as_ref()
            .map(|e| format!("{e:?}")),
        outcome.return_data.as_value().unwrap_or_default(),
    )
}

fn compile(src: &str) -> Vec<u8> {
    lisp_rlm_wasm::wasm_emit::compile_near(src).expect("compile error")
}

const PROLOG: &str = r#"
(define (pad9 s) (let ((n (- 9 (str-length s)))) (if (<= n 0) s (str-cat (str-substring "000000000" 0 n) s))))
(define (fmt b l)
  (loop ((j (- l 2)) (acc (to-string (limb-get b (- l 1)))))
    (if (< j 0) acc (recur (- j 1) (str-cat acc (pad9 (to-string (limb-get b j))))))))
"#;

#[test]
fn limb_add_fib_100() {
    // fib via 3 rotating limb buffers (mirrors the deployed bnlimb program)
    let src = format!(r#"{PROLOG}
(define (fib n)
  (let ((A (buf-alloc 128)) (B (buf-alloc 128)) (R (buf-alloc 128)))
    (begin
      (limb-set! A 0 0)
      (limb-set! B 0 1)
      (loop ((a A) (b B) (r R) (la 1) (lb 1) (i 0))
        (if (>= i (- n 1))
          (fmt b lb)
          (recur b r a lb (limb-add a b r la lb) (+ i 1)))))))
(define (run) (near/return (fib 100)))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "fib aborted: {aborted:?}");
    // python: fib(100) = 354224848179261915075
    assert_eq!(String::from_utf8_lossy(&ret), "354224848179261915075");
}

#[test]
fn limb_sub_exact_and_strip() {
    let src = format!(r#"{PROLOG}
(define (run)
  (let ((A (buf-alloc 64)) (B (buf-alloc 64)) (R (buf-alloc 64)))
    (begin
      (limb-set! A 0 123456789)
      (limb-set! A 1 987654321)
      (limb-set! A 2 555555555)
      (limb-set! A 3 111111111)
      (limb-set! B 0 999999999)
      (limb-set! B 1 111111111)
      (limb-set! B 2 888888888)
      (near/return (fmt R (limb-sub A B R 4 3))))))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "sub aborted: {aborted:?}");
    // python: 111111111*10^27 + 555555555*10^18 + 987654321*10^9 + 123456789
    //       - 888888888*10^18 - 111111111*10^9 - 999999999
    let a = 111111111u128 * 10u128.pow(27)
        + 555555555u128 * 10u128.pow(18)
        + 987654321u128 * 10u128.pow(9)
        + 123456789u128;
    let b = 888888888u128 * 10u128.pow(18) + 111111111u128 * 10u128.pow(9) + 999999999u128;
    assert_eq!(String::from_utf8_lossy(&ret), (a - b).to_string());
}

#[test]
fn limb_sub_strips_to_shorter_length() {
    // 1000000000 - 1 = 999999999 → single limb (strip works)
    let src = format!(r#"{PROLOG}
(define (run)
  (let ((A (buf-alloc 64)) (B (buf-alloc 64)) (R (buf-alloc 64)))
    (begin
      (limb-set! A 0 0)
      (limb-set! A 1 1)
      (limb-set! B 0 1)
      (near/return (fmt R (limb-sub A B R 2 1))))))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "strip aborted: {aborted:?}");
    assert_eq!(String::from_utf8_lossy(&ret), "999999999");
}

#[test]
fn limb_sub_underflow_traps() {
    let src = format!(r#"{PROLOG}
(define (run)
  (let ((A (buf-alloc 64)) (B (buf-alloc 64)) (R (buf-alloc 64)))
    (begin
      (limb-set! A 0 5)
      (limb-set! B 0 3)
      (near/return (to-string (limb-sub B A R 1 1))))))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, _ret) = call(&wasm, "run");
    assert!(aborted.is_some(), "3-5 must trap (checked policy)");
}

#[test]
fn limb_cmp_signs_and_length_crossing() {
    // [0,1]=1e9 (2 limbs) vs [999999999] (1 limb): length wins → 1
    let src = format!(r#"{PROLOG}
(define (run)
  (let ((A (buf-alloc 64)) (B (buf-alloc 64)))
    (begin
      (limb-set! A 0 0)
      (limb-set! A 1 1)
      (limb-set! B 0 999999999)
      (near/return (str-cat (str-cat (to-string (limb-cmp A B 2 1)) (to-string (limb-cmp B A 1 2))) (to-string (limb-cmp A A 2 2)))))))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "cmp aborted: {aborted:?}");
    assert_eq!(String::from_utf8_lossy(&ret), "1-10");
}

#[test]
fn limb_mul_factorial_100() {
    let src = format!(r#"{PROLOG}
(define (fact n)
  (let ((X (buf-alloc 4096)) (Y (buf-alloc 4096)) (B (buf-alloc 8)))
    (begin
      (limb-set! X 0 1)
      (limb-set! B 0 1)
      (loop ((x X) (y Y) (lx 1) (k 2))
        (if (> k n)
          (fmt x lx)
          (recur y x (begin (limb-set! B 0 k) (limb-mul x B y lx 1)) (+ k 1)))))))
(define (run) (near/return (fact 100)))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "fact aborted: {aborted:?}");
    // python: 100! (158 digits, exact)
    let expect = "93326215443944152681699238856266700490715968264381621468592963895217599993229915608941463976156518286253697920827223758251185210916864000000000000000000000000";
    assert_eq!(String::from_utf8_lossy(&ret), expect);
}

#[test]
fn limb_mul_two_big_numbers() {
    // A = [1,1,1] = 10^18 + 10^9 + 1. A^2 = 10^36 + 2*10^27 + 3*10^18
    // + 2*10^9 + 1 — crosses many limbs, tests the carry-into-still-zero-slot
    // invariant and multi-limb carries
    let src = format!(r#"{PROLOG}
(define (run)
  (let ((A (buf-alloc 64)) (B (buf-alloc 64)) (R (buf-alloc 128)))
    (begin
      (limb-set! A 0 1)
      (limb-set! A 1 1)
      (limb-set! A 2 1)
      (limb-set! B 0 1)
      (limb-set! B 1 1)
      (limb-set! B 2 1)
      (near/return (fmt R (limb-mul A B R 3 3))))))
(export "run" run #t)
"#);
    let wasm = compile(&src);
    let (aborted, ret) = call(&wasm, "run");
    assert!(aborted.is_none(), "mul aborted: {aborted:?}");
    // python: (10**18+1)**2
    let expect = "1000000002000000003000000002000000001";
    assert_eq!(String::from_utf8_lossy(&ret), expect);
}

#[test]
fn limb_bounds_guards_trap() {
    // limb-set! past the buffer's allocated bytes must trap
    let src = r#"
(define (run)
  (let ((A (buf-alloc 4)))
    (begin (limb-set! A 0 1) (limb-set! A 1 1) (near/return (to-string (limb-get A 1))))))
(export "run" run #t)
"#;
    let wasm = compile(src);
    let (aborted, _ret) = call(&wasm, "run");
    assert!(aborted.is_some(), "OOB limb-set! must trap");
}
