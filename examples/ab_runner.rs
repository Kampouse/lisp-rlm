//! ab-runner: replay a steps file against a NEAR contract wasm under the REAL
//! near-vm-runner (0.37.3, wasmtime) with MockedExternal trie + real gas
//! metering, and emit one JSON line per step for offline diffing.
//!
//! Steps file format — one step per line, TAB-separated:
//!   METHOD <TAB> SIGNER <TAB> ATTACH_YOCTO <TAB> RAW_JSON_INPUT
//! Blank lines and lines starting with '#' are skipped.
//!
//! Usage: ab-runner <contract.wasm> <steps.tsv>

use std::sync::Arc;

use near_parameters::vm::VMKind;
use near_parameters::{RuntimeConfigStore, RuntimeFeesConfig};
use near_primitives_core::code::ContractCode;
use near_primitives_core::types::{Balance, Gas};
use near_vm_runner::internal::VMKindExt;
use near_vm_runner::logic::mocks::mock_external::MockedExternal;
use near_vm_runner::logic::VMContext;

const SELF: &str = "xcross-9f3.testnet";

struct Ctx {
    ext: MockedExternal,
}

fn ctx(signer: &str, input: &str, attach: u128) -> VMContext {
    VMContext {
        current_account_id: SELF.parse().unwrap(),
        signer_account_id: signer.parse().unwrap(),
        signer_account_pk: vec![0u8; 33],
        predecessor_account_id: signer.parse().unwrap(),
        refund_to_account_id: SELF.parse().unwrap(),
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
        prepaid_gas: Gas::from_teragas(500),
        random_seed: vec![0, 1, 2],
        view_config: None,
        output_data_receivers: vec![],
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn esc(s: &str) -> String {
    s.chars().map(|c| if c == '"' { '\'' } else { c }).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let wasm_path = args
        .get(1)
        .expect("usage: ab-runner <contract.wasm> <steps.tsv>");
    let steps_path = args
        .get(2)
        .expect("usage: ab-runner <contract.wasm> <steps.tsv>");
    let wasm = std::fs::read(wasm_path).expect("read wasm");

    let mut c = Ctx {
        ext: MockedExternal::with_code(ContractCode::new(wasm, None)),
    };

    let store = RuntimeConfigStore::test();
    let config = store
        .get_config(near_primitives_core::version::PROTOCOL_VERSION)
        .wasm_config
        .clone();
    let fees = Arc::new(RuntimeFeesConfig::test());

    let text = std::fs::read_to_string(steps_path).expect("read steps");
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() < 3 {
            println!("{{\"i\":{},\"error\":\"bad step line\"}}", i + 1);
            continue;
        }
        let method = parts[0];
        let signer = parts[1];
        let attach: u128 = parts[2].parse().unwrap_or(0);
        let input = parts.get(3).copied().unwrap_or("{}");

        let context = ctx(signer, input, attach);
        let gas_counter = context.make_gas_counter(&config);
        let runtime = VMKind::Wasmtime
            .runtime(config.clone())
            .expect("wasmtime runtime not compiled");
        let outcome = runtime.prepare(&c.ext, None, gas_counter, method).run(
            &mut c.ext,
            &context,
            Arc::clone(&fees),
        );

        match outcome {
            Ok(o) => {
                if let Some(err) = &o.aborted {
                    println!(
                        "{{\"i\":{},\"method\":\"{}\",\"ok\":false,\"gas\":{},\"err\":\"{}\"}}",
                        i + 1,
                        method,
                        o.burnt_gas.as_gas(),
                        esc(&err.to_string())
                    );
                } else {
                    let out = o.return_data.as_value().unwrap_or_default();
                    println!(
                        "{{\"i\":{},\"method\":\"{}\",\"ok\":true,\"gas\":{},\"out\":\"{}\"}}",
                        i + 1,
                        method,
                        o.burnt_gas.as_gas(),
                        esc(&hex(&out))
                    );
                }
            }
            Err(e) => println!(
                "{{\"i\":{},\"method\":\"{}\",\"ok\":false,\"gas\":0,\"err\":\"prepare: {}\"}}",
                i + 1,
                method,
                esc(&format!("{e:?}"))
            ),
        }
    }
}
