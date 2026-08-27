//! Generic sandbox smoke runner — layer 3 of `scripts/verify.sh`.
//!
//! Usage: smoke <wasm> ['{"method": "...", "args": {...}}' ...]
//!        (JSON arg form: object with "method" and optional "args")
//!
//! Deploys the wasm on a real sandbox node and runs each call sequentially
//! (one shared state), printing results in near-mock format:
//!   method → 📄 <raw-return>  [⛽ X Tgas]
//! Exits 1 if any call FAILS at the transaction level. "" contract results
//! are contract logic (refusals), not runner failures.

use near_workspaces::types::Gas;
use serde_json::Value;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: smoke <wasm> [spec ...]  spec = {{\"method\":\"ft_mint\",\"args\":{{...}}}}");
        std::process::exit(2);
    }

    let wasm = std::fs::read(&args[0])?;
    let worker = near_workspaces::sandbox().await?;
    let contract = worker.dev_deploy(&wasm).await?;
    let holder = contract.id().to_string();
    println!("🏝️  deployed {} as {}", args[0], contract.id());

    // each step: {"method": "...", "args": {...}} — $HOLDER substitutes the
    // contract account id (needed because dev_deploy names are random)
    let mut failures = 0u32;
    for spec in &args[1..] {
        let v: Value = serde_json::from_str(spec)?;
        let method = v["method"].as_str().unwrap_or_default().to_string();
        let args_val = v.get("args").cloned().unwrap_or(serde_json::json!({}));
        let args_str = serde_json::to_string(&args_val)?;
        let args_str = args_str.replace("$HOLDER", &holder);
        let args_val: Value = serde_json::from_str(&args_str)?;

        let r = contract
            .call(&method)
            .args_json(args_val)
            .gas(Gas::from_tgas(300))
            .transact()
            .await?;
        let burnt = r.total_gas_burnt.as_gas();
        if r.is_success() {
            let out = String::from_utf8(r.into_result()?.raw_bytes()?)?;
            println!("{} → 📄 {}  [⛽ {:.6} Tgas]", method, out, burnt as f64 / 1e12);
        } else {
            failures += 1;
            println!("{} → ❌ FAILED  [⛽ {:.6} Tgas]", method, burnt as f64 / 1e12);
        }
    }
    if failures > 0 {
        anyhow::bail!("{failures} call(s) failed");
    }
    Ok(())
}
