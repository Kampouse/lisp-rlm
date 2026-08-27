//! Sandbox (real node, receipts, multi-account) tests for lisp-rlm contracts.
//!
//! Layer 3 of the test pyramid:
//!   near-mock (fast loop) → near-vm-run (VMLogic oracle) → sandbox-tests (this).
//!
//! Runs the deploy/*/target/*.wasm artifacts through a real sandbox node via
//! near-workspaces. Proves what neither lower layer can: multi-signer flows
//! (safe 2-of-3), real receipts, real transaction gas on the full stack.

use near_workspaces::types::Gas;
use serde_json::json;

const ERC20_WASM: &str = "../deploy/erc20/target/erc20.wasm";

#[tokio::test]
async fn erc20_lifecycle_on_sandbox() -> anyhow::Result<()> {
    let worker = near_workspaces::sandbox().await?;

    let wasm = std::fs::read(ERC20_WASM)?;
    let contract = worker.dev_deploy(&wasm).await?;

    let call = |method: &str, args: serde_json::Value| {
        contract
            .call(method)
            .args_json(args)
            .gas(Gas::from_tgas(300))
            .transact()
    };

    // On sandbox the caller/predecessor for contract.call() is the contract
    // account itself — mint to it so ft_transfer (which moves from the
    // predecessor) has funds.
    let holder = contract.id().to_string();
    let r = call("ft_mint", json!({"to": holder, "amount": "1000"})).await?;
    assert!(r.is_success(), "mint failed");
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1000"}"#);

    let r = call("ft_mint", json!({"to": "bob.near", "amount": "250"})).await?;
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1250"}"#);

    // transfer (predecessor = contract account itself — it holds the 1000)
    let r = call("ft_transfer", json!({"to": "bob.near", "amount": "400"})).await?;
    assert!(r.is_success(), "transfer failed: {:?}", format!("{:?}", r.is_failure()));
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1"}"#);

    // views
    let r = call("ft_balance_of", json!({"account": "bob.near"})).await?;
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "650"}"#);

    let r = call("ft_total_supply", json!({})).await?;
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1250"}"#);

    // overspend refusal: predecessor balance 600, try 9999
    let r = call("ft_transfer", json!({"to": "bob.near", "amount": "9999"})).await?;
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": ""}"#);

    println!("✅ erc20 lifecycle green on sandbox");
    Ok(())
}
