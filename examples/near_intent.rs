//! near-intent: run a lisp-rlm contract through NEAR's REAL production VM
//! (vendored near-vm-runner 0.37.3) with MockedExternal trie, real host fns,
//! real gas metering. This is the code path mainnet actually executes.
//!
//! Usage: near-intent <contract.wasm>

use std::sync::Arc;

use near_parameters::{RuntimeConfigStore, RuntimeFeesConfig};
use near_primitives_core::code::ContractCode;
use near_primitives_core::types::{Balance, Gas};
use near_vm_runner::logic::mocks::mock_external::MockedExternal;
use near_vm_runner::logic::VMContext;
use near_vm_runner::internal::VMKindExt;
use near_parameters::vm::VMKind;

const DAO: &str = "dao.testnet";

struct Step {
    signer: &'static str,
    method: &'static str,
    input: &'static str,
    attach: u128,
    expect: Expect,
}

enum Expect {
    Ok,
    Trap(&'static str),
}

struct Ctx {
    ext: MockedExternal,
    code: Arc<ContractCode>,
}

fn ctx(signer: &str, input: &str, attach: u128) -> VMContext {
    VMContext {
        current_account_id: DAO.parse().unwrap(),
        signer_account_id: signer.parse().unwrap(),
        signer_account_pk: vec![0u8; 33],
        predecessor_account_id: signer.parse().unwrap(),
        refund_to_account_id: DAO.parse().unwrap(),
        input: std::rc::Rc::from(input.as_bytes()),
        promise_results: Vec::new().into(),
        block_height: 10,
        block_timestamp: 42,
        epoch_height: 1,
        account_balance: Balance::from_yoctonear(1_000_000_000_000_000_000_000_000_000),
        account_locked_balance: Balance::ZERO,
        storage_usage: 2000,
        account_contract: near_primitives_core::account::AccountContract::None,
        attached_deposit: Balance::from_yoctonear(attach),
        prepaid_gas: Gas::from_teragas(100),
        random_seed: vec![0, 1, 2],
        view_config: None,
        output_data_receivers: vec![],
    }
}

fn run_step(c: &mut Ctx, s: &Step) -> Result<Option<Vec<u8>>, String> {
    let store = RuntimeConfigStore::test();
    let config = store.get_config(near_primitives_core::version::PROTOCOL_VERSION).wasm_config.clone();
    let fees = Arc::new(RuntimeFeesConfig::test());
    let context = ctx(s.signer, s.input, s.attach);
    let gas_counter = context.make_gas_counter(&config);
    let runtime = VMKind::Wasmtime
        .runtime(config)
        .ok_or("wasmtime runtime not compiled")?;
    let outcome = runtime
        .prepare(&c.ext, None, gas_counter, s.method)
        .run(&mut c.ext, &context, fees)
        .map_err(|e| format!("prepare error: {e:?}"))?;
    if let Some(err) = &outcome.aborted {
        return Err(err.to_string());
    }
    println!(
        "     gas burnt: {} ({:.1} Tgas)",
        outcome.burnt_gas.as_gas(),
        outcome.burnt_gas.as_gas() as f64 / 1e12
    );
    Ok(outcome.return_data.as_value())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: near-intent <contract.wasm>");
    let wasm = std::fs::read(path).expect("read wasm");

    let mut c = Ctx {
        ext: MockedExternal::with_code(ContractCode::new(wasm, None)),
        code: Arc::new(ContractCode::new(vec![], None)),
    };

    let steps: Vec<Step> = vec![
        Step { signer: "gov.testnet", method: "init", input: r#"{"threshold":2,"admin":"gov.testnet"}"#, attach: 0, expect: Expect::Ok },
        Step { signer: "gov.testnet", method: "admin_add", input: r#"{"candidate":"carol.testnet"}"#, attach: 0, expect: Expect::Ok },
        Step { signer: "gov.testnet", method: "admin_add", input: r#"{"candidate":"dave.testnet"}"#, attach: 0, expect: Expect::Ok },
        // I1+I2: no self-admission / no sybil vouch
        Step { signer: "mallory.testnet", method: "vouch", input: r#"{"voter":"mallory.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Trap("not vetted") },
        Step { signer: "gov.testnet", method: "vouch", input: r#"{"voter":"gov.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Trap("not vetted") },
        // deposit gate
        Step { signer: "mallory.testnet", method: "apply", input: r#"{"candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Trap("deposit") },
        Step { signer: "mallory.testnet", method: "apply", input: r#"{"candidate":"mallory.testnet"}"#, attach: 1_000_000_000_000_000_000_000, expect: Expect::Ok },
        // applicant cannot self-vouch (status 1 != member)
        Step { signer: "mallory.testnet", method: "vouch", input: r#"{"voter":"mallory.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Trap("not vetted") },
        Step { signer: "carol.testnet", method: "vouch", input: r#"{"voter":"carol.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Ok },
        // 1 vouch < threshold 2: not yet promoted
        Step { signer: "dave.testnet", method: "vouch", input: r#"{"voter":"dave.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Ok },
        // double vouch
        Step { signer: "carol.testnet", method: "vouch", input: r#"{"voter":"carol.testnet","candidate":"mallory.testnet"}"#, attach: 0, expect: Expect::Trap("not pending") }, // re-vouch after promotion
        // now member: promote works, gain vouch power (I4)
        Step { signer: "mallory.testnet", method: "vouch", input: r#"{"voter":"mallory.testnet","candidate":"eve.testnet"}"#, attach: 0, expect: Expect::Trap("not pending") }, // never applied
        // I4: promoted member gains vouch power — eve applies, mallory (promoted) vouches
        Step { signer: "eve.testnet", method: "apply", input: r#"{"candidate":"eve.testnet"}"#, attach: 1_000_000_000_000_000_000_000, expect: Expect::Ok },
        Step { signer: "mallory.testnet", method: "vouch", input: r#"{"voter":"mallory.testnet","candidate":"eve.testnet"}"#, attach: 0, expect: Expect::Ok },
        // renounce kills admin (I6)
        Step { signer: "gov.testnet", method: "renounce_admin", input: "{}", attach: 0, expect: Expect::Ok },
        Step { signer: "gov.testnet", method: "admin_add", input: r#"{"candidate":"zed.testnet"}"#, attach: 0, expect: Expect::Trap("not admin") },
    ];

    let mut pass = 0usize;
    let mut fail = 0usize;
    for (i, s) in steps.iter().enumerate() {
        let r = run_step(&mut c, s);
        let ok = match (&s.expect, &r) {
            (Expect::Ok, Ok(_)) => true,
            (Expect::Trap(sub), Err(e)) => e.contains(sub),
            _ => false,
        };
        if ok {
            pass += 1;
            println!("PASS {:2} {}({})", i + 1, s.method, s.signer);
        } else {
            fail += 1;
            println!("FAIL {:2} {}({}): got {:?}", i + 1, s.method, s.signer, r.as_ref().err());
        }
    }
    println!("\n{pass} passed, {fail} failed (real near-vm-runner, wasmtime backend)");
    std::process::exit(if fail == 0 { 0 } else { 1 });
}
