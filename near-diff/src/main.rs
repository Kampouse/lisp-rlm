//! Differential tester: run one scenario.json on near-mock AND on a real
//! near-workspaces sandbox, diff per-step status / return value / gas.
//!
//! Usage: near-diff <scenario.json> [--near-mock <path>]
//!
//! Exit 1 on any disagreement (status or value). Gas deltas are reported
//! but never fail the run — they quantify the wasm-exec fuel model gap.

use near_workspaces::{network::Sandbox, Worker};
use serde_json::Value;

struct Step {
    acct: String,
    method: String,
    args: Value,
    signer: Option<String>,
    attach: Option<String>,
    note: String,
    skip_sandbox: bool,
    compare_value: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: near-diff <scenario.json> [--near-mock <path>]");
    let nm_path = args.iter().position(|a| a == "--near-mock")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "target/release/near-mock".into());
    let raw = std::fs::read_to_string(path).expect("read scenario");
    let sc: Value = serde_json::from_str(&raw).expect("parse scenario");

    let contracts: Vec<(String, String)> = sc["contracts"].as_object().expect("contracts")
        .iter().map(|(a, w)| (a.clone(), w.as_str().unwrap_or_default().to_string())).collect();
    let steps: Vec<Step> = sc["steps"].as_array().expect("steps").iter().filter_map(|s| {
        let call = s["call"].as_array()?;
        Some(Step {
            acct: call[0].as_str()?.into(),
            method: call[1].as_str()?.into(),
            args: call.get(2).cloned().unwrap_or(Value::Null),
            signer: s["signer"].as_str().map(String::from),
            attach: s["attach"].as_str().map(String::from),
            note: s["note"].as_str().unwrap_or("").into(),
            skip_sandbox: s["skipSandbox"].as_bool().unwrap_or(false),
            compare_value: s["compareValue"].as_bool().unwrap_or(true),
        })
    }).collect();

    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
        .block_on(async_main(&contracts, &steps, &nm_path));
}

async fn async_main(contracts: &[(String, String)], steps: &[Step], nm_path: &str) {
    // ── Sandbox side ──────────────────────────────────────────────
    let worker: Worker<Sandbox> = near_workspaces::sandbox().await.expect("sandbox worker");
    let root = worker.root_account().expect("root");
    let mut sandbox_accts: Vec<(String, near_workspaces::Account)> = Vec::new();
    for (acct, wasm) in contracts {
        let short = acct.split('.').next().unwrap_or(acct);
        let sub = root.create_subaccount(short).initial_balance(near_workspaces::types::NearToken::from_near(1000)).transact().await.expect("create acct").unwrap();
        let wasm_bytes = std::fs::read(wasm).expect("wasm read");
        sub.deploy(&wasm_bytes).await.expect("deploy").unwrap();
        sandbox_accts.push((acct.clone(), sub));
    }
    let mut signers: Vec<(String, near_workspaces::Account)> = Vec::new();
    let all_names: Vec<String> = steps.iter().filter_map(|s| s.signer.clone()).chain(
        steps.iter().filter_map(|s| s.args.get("account_id").and_then(|v| v.as_str()).map(String::from))
    ).chain(steps.iter().filter_map(|s| s.args.get("receiver_id").and_then(|v| v.as_str()).map(String::from)))
    .collect();
    for name in all_names {
        if signers.iter().any(|(n, _)| n == &name) { continue; }
        if let Some((_, c)) = sandbox_accts.iter().find(|(a, _)| a == &name) {
            signers.push((name.clone(), c.clone())); continue;
        }
        let short = name.split('.').next().unwrap_or(&name);
        if let Ok(sub) = root.create_subaccount(short).initial_balance(near_workspaces::types::NearToken::from_near(100)).transact().await {
            if let Ok(acc) = sub.into_result() { signers.push((name.clone(), acc)); }
        }
    }

    // ── near-mock side ────────────────────────────────────────────
    let state = "/tmp/near-diff.bin";
    let _ = std::fs::remove_file(state);
    let manifest_str = contracts.iter().map(|(a, w)| format!("{}={}", a, w)).collect::<Vec<_>>().join(",");

    println!("{:<3} {:28} {:>8} {:>10} {:>10}  {}", "#", "step", "status", "mock gas", "sbx gas", "notes");
    println!("{}", "─".repeat(90));
    let (mut agree, mut disagree) = (0, 0);
    for (i, st) in steps.iter().enumerate() {
        // mock
        let mock = run_mock(nm_path, state, &manifest_str, st);
        // sandbox
        let sbx = if st.skip_sandbox { None } else { Some(run_sandbox(&sandbox_accts, &signers, st).await) };
        let (m_ok, m_val, m_gas) = (mock.0, mock.1.clone(), Some(mock.2));
        let (s_ok, s_val, s_gas) = match &sbx {
            Some((a, b, c)) => (Some(*a), b.clone(), Some(*c)),
            None => (None, None, None),
        };

        let status_match = s_ok.map(|s| s == m_ok);
        let val_match = if st.compare_value {
            match (&s_val, &m_val) {
                (Some(s), Some(m)) => Some(norm(s) == norm(m)),
                _ => None,
            }
        } else { None };
        let bad = status_match == Some(false) || val_match == Some(false);
        if bad { disagree += 1; } else { agree += 1; }
        let mark = if bad { "❌" } else { "✅" };
        println!("{:<3} {:28} {:>8} {:>10} {:>10}  {} {}",
            i + 1,
            format!("{}.{}", short_acct(&st.acct), st.method),
            format!("{:?}/{:?}", m_ok, s_ok.unwrap_or(true)),
            format!("{:.3}T", m_gas.unwrap_or(0.0) / 1e12),
            format!("{:.3}T", s_gas.unwrap_or(0.0) / 1e12),
            mark,
            if st.note.is_empty() { String::new() } else { format!("{} ", st.note) }
                + &match (&val_match, &m_val, &s_val) {
                    (Some(false), m, s) => format!("VALUE mock={:?} sbx={:?}", m, s),
                    _ => String::new(),
                });
    }
    println!("{}", "─".repeat(90));
    println!("near-diff: {} agree, {} disagree", agree, disagree);
    if disagree > 0 { std::process::exit(1); }
}

