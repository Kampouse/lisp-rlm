#!/usr/bin/env python3
"""main() rewiring for near_mock.rs — flags, help, safe printer, --json, --dry-run."""

PATH = "/Users/asil/dev/lisp-rlm/src/bin/near_mock.rs"
R = []

# A1: help + flag parsing + RUN_CFG
R.append((
"""    if args.len() < 3 {
        eprintln!("Usage: near-mock <wasm> <method> [args-json]");
        eprintln!("       near-mock <wasm> exports|imports|reset");
        std::process::exit(1);
    }

    // Flags (parsed early — view/prepaid shape host fn construction)
    let run_view = args.iter().any(|a| a == "--view");
    let prepaid_tgas: f64 = args
        .iter()
        .position(|a| a == "--prepaid")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(200.0); // NEAR default prepaid gas per function call
    let prepaid_g: u64 = (prepaid_tgas * 1e12) as u64;
""",
"""    fn print_main_usage() {
        println!("near-mock — local NEAR contract runner (wasmtime, no node)");
        println!();
        println!("USAGE:");
        println!("  near-mock <wasm> <method> [args-json] [flags]");
        println!("  near-mock <wasm> exports|imports|reset");
        println!("  near-mock <wasm> symbolicate <idx-or-name> [map-file]");
        println!("  near-mock cross <state.bin> <acct=/path.wasm,...> <contract-acct> <method> [args-json]");
        println!();
        println!("ARGS:");
        println!("  <args-json>  JSON string, or @file for raw bytes (NUL/invalid UTF-8 ok)");
        println!();
        println!("FLAGS:");
        println!("  --view               read-only call (ProhibitedInView enforced, no persist)");
        println!("  --prepaid <TGAS>     prepaid gas, default 200");
        println!("  --deposit <yocto>    attached deposit (decimal yocto)");
        println!("  --gas-schedule <f>   JSON gas table (see: near-mock --gas-schedule-help)");
        println!("  --staking            enforce 1e20 yocto/byte storage staking");
        println!("  --dry-run            execute + report, do NOT persist state");
        println!("  --now <unix-secs>    fixed block_timestamp base (deterministic time)");
        println!("  --advance <secs>     time-travel: added to the --now base");
        println!("  --json               machine-readable result line (JSON {{...}})");
        println!("  --debug              verbose host traces ([schnorr-dbg], ptr/len)");
        println!("  --once               accepted no-op (kept for script compat)");
        println!();
        println!("ENV:");
        println!("  NEAR_MOCK_STATE       state file path (default /tmp/near-mock-state.bin)");
        println!("  NEAR_MOCK_ATTACH      attached deposit (decimal yocto)");
        println!("  NEAR_MOCK_SIGNER      signer account (default owner.test.near)");
        println!("  NEAR_MOCK_CONTRACT    contract account (default escrow.test.near)");
        println!("  NEAR_MOCK_NOW         fixed timestamp base (unix seconds)");
        println!("  NEAR_MOCK_SEED        pin random_seed (string, zero-padded to 64 hex)");
        println!("  NEAR_MOCK_DEBUG=1     same as --debug");
        println!("  NEAR_MOCK_WARN_STUBS=1  warn on unimplemented host stubs");
    }

    fn print_gas_schedule_default() {
        println!("{}", GasSchedule::default().to_json());
    }

    if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
        print_main_usage();
        return Ok(()); // exit 0 — scripts probe this for availability
    }
    if args.iter().any(|a| a == "--gas-schedule-help") {
        print_gas_schedule_default();
        return Ok(());
    }
    if args.len() < 3 {
        eprintln!("Usage: near-mock <wasm> <method> [args-json] [flags]");
        eprintln!("       near-mock <wasm> exports|imports|reset");
        eprintln!("       near-mock --help   (full flag reference)");
        std::process::exit(1);
    }

    fn hex_key(k: &[u8]) -> String {
        k.iter().map(|b| format!("{b:02x}")).collect()
    }

    // Flags (parsed early — view/prepaid shape host fn construction)
    let run_view = args.iter().any(|a| a == "--view");
    let flag_val = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let prepaid_tgas: f64 = flag_val("--prepaid")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(200.0); // NEAR default prepaid gas per function call
    let prepaid_g: u64 = (prepaid_tgas * 1e12) as u64;
    // --deposit <yocto>: wired into the same path the cross driver uses
    // (NEAR_MOCK_ATTACH) so attached_deposit() sees it. Was silently ignored.
    if let Some(d) = flag_val("--deposit") {
        std::env::set_var("NEAR_MOCK_ATTACH", d.trim());
    }
    let mut cfg = RunCfg::default();
    cfg.staking = args.iter().any(|a| a == "--staking");
    cfg.dry_run = args.iter().any(|a| a == "--dry-run");
    if args.iter().any(|a| a == "--debug") {
        cfg.debug = true;
    }
    if let Some(p) = flag_val("--gas-schedule") {
        cfg.gas =
            GasSchedule::from_json_file(p.trim()).map_err(|e| format!("--gas-schedule: {e}"))?;
        eprintln!("📏 gas schedule loaded from {}", p.trim());
    }
    if let Some(n) = flag_val("--now") {
        cfg.base_ts = Some(
            n.trim()
                .parse::<i64>()
                .map_err(|_| "--now must be unix seconds")?,
        );
    }
    if let Some(a) = flag_val("--advance") {
        cfg.advance_secs = a
            .trim()
            .parse::<i64>()
            .map_err(|_| "--advance must be seconds")?;
    }
    RUN_CFG.with(|c| *c.borrow_mut() = Some(cfg));
    let json_out = args.iter().any(|a| a == "--json");
"""))

