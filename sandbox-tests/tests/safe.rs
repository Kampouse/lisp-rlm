//! safe.lisp 2-of-3 multisig on a REAL sandbox node — the flow near-mock
//! structurally cannot prove (needs three distinct signers).
//!
//! Generates a safe variant with owners = three fresh dev accounts, deploys
//! it, then: propose (alice, auto-approve 1) → approve (bob, count 2) →
//! execute → payout. Plus refusal paths.

use near_workspaces::types::Gas;
use serde_json::json;

#[tokio::test]
async fn safe_two_of_three_happy_path() -> anyhow::Result<()> {
    let worker = near_workspaces::sandbox().await?;

    // Three fresh signers
    let alice = worker.dev_create_account().await?;
    let bob = worker.dev_create_account().await?;
    let carol = worker.dev_create_account().await?;
    let owners = format!("{},{},{}", alice.id(), bob.id(), carol.id());
    println!("owners: {owners}");

    // Generate + compile the safe variant with these owners
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let out = std::process::Command::new("python3")
        .args([
            repo_root.join("scripts/gen_deploy.py").to_string_lossy().to_string(),
            "safe-sandbox".to_string(),
            "--owners".to_string(),
            owners.clone(),
        ])
        .output()?;
    assert!(out.status.success(), "gen_deploy failed: {}", String::from_utf8_lossy(&out.stderr));
    let out = std::process::Command::new(repo_root.join("target/debug/near-compile"))
        .arg("build")
        .arg(repo_root.join("deploy/safe-sandbox"))
        .output()?;
    assert!(
        out.status.success(),
        "near-compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let wasm =
        std::fs::read(repo_root.join("deploy/safe-sandbox/target/safe-sandbox.wasm"))?;
    let contract = worker.dev_deploy(&wasm).await?;

    let r = contract
        .call("init")
        .args_json(json!({}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(r.is_success(), "init failed: {:?}", format!("{:?}", r.is_failure()));
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1"}"#);

    // propose as ALICE (predecessor = alice) → auto-approval, count 1
    let r = alice
        .call(contract.id(), "propose")
        .args_json(json!({"id": "tx1", "recipient": "dan.near", "amount": "500"}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(r.is_success(), "propose failed: {:?}", format!("{:?}", r.is_failure()));
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "500"}"#);

    // execute now (count 1 < 2) → refusal
    let r = alice
        .call(contract.id(), "execute")
        .args_json(json!({"id": "tx1", "recipient": "dan.near"}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(r.is_success());
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": ""}"#);

    // approve as BOB (count 2) → quorum
    let r = bob
        .call(contract.id(), "approve")
        .args_json(json!({"id": "tx1", "recipient": "dan.near"}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(r.is_success(), "approve failed: {:?}", format!("{:?}", r.is_failure()));
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1"}"#);

    // execute as CAROL → payout
    let r = carol
        .call(contract.id(), "execute")
        .args_json(json!({"id": "tx1", "recipient": "dan.near"}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(r.is_success(), "execute failed: {:?}", format!("{:?}", r.is_failure()));
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "1"}"#);

    // tx is gone after execution — views return zeroed state
    let r = alice
        .call(contract.id(), "tx_amount")
        .args_json(json!({"id": "tx1"}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
    assert_eq!(out, r#"{"result": "0"}"#);

    println!("✅ safe 2-of-3 happy path green on sandbox (propose→approve→execute→cleanup)");
    Ok(())
}
