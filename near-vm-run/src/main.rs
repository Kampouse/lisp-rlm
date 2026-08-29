//! near-vm-run — the production-VMLogic oracle for lisp-rlm contracts.
//!
//! Runs a compiled contract through the REAL near-vm-runner (Wasmtime
//! backend, production host functions, real fee schedule) with a
//! file-persisted storage backend. CLI mirrors near-mock:
//!
//!   near-vm-run <wasm> <method> [args-json] [--view] [--prepaid <TGAS>]
//!   near-vm-run <wasm> reset
//!
//! Conformance oracle: near-mock is the fast inner loop; this is the
//! deploy gate. Output divergence between the two is a bug in one of
//! them (historically near-mock, e.g. the input() register bug).

use near_parameters::RuntimeConfigStore;
use near_parameters::vm::VMKind;
use near_parameters::RuntimeFeesConfig;
use near_primitives_core::code::ContractCode;
use near_primitives_core::config::ViewConfig;
use near_vm_runner::logic::VMContext;
use near_vm_runner::logic::External;
use near_vm_runner::logic::mocks::mock_external::MockedExternal;
use near_vm_runner::{prepare, run};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

const STATE_FILE: &str = "/tmp/near-vm-run-state.bin";

// Context defaults — parity with near-mock
const CURRENT: &str = "escrow.test.near";
const SIGNER: &str = "owner.test.near";
const PREDECESSOR: &str = "owner.test.near";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: near-vm-run <wasm> <method> [args-json] [--view] [--prepaid <TGAS>]\n       near-vm-run <wasm> reset"
        );
        std::process::exit(1);
    }

    let run_view = args.iter().any(|a| a == "--view");
    let prepaid_tgas: f64 = args
        .iter()
        .position(|a| a == "--prepaid")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(300.0);
    let prepaid: u64 = (prepaid_tgas * 1e12) as u64;
    // --deposit <NEAR> (decimal NEAR) and --ts <ns> override context values.
    let deposit_yocto: u128 = args
        .iter()
        .position(|a| a == "--deposit")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|n| (n * 1e24) as u128)
        .unwrap_or(0);
    let ts_override: u64 = args
        .iter()
        .position(|a| a == "--ts")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(42);

    let wasm_path = &args[1];
    let method = args[2].clone();
    let args_json = args.get(3).cloned().unwrap_or_else(|| "{}".to_string());

    if method == "reset" {
        let _ = std::fs::remove_file(STATE_FILE);
        println!("🗑️  State cleared");
        return Ok(());
    }

    // Load persisted storage into the mocked external
    let persisted: HashMap<Vec<u8>, Vec<u8>> = std::fs::read(STATE_FILE)
        .ok()
        .and_then(|d| bincode::deserialize(&d).ok())
        .unwrap_or_default();
    println!(
        "{}",
        if persisted.is_empty() {
            "🆕 Fresh state".to_string()
        } else {
            format!("📂 Loaded {} storage keys", persisted.len())
        }
    );

    let mut ext = MockedExternal::new();
    ext.fake_trie.extend(persisted);

    let context = VMContext {
        current_account_id: CURRENT.parse().unwrap(),
        signer_account_id: SIGNER.parse().unwrap(),
        signer_account_pk: vec![0, 1, 2],
        predecessor_account_id: PREDECESSOR.parse().unwrap(),
        refund_to_account_id: PREDECESSOR.parse().unwrap(),
        input: Rc::from(args_json.as_bytes()),
        promise_results: Vec::new().into(),
        block_height: 1,
        block_timestamp: ts_override,
        epoch_height: 0,
        account_balance: near_primitives_core::types::Balance::from_near(100),
        account_locked_balance: near_primitives_core::types::Balance::ZERO,
        storage_usage: 100,
        account_contract: near_primitives_core::account::AccountContract::None,
        attached_deposit: near_primitives_core::types::Balance::from_yoctonear(deposit_yocto),
        prepaid_gas: near_primitives_core::gas::Gas::from_gas(prepaid),
        random_seed: vec![0, 1, 2],
        view_config: if run_view {
            Some(ViewConfig { max_gas_burnt: near_primitives_core::gas::Gas::from_teragas(300) })
        } else {
            None
        },
        output_data_receivers: vec![],
    };

    let wasm_bytes = std::fs::read(wasm_path)?;
    println!("📦 {} ({} bytes)", wasm_path, wasm_bytes.len());

    // Config: default = latest in-crate schedule; --config <file> = live
    // protocol config JSON from EXPERIMENTAL_protocol_config RPC (exact
    // chain schedule — gas tables evolve, the crate line may not).
    let config_store = RuntimeConfigStore::new(None);
    let runtime_config = config_store.get_config(u32::MAX);
    let mut wasm_config = (*runtime_config.wasm_config).clone();
    let mut fees = RuntimeFeesConfig::test();
    if let Some(i) = args.iter().position(|a| a == "--config") {
        let path = args.get(i + 1).ok_or("--config needs a path")?;
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let rc = &raw["result"]["runtime_config"];
        let wcj = &rc["wasm_config"];
        // Hand-apply the fields that matter for VM gas (VMConfig has no serde).
        if let Some(v) = wcj["fix_contract_loading_cost"].as_bool() {
            wasm_config.fix_contract_loading_cost = v;
        }
        if let Some(v) = wcj["grow_mem_cost"].as_u64() {
            wasm_config.grow_mem_cost = v as u32;
        }
        if let Some(v) = wcj["regular_op_cost"].as_u64() {
            wasm_config.regular_op_cost = v as u32;
        }
        if let Some(v) = wcj["linear_op_base_cost"].as_u64() {
            wasm_config.linear_op_base_cost = v;
        }
        if let Some(v) = wcj["linear_op_unit_cost"].as_u64() {
            wasm_config.linear_op_unit_cost = v;
        }
        // Live action fees — VMLogic charges promise actions from this table.
        // test() inflates function_call_base to 2.32 Tgas (send+exec), live is
        // 0.78 exec / 0.2 send → overstates promise-carrying calls ~8x.
        let mut live_fees = RuntimeFeesConfig::test();
        let ac = &rc["transaction_costs"]["action_creation_config"];
        fn fee_of(v: &serde_json::Value) -> Option<near_parameters::Fee> {
            Some(near_parameters::Fee::new(
                v.get("send_sir")?.as_u64()?,
                v.get("send_not_sir")?.as_u64()?,
                v.get("execution")?.as_u64()?,
            ))
        }
        if let Some(f) = fee_of(&ac["function_call_cost"]) {
            live_fees.action_fees[near_parameters::ActionCosts::function_call_base] = f;
        }
        if let Some(pb) = ac["function_call_cost_per_byte"].as_u64() {
            live_fees.action_fees[near_parameters::ActionCosts::function_call_byte] =
                near_parameters::Fee::new(pb, pb, pb);
        }
        if let Some(f) = fee_of(&ac["deploy_contract_cost"]) {
            live_fees.action_fees[near_parameters::ActionCosts::deploy_contract_base] = f;
        }
        if let Some(f) = fee_of(&ac["transfer_cost"]) {
            live_fees.action_fees[near_parameters::ActionCosts::transfer] = f;
        }
        // .then() registers data receivers → VMLogic charges new_data_receipt_base
        // (+byte) from this table. test() has the ancient 4.3 Tgas base; live is 0.0365.
        let drc = &rc["transaction_costs"]["data_receipt_creation_config"];
        if let Some(f) = fee_of(&drc["base_cost"]) {
            live_fees.action_fees[near_parameters::ActionCosts::new_data_receipt_base] = f;
        }
        if let (Some(s), Some(n), Some(e)) = (
            drc["cost_per_byte"]["send_sir"].as_u64(),
            drc["cost_per_byte"]["send_not_sir"].as_u64(),
            drc["cost_per_byte"]["execution"].as_u64(),
        ) {
            live_fees.action_fees[near_parameters::ActionCosts::new_data_receipt_byte] =
                near_parameters::Fee::new(s, n, e);
        }
        fees = live_fees;
        println!("🔧 live protocol config: v{} (fix_contract_loading_cost={})", raw["result"]["protocol_version"], wasm_config.fix_contract_loading_cost);
    }
    let fees = Arc::new(fees);

    // Wrap ContractCode in the Contract trait the runner expects.
    struct CodeWrap(std::sync::Arc<ContractCode>);
    impl near_vm_runner::Contract for CodeWrap {
        fn hash(&self) -> near_primitives_core::hash::CryptoHash {
            *self.0.hash()
        }
        fn get_code(&self) -> Option<std::sync::Arc<ContractCode>> {
            Some(self.0.clone())
        }
    }
    let contract = CodeWrap(std::sync::Arc::new(ContractCode::new(wasm_bytes, None)));

    let gas_counter = context.make_gas_counter(&wasm_config);

    // Ensure the wasmtime backend is the configured VM kind.
    assert!(matches!(wasm_config.vm_kind, VMKind::Wasmtime));

    println!("▶ {}({})", method, if args_json == "{}" { "" } else { &args_json });

    let prepared = prepare(&contract, std::sync::Arc::new(wasm_config.clone()), None, gas_counter, &method);
    let outcome = run(prepared, &mut ext, &context, fees)
        .map_err(|e| format!("execution error: {e:?}"))?;

    // Report — same shapes as near-mock
    if let Some(err) = &outcome.aborted {
        println!("❌ {}", err);
    } else {
        println!("✅ Success");
        let returned = outcome.return_data.as_value().unwrap_or_default();
        if !returned.is_empty() {
            println!("📄 {}", String::from_utf8_lossy(&returned));
        }
        for log in &outcome.logs {
            println!("  LOG: {}", log);
        }
    }
    println!(
        "⛽ gas: {:.6} Tgas burnt / {:.6} Tgas prepaid",
        outcome.burnt_gas.as_gas() as f64 / 1e12,
        prepaid_tgas
    );

    // Persist storage snapshot (public fake_trie field)
    let store: HashMap<Vec<u8>, Vec<u8>> = ext.fake_trie.clone();
    let encoded = bincode::serialize(&store)?;
    std::fs::write(STATE_FILE, encoded)?;
    println!("💾 Saved {} keys", store.len());

    Ok(())
}