# A2: method-not-found lists exports
R.append((
"""    // Call the target method
    let func = instance
        .get_func(&mut store, method)
        .ok_or_else(|| format!("Method '{}' not found", method))?;
""",
"""    // Call the target method
    let func = instance.get_func(&mut store, method).ok_or_else(|| {
        let mut avail: Vec<String> = module
            .exports()
            .filter_map(|e| match e.ty() {
                wasmtime::ExternType::Func(_) => Some(e.name().to_string()),
                _ => None,
            })
            .collect();
        avail.sort();
        format!(
            "Method '{}' not found. Available exports:\\n  {}",
            method,
            avail.join("\\n  ")
        )
    })?;
"""))

# A3a: outcome vars + safe_report open
R.append((
"""    match result {
        Ok(_) => {
            println!("✅ Success");
            let st = state.lock().unwrap();
            if let Some(ref data) = st.return_data {
""",
"""    let mut run_outcome: &str = "ok";
    let mut json_return: Option<String> = None;
    match result {
        Ok(_) => {
            println!("✅ Success");
            let st = state.lock().unwrap();
            // G-15: the result/storage printer must NEVER turn a successful
            // contract call into a process failure (exit 101 after a committed
            // mutation was exactly this bug class). Panic → report, exit 0.
            safe_report("result/storage printer", || {
            if let Some(ref data) = st.return_data {
"""))

# A3b: close safe_report + capture json return
R.append((
"""                    println!(
                        "  [{}b]={} → [{}b]={}",
                        k.len(),
                        kshow,
                        v.len(),
                        vshow
                    );
                }
            }
            // G-14: resolve receipts exactly like the cross driver — the
""",
"""                    println!(
                        "  [{}b]={} → [{}b]={}",
                        k.len(),
                        kshow,
                        v.len(),
                        vshow
                    );
                }
            }
            });
            json_return = st
                .return_data
                .as_ref()
                .map(|d| String::from_utf8_lossy(d).into_owned());
            // G-14: resolve receipts exactly like the cross driver — the
"""))

# A4: trap arm outcome
R.append((
"""            let msg = format!("{}", e);
            if msg.contains("all fuel consumed") {
                println!("❌ OutOfGas — exceeded {:.6} Tgas prepaid", prepaid_tgas);
            } else {
                println!("❌ {}", e);
""",
"""            let msg = format!("{}", e);
            if msg.contains("all fuel consumed") {
                run_outcome = "out_of_gas";
                println!("❌ OutOfGas — exceeded {:.6} Tgas prepaid", prepaid_tgas);
            } else {
                run_outcome = "trap";
                println!("❌ {}", e);
"""))