fn short_acct(a: &str) -> String {
    a.split('.').next().unwrap_or(a).to_string()
}

fn norm(v: &Value) -> Value {
    // near-sdk borsh-JSON returns strings for u128; our 📄 line may carry
    // the same string quoted differently. Normalize numbers-as-strings.
    match v {
        Value::String(s) => {
            if let Ok(n) = s.parse::<u128>() { Value::String(n.to_string()) } else { v.clone() }
        }
        Value::Number(n) => Value::String(n.to_string()),
        other => other.clone(),
    }
}

/// (ok, value, gas)
fn run_mock(nm: &str, state: &str, manifest: &str, st: &Step) -> (bool, Option<Value>, f64) {
    let mut cmd = std::process::Command::new(nm);
    cmd.args(["cross", state, manifest, &st.acct, &st.method]).arg(st.args.to_string());
    if let Some(s) = &st.signer { cmd.env("NEAR_MOCK_SIGNER", s); }
    if let Some(a) = &st.attach { cmd.env("NEAR_MOCK_ATTACH", a); }
    let out = cmd.output().expect("run near-mock");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let ok = !text.contains("PANIC:") && !text.contains("❌");
    let val = text.lines().rev().find(|l| l.contains("📄")).and_then(|l| {
        let v = l.splitn(2, "📄").nth(1).unwrap_or("").trim();
        serde_json::from_str::<Value>(v).ok().or_else(|| Some(Value::String(v.to_string())))
    });
    let gas = text.lines().rev().find(|l| l.contains("⛽")).and_then(|l| {
        l.split_whitespace().find(|t| t.parse::<f64>().is_ok()).and_then(|t| t.parse::<f64>().ok())
    }).map(|t| t * 1e12).unwrap_or(0.0);
    (ok, val, gas)
}

async fn run_sandbox(accts: &[(String, near_workspaces::Account)], signers: &[(String, near_workspaces::Account)], st: &Step) -> (bool, Option<Value>, f64) {
    let contract = accts.iter().find(|(a, _)| a == &st.acct).map(|(_, c)| c.clone()).unwrap();
    let caller = st.signer.as_ref()
        .and_then(|s| signers.iter().find(|(n, _)| n == s).map(|(_, a)| a.clone()))
        .unwrap_or_else(|| accts.get(0).map(|(_, a)| a.clone()).unwrap());
    let mut call = caller.call(&contract.id(), &st.method).args_json(&st.args);
    if let Some(a) = &st.attach {
        if let Ok(y) = a.parse::<u128>() {
            call = call.deposit(near_workspaces::types::NearToken::from_yoctonear(y));
        }
    }
    let outcome = call.transact().await.expect("tx");
    let ok = outcome.is_success();
    if !ok {
        eprintln!("      [sbx] {}.{} failed: {}", st.acct, st.method,
            outcome.failures().iter().map(|f| format!("{:?}", f)).collect::<Vec<_>>().join("; "));
    }
    let gas = outcome.total_gas_burnt.as_gas() as f64;
    let val = outcome.into_result().ok().map(|r| r.json::<Value>().unwrap_or(Value::Null));
    (ok, val, gas)
}