# A5: gas capture + diff + --json + conditional persist
R.append((
"""    // Gas report (1 fuel = 1 gas unit; host-call table is indicative-legacy)
    if let Ok(remaining) = store.get_fuel() {
        let burnt = prepaid_g.saturating_sub(remaining);
        println!(
            "⛽ gas: {:.6} Tgas burnt / {:.6} Tgas prepaid",
            burnt as f64 / 1e12,
            prepaid_tgas
        );
    }

    // Persist storage
    {
        let st = state.lock().unwrap();
        let encoded = bincode::serialize(&st.storage)?;
        std::fs::write(state_file(), encoded)?;
        println!("💾 Saved {} keys", st.storage.len());
    }

    Ok(())
}
""",
"""    // Gas report (1 fuel = 1 gas unit; host-call table is indicative-legacy)
    let mut gas_burnt = prepaid_g;
    if let Ok(remaining) = store.get_fuel() {
        gas_burnt = prepaid_g.saturating_sub(remaining);
        println!(
            "⛽ gas: {:.6} Tgas burnt / {:.6} Tgas prepaid",
            gas_burnt as f64 / 1e12,
            prepaid_tgas
        );
    }

    // Storage diff vs the pre-call snapshot (human summary + --json payload)
    let (added, changed, removed) = {
        let st = state.lock().unwrap();
        let mut added: Vec<(String, usize)> = Vec::new();
        let mut changed: Vec<(String, usize, usize)> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for (k, v) in &st.storage {
            match tx_snapshot.get(k) {
                None => added.push((hex_key(k), v.len())),
                Some(old) if old != v => changed.push((hex_key(k), old.len(), v.len())),
                _ => {}
            }
        }
        for k in tx_snapshot.keys() {
            if !st.storage.contains_key(k) {
                removed.push(hex_key(k));
            }
        }
        (added, changed, removed)
    };
    if !(added.is_empty() && changed.is_empty() && removed.is_empty()) {
        println!(
            "📦 diff: +{} ~{} -{} keys",
            added.len(),
            changed.len(),
            removed.len()
        );
    }

    // --json: one machine-readable blob for harnesses/CI assertions
    if json_out {
        let events = JSON_EVENTS.with(|e| e.borrow().clone());
        let log_count = LOG_COUNT.with(|l| *l.borrow());
        let st = state.lock().unwrap();
        let locked = if mock_cfg().staking {
            Some(locked_balance_for(&st, &exec_ctx_or_default().contract))
        } else {
            None
        };
        let j = serde_json::json!({
            "outcome": run_outcome,
            "return": json_return,
            "gas_burnt_tgas": gas_burnt as f64 / 1e12,
            "gas_prepaid_tgas": prepaid_tgas,
            "logs": log_count,
            "events": events,
            "storage": {
                "keys_total": st.storage.len(),
                "added": added,
                "changed": changed,
                "removed": removed,
                "locked_yocto": locked.map(|l| l.to_string()),
            },
            "dry_run": mock_cfg().dry_run,
        });
        println!("JSON {}", serde_json::to_string(&j).unwrap_or_default());
    }

    // Persist storage. --dry-run inspects without committing; --view never
    // persists either (real NEAR view calls can't write state).
    if mock_cfg().dry_run {
        println!("🏜  dry-run: state NOT persisted");
    } else if run_view {
        println!("👁  view call: state NOT persisted");
    } else {
        let st = state.lock().unwrap();
        let encoded = bincode::serialize(&st.storage)?;
        std::fs::write(state_file(), encoded)?;
        println!("💾 Saved {} keys", st.storage.len());
    }

    Ok(())
}
"""))

with open(PATH) as f:
    content = f.read()

failed = []
for i, (old, new) in enumerate(R, 1):
    n = content.count(old)
    if n != 1:
        failed.append(i)
        print(f"A{i}: MATCH COUNT {n} (expected 1) — SKIPPED")
    else:
        content = content.replace(old, new, 1)
        print(f"A{i}: applied")

if failed:
    print(f"\n{len(failed)} failed — file NOT written")
    raise SystemExit(1)

with open(PATH, "w") as f:
    f.write(content)
print("\nAll main() replacements applied.")
