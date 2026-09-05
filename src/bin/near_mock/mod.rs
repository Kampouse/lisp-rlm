//! NEAR contract mock runner with state persistence.
//! Warms up wee_alloc by calling a cheap init method first.
//!
//! Usage:
//!   cargo run --bin near-mock -- <wasm> <method> [args-json] [--once] [--view] [--prepaid <TGAS>]
//!   cargo run --bin near-mock -- <wasm> exports|imports|reset
//!   cargo run --bin near-mock -- <wasm> symbolicate <idx-or-name> [map-file]
//!
//! Gas model (v2, 2026-08-27): wasmtime fuel, 1 fuel = 1 gas unit.
//! Host-call costs are indicative legacy NEAR fee-schedule values.
//! --view enforces ProhibitedInView on storage writes (see VMLogic).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use wasmtime::*;
use lisp_rlm_wasm::bls_validate;
use lisp_rlm_wasm::builtin_ed25519::ed25519_verify_impl;
use lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl;

// 2026-09-05 module split (item 11): hosts/gas/state/promises extracted so
// edits land in smaller files (the old 3.2k-line single file was where the
// fuzzy-patch tool did its worst damage).
#[path = "state.rs"]
mod state;
#[path = "gas.rs"]
mod gas;
#[path = "promises.rs"]
mod promises;
#[path = "hosts.rs"]
mod hosts;

pub(crate) use gas::{
    apply_staking_delta, locked_balance_for, splitmix64, stub_warn, trie_charge, trie_charge_write,
    GasSchedule, RunCfg, STAKING_COST_PER_BYTE,
};
pub(crate) use hosts::build_env_linker;
pub(crate) use promises::{dag_push, execute_promise, sub_execute, PAction, PromiseBatch};
pub(crate) use state::{
    prefixed_key, restore_partition, snapshot_partition, state_file, write_reg_checked, MockState,
};





// near-mock cross <state.bin> <acct=/path.wasm,...> <contract-acct> <method> [args-json]
//
// Multi-contract mode: manifest maps accounts to wasm files. Storage is
// per-account (prefixed keys in one state map). Promise DAGs execute
// synchronously after promise_return; sub-calls run in fresh Stores.
fn run_cross(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() < 5 {
        eprintln!(
            "Usage: near-mock cross <state.bin> <acct=wasm,...> <contract-acct> <method> [args-json]"
        );
        std::process::exit(1);
    }
    let state_path = &args[2];
    let manifest = &args[3];
    let contract_acct = &args[4];
    let method = &args[5];
    let args_json = args.get(6).cloned().unwrap_or_else(|| "{}".into());
    let run_view = args.iter().any(|a| a == "--view");

    let mut fuel_cfg = Config::new();
    fuel_cfg.consume_fuel(true);
    fuel_cfg.max_wasm_stack(64 * 1024 * 1024);
    fuel_cfg.async_stack_size(64 * 1024 * 1024);
    let engine = Rc::new(wasmtime::Engine::new(&fuel_cfg)?);

    let mut modules: HashMap<String, wasmtime::Module> = HashMap::new();
    for pair in manifest.split(',') {
        let (acct, path) = pair.split_once('=').ok_or("manifest entries must be acct=/path")?;
        let bytes = std::fs::read(path)?;
        eprintln!("📦 {} → {}", acct, path);
        modules.insert(acct.to_string(), wasmtime::Module::from_binary(&engine, &bytes)?);
    }

    let loaded_storage: HashMap<Vec<u8>, Vec<u8>> = std::fs::read(state_path)
        .ok()
        .and_then(|d| bincode::deserialize(&d).ok())
        .unwrap_or_default();
    if loaded_storage.is_empty() {
        println!("🆕 Fresh state");
    } else {
        println!("📂 Loaded {} storage keys", loaded_storage.len());
    }
    let state: Arc<Mutex<MockState>> = Arc::new(Mutex::new(MockState {
        storage: loaded_storage,
        touched: Default::default(),
        registers: HashMap::new(),
        return_data: None,
        view: run_view,
    }));

    MODULES.with(|m| *m.borrow_mut() = Some(Arc::new(modules)));
    STATE_ARC.with(|s| *s.borrow_mut() = Some(state.clone()));
    ENGINE_TLS.with(|e| *e.borrow_mut() = Some(engine.clone()));

    let signer = std::env::var("NEAR_MOCK_SIGNER").unwrap_or_else(|_| "caller.test.near".into());
    EXEC_CTX.with(|c| {
        *c.borrow_mut() = Some(ExecCtx {
            input: args_json.clone().into_bytes(),
            signer: signer.clone(),
            predecessor: signer.clone(),
            contract: contract_acct.clone(),
            view: run_view,
        })
    });

    let module = MODULES
        .with(|m| m.borrow().as_ref().unwrap().get(contract_acct).cloned())
        .ok_or(format!("contract account {} not in manifest", contract_acct))?;

    // Attached deposit (NEAR_MOCK_ATTACH=decimal yocto) — credited to the
    // callee's NEAR balance before the entry runs, like a real receipt.
    if let Ok(attach) = std::env::var("NEAR_MOCK_ATTACH") {
        if !attach.is_empty() {
            let amt: u128 = attach.trim().parse().map_err(|_| "NEAR_MOCK_ATTACH must be decimal yocto")?;
            let state0 = state.lock().unwrap();
            let key = prefixed_key(contract_acct, b"\x00near-bal");
            let bal: u128 = state0
                .storage
                .get(&key)
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u128);
            let key_owned = key.clone();
            drop(state0);
            let mut state0 = state.lock().unwrap();
            state0.storage.insert(key_owned, (bal + amt).to_string().into_bytes());
            eprintln!("  💰 attached {} yocto → {} (bal {})", amt, contract_acct, bal + amt);
        }
    }

    let mut store = wasmtime::Store::new(&*engine, ());
    store.set_fuel(PREPAID_FUEL.with(|f| *f.borrow()))?;
    let linker = build_env_linker(&mut store, &*engine, state.clone(), args_json.clone().into_bytes())?;
    let instance = linker.instantiate(&mut store, &module)?;

    let func = instance
        .get_func(&mut store, method)
        .ok_or_else(|| format!("Method '{}' not found", method))?;
    println!("▶ {}.{}({})", contract_acct, method, if args_json == "{}" { "".into() } else { args_json.clone() });
    let result = func.call(&mut store, &[], &mut []);
    let tx_snapshot: HashMap<Vec<u8>, Vec<u8>> = {
        // taken AFTER attach credit — the deposit is part of the tx; a
        // failed tx refunds it (NEAR: attached deposit returns on failure)
        let st = state.lock().unwrap();
        let mut s2: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for (k, v) in st.storage.iter() {
            if k != &prefixed_key(contract_acct, b"\x00near-bal") || result.is_ok() {
                // keep pre-entry balances only when the entry failed;
                // on success we snapshot post-attach (deposit sticks)
            }
            s2.insert(k.clone(), v.clone());
        }
        if result.is_err() {
            // entry failed: snapshot WITHOUT the attach credit → full refund
            if let Ok(attach) = std::env::var("NEAR_MOCK_ATTACH") {
                if let Ok(amt) = attach.trim().parse::<u128>() {
                    let key = prefixed_key(contract_acct, b"\x00near-bal");
                    if let Some(v) = s2.get(&key).cloned() {
                        let bal: u128 = String::from_utf8_lossy(&v).trim().parse().unwrap_or(0);
                        let pre_bal = bal.saturating_sub(amt);
                        if pre_bal > 0 {
                            s2.insert(key, pre_bal.to_string().into_bytes());
                        } else {
                            s2.remove(&key);
                        }
                    }
                }
            }
        }
        s2
    };

    match result {
        Ok(_) => {
            println!("✅ Success");
            // Entry return-data is authoritative ONLY when no promise was
            // returned; with a promise the callback's result is the tx result.
            let pending = PENDING_RETURN.with(|p| *p.borrow());
            if pending.is_none() {
                let st = state.lock().unwrap();
                if let Some(ref data) = st.return_data {
                    let s = String::from_utf8_lossy(data);
                    if !s.is_empty() {
                        println!("📄 {}", s);
                    }
                }
            }
            // Resolve any promise returned by the entry
            if let Some(idx) = pending {
                eprintln!("  ⛓ resolving promise DAG (root {})", idx);
                let dag = execute_promise(idx);
                if let Err(e) = &dag {
                    println!("❌ receipt chain failed: {}", e);
                    println!("   ↺ full rollback (single tx = atomic)");
                    let mut st = state.lock().unwrap();
                    st.storage = tx_snapshot;
                    drop(st);
                    let st = state.lock().unwrap();
                    let mut keys: Vec<(&Vec<u8>, &Vec<u8>)> = st.storage.iter().collect();
                    keys.sort();
                    println!("💾 Saved {} keys (rolled back)", keys.len());
                    let encoded = bincode::serialize(&st.storage)?;
                    std::fs::write(state_path, encoded)?;
                    return Ok(());
                }
                let results = dag.unwrap();
                let last = results.iter().rev().find_map(|r| r.as_ref().cloned());
                if let Some(bytes) = last {
                    let s = String::from_utf8_lossy(&bytes);
                    if !s.is_empty() {
                        println!("📄 {}", s);
                    }
                }
            }
            // Fire-and-forget receipts (2026-09-02): batches created but not
            // part of any returned DAG still execute on-chain as independent
            // receipts. The mock used to drop them silently — a promise to a
            // phantom account vanished instead of failing like live NEAR
            // (nostr-gov tk="nil" shipped through every gate). Drain any
            // unexecuted batches in creation order; their failures do NOT
            // roll back the parent tx (receipt independence).
            loop {
                let next = PROMISE_DAG.with(|d| {
                    d.borrow()
                        .iter()
                        .enumerate()
                        .find(|(i, _)| !EXECUTED_PROMISES.with(|e| e.borrow().contains(i)))
                        .map(|(i, _)| i)
                });
                let Some(idx) = next else { break };
                eprintln!("  ⛓ orphan receipt {} (fire-and-forget)", idx);
                match execute_promise(idx) {
                    Ok(_) => {}
                    Err(e) => println!(
                        "❌ orphan receipt failed: {} (parent tx stays committed)",
                        e
                    ),
                }
            }
        }
        Err(e) => {
            println!("❌ {}", e);
            println!("   ↳ debug: {:?}", e);
            println!("   ↺ entry trapped — full rollback (single tx = atomic)");
            let mut st = state.lock().unwrap();
            st.storage = tx_snapshot;
        }
    }

    // Persist
    {
        let st = state.lock().unwrap();
        if !st.storage.is_empty() {
            let mut keys: Vec<(&Vec<u8>, &Vec<u8>)> = st.storage.iter().collect();
            keys.sort();
            println!("💾 Saved {} keys", keys.len());
            let encoded = bincode::serialize(&st.storage)?;
            std::fs::write(state_path, encoded)?;
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// Cross-contract engine (2026-09-01)
//
// NEAR promise semantics, executed synchronously (deterministic):
//   promise_return(idx) marks the DAG root; the runtime then resolves
//   deps depth-first, runs each batch's actions as fresh sub-executions
//   (fresh Store/Instance — no re-entrancy into a live instance, which
//   would clobber the shared heap-pointer global), and delivers dep
//   results to callbacks via promise_result(i).
//
// Storage is PER-ACCOUNT (NEAR trie model): keys are prefixed
// "<account>\x01<key>" in the one shared state map. Empty contract
// prefix = single-contract mode, byte-compatible with old state files.
// A trapping sub-execution REVERTS its account partition (NEAR failed
// receipts discard state changes).
// ═══════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct ExecCtx {
    input: Vec<u8>,
    signer: String,
    predecessor: String,
    contract: String,
    view: bool,
}



thread_local! {
    static PREPAID_FUEL: std::cell::RefCell<u64> = const { std::cell::RefCell::new(200 * 1_000_000_000_000) };
    static EXEC_CTX: std::cell::RefCell<Option<ExecCtx>> = const { std::cell::RefCell::new(None) };
    static PROMISE_DAG: std::cell::RefCell<Vec<PromiseBatch>> = const { std::cell::RefCell::new(Vec::new()) };
    static PROMISE_RESULTS: std::cell::RefCell<Vec<Option<Vec<u8>>>> = const { std::cell::RefCell::new(Vec::new()) };
    static PENDING_RETURN: std::cell::RefCell<Option<usize>> = const { std::cell::RefCell::new(None) };
    static MODULES: std::cell::RefCell<Option<std::sync::Arc<HashMap<String, wasmtime::Module>>>> =
        const { std::cell::RefCell::new(None) };
    static STATE_ARC: std::cell::RefCell<Option<std::sync::Arc<Mutex<MockState>>>> =
        const { std::cell::RefCell::new(None) };
    static ENGINE_TLS: std::cell::RefCell<Option<std::rc::Rc<wasmtime::Engine>>> =
        const { std::cell::RefCell::new(None) };
}

fn exec_ctx_or_default() -> ExecCtx {
    EXEC_CTX.with(|c| c.borrow().clone()).unwrap_or(ExecCtx {
        input: b"{}".to_vec(),
        signer: "owner.test.near".into(),
        predecessor: "owner.test.near".into(),
        contract: String::new(),
        view: false,
    })
}



// batches already resolved this run (returned-DAG traversal + orphan drain)
thread_local! {
    static EXECUTED_PROMISES: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}





impl Default for RunCfg {
    fn default() -> Self {
        RunCfg {
            gas: GasSchedule::default(),
            staking: false,
            dry_run: false,
            debug: std::env::var("NEAR_MOCK_DEBUG").map(|v| v == "1").unwrap_or(false),
            warn_stubs: std::env::var("NEAR_MOCK_WARN_STUBS").map(|v| v == "1").unwrap_or(false),
            base_ts: std::env::var("NEAR_MOCK_NOW").ok().and_then(|s| s.parse().ok()),
            advance_secs: 0,
        }
    }
}

thread_local! {
    static RUN_CFG: std::cell::RefCell<Option<RunCfg>> = const { std::cell::RefCell::new(None) };
}

fn mock_cfg() -> RunCfg {
    RUN_CFG.with(|c| c.borrow().clone()).unwrap_or_default()
}

/// Effective block timestamp in ns: (base_ts + advance) scaled, real clock
/// otherwise. --now/--advance (or NEAR_MOCK_NOW) make time-based contracts
/// deterministic; scripts time-travel by re-invoking with --advance.
fn mock_now_nanos() -> i64 {
    let c = mock_cfg();
    let base = c.base_ts.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    });
    (base + c.advance_secs) * 1_000_000_000
}



// ============ NEP-297 event capture + log counting (--json) ============
thread_local! {
    /// Event JSON strings (NEP-297 EVENT_JSON: logs), for --json output.
    static JSON_EVENTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static LOG_COUNT: std::cell::RefCell<usize> = const { std::cell::RefCell::new(0) };
}

/// Route one decoded log line: NEP-297 EVENT_JSON: gets structured decoding,
/// everything else prints as LOG. Never panics on weird payloads.
fn handle_log_line(msg: &str, debug: bool, suffix: &str) {
    LOG_COUNT.with(|c| *c.borrow_mut() += 1);
    if let Some(rest) = msg.strip_prefix("EVENT_JSON:") {
        match serde_json::from_str::<serde_json::Value>(rest) {
            Ok(v) => {
                JSON_EVENTS.with(|e| e.borrow_mut().push(v.to_string()));
                let std_name = v.get("standard").and_then(|x| x.as_str()).unwrap_or("?");
                let ver = v.get("version").and_then(|x| x.as_str()).unwrap_or("?");
                let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("?");
                let data = v.get("data").map(|d| d.to_string()).unwrap_or_default();
                println!("  📣 EVENT {std_name} v{ver} :: {ev} {data}");
            }
            Err(_) => println!("  LOG: {msg}{suffix} (EVENT_JSON but malformed)"),
        }
    } else if debug {
        println!("  LOG: {msg}{suffix}");
    } else {
        println!("  LOG: {msg}");
    }
}

/// Run a pretty-printing section with panic containment: a reporting bug must
/// never eat a successful run (the 2026-09-05 storage-dump char-boundary
/// panic turned ✅ contract successes into exit 101).
fn safe_report<F: FnOnce()>(label: &str, f: F) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    if r.is_err() {
        eprintln!("⚠ {label}: reporting section panicked (contract result unaffected)");
    }
}





/// Execute one function call on `account`'s contract in a FRESH Store
/// (never re-enter a live instance — the heap global would be clobbered).
/// Signer/predecessor = `predecessor` (promise calls aren't user-signed).
/// Returns Some(return-bytes) on success, None on trap (state reverted).
thread_local! {
    /// The CURRENT receipt's attached deposit. Set by sub_execute for
    /// batch function-call children (was: silently dropped — dep_ptr was
    /// read nowhere; 2026-09-01). Top-level entries fall back to
    /// NEAR_MOCK_ATTACH via the host fn.
    static CURRENT_DEPOSIT: std::cell::RefCell<Option<u128>> = const { std::cell::RefCell::new(None) };
}



// ── real promise hosts (cross engine) ──
fn mem_read_str(caller: &mut wasmtime::Caller<'_, ()>, len: i64, ptr: i64) -> Option<String> {
    let len = len as usize;
    let ptr = ptr as usize;
    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
        let md = mem.data(&caller);
        if ptr + len <= md.len() {
            return Some(String::from_utf8_lossy(&md[ptr..ptr + len]).into_owned());
        }
    }
    None
}

/// Read `mem[ptr..ptr+len]` from guest memory, or None on OOB. Mock
/// equivalent of nearcore's `get_memory_or_register!` (which traps with
/// MemoryAccessViolation when ptr+len exceeds memory).
fn read_guest_bytes(caller: &mut wasmtime::Caller<'_, ()>, len: i64, ptr: i64) -> Option<Vec<u8>> {
    let (len, ptr) = (len as usize, ptr as usize);
    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
        let md = mem.data(&caller);
        if ptr + len <= md.len() {
            return Some(md[ptr..ptr + len].to_vec());
        }
    }
    None
}

#[allow(clippy::type_complexity)]
fn build_promise_hosts(
    store: &mut wasmtime::Store<()>,
    engine: &wasmtime::Engine,
) -> Result<
    (
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
        wasmtime::Func,
    ),
    Box<dyn std::error::Error>,
> {
    // 39 promise_batch_create(acct_len, acct_ptr) -> idx
    let pbc = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let acct = mem_read_str(&mut caller, args[0].unwrap_i64(), args[1].unwrap_i64())
                .unwrap_or_default();
            eprintln!("  → promise_batch_create({}) [dag]", acct);
            results[0] = Val::I64(dag_push(vec![], acct, vec![]) as i64);
            Ok(())
        },
    );
    // 40 promise_batch_then(idx, acct_len, acct_ptr) -> new idx
    let pbt = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
        move |mut caller, args, results| {
            let idx = args[0].unwrap_i64() as usize;
            let acct = mem_read_str(&mut caller, args[1].unwrap_i64(), args[2].unwrap_i64())
                .unwrap_or_default();
            results[0] = Val::I64(dag_push(vec![idx], acct, vec![]) as i64);
            Ok(())
        },
    );
    // 43 promise_batch_action_function_call(idx, m_len, m_ptr, a_len, a_ptr, dep_ptr, gas)
    let pafc = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![]),
        move |mut caller, args, _| {
            let idx = args[0].unwrap_i64() as usize;
            let method = mem_read_str(&mut caller, args[1].unwrap_i64(), args[2].unwrap_i64())
                .unwrap_or_default();
            let args_json = mem_read_str(&mut caller, args[3].unwrap_i64(), args[4].unwrap_i64())
                .unwrap_or_default();
            let gas = args[6].unwrap_i64() as u64;
            let dep = {
                let ptr = args[5].unwrap_i64() as usize;
                let mut buf = [0u8; 16];
                if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let md = mem.data(&caller);
                    if ptr + 16 <= md.len() {
                        buf.copy_from_slice(&md[ptr..ptr + 16]);
                    }
                }
                u128::from_le_bytes(buf)
            };
            eprintln!(
                "  → action_fn_call(idx={}, {} args={} dep={})",
                idx, method, args_json, dep
            );
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::FnCall { method, args: args_json.into_bytes(), gas, dep });
                }
            });
            Ok(())
        },
    );
    // 44 promise_batch_action_transfer(idx, amt_ptr) — u128 LE at ptr (16 bytes)
    let pbat = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let idx = args[0].unwrap_i64() as usize;
            let amt = {
                let ptr = args[1].unwrap_i64() as usize;
                let len = 16usize;
                let mut buf = [0u8; 16];
                if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let md = mem.data(&caller);
                    if ptr + len <= md.len() {
                        buf[..len].copy_from_slice(&md[ptr..ptr + len]);
                    }
                }
                u128::from_le_bytes(buf)
            };
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::Transfer(amt));
                }
            });
            Ok(())
        },
    );
    // 82 promise_yield_create(m_len, m_ptr, a_len, a_ptr, gas, weight, reg) -> idx
    // data_id ("yd:<idx>") lands in the register; the promise index IS the
    // resume handle (documented mock simplification of NEAR's opaque data_id).
    let pyc = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![ValType::I64]),
        move |mut caller, args, results| {
            let method = mem_read_str(&mut caller, args[0].unwrap_i64(), args[1].unwrap_i64())
                .unwrap_or_default();
            let args_json = mem_read_str(&mut caller, args[2].unwrap_i64(), args[3].unwrap_i64())
                .unwrap_or_default();
            let reg = args[6].unwrap_i64() as u64;
            let contract = exec_ctx_or_default().contract;
            eprintln!("  → promise_yield_create({} args={}) on {}", method, args_json, contract);
            let batch_creator = exec_ctx_or_default().contract;
            let args_bytes = args_json.clone().into_bytes();
            let idx = {
                PROMISE_DAG.with(|d| {
                    let mut d = d.borrow_mut();
                    d.push(PromiseBatch {
                        deps: vec![],
                        account: contract.clone(),
                        creator: batch_creator.clone(),
                        actions: vec![PAction::FnCall {
                            method: method.clone(),
                            args: args_bytes.clone(),
                            gas: args[4].unwrap_i64() as u64,
                            dep: 0,
                        }],
                        is_yield: true,
                    });
                    d.len() - 1
                })
            };
            let did = format!("yd:{}", idx);
            let st = STATE_ARC.with(|s| s.borrow().clone());
            if let Some(st) = st {
                let mut st = st.lock().unwrap();
                st.registers.insert(reg, did.into_bytes());
                // persist: \x00yield:<idx> = account \x1f method \x1f creator \x1f args_json
                let spec = format!("{}\x1f{}\x1f{}\x1f{}", contract, method, batch_creator, args_json);
                let key = format!("\x00yield:{}", idx);
                st.storage.insert(key.into_bytes(), spec.into_bytes());
            }
            results[0] = Val::I64(idx as i64);
            Ok(())
        },
    );
    // 83 promise_yield_resume(idx, p_len, p_ptr) -> 1/0
    // NOTE: the host-table ABI is (i64 x4) — idx, d_len, d_ptr, p_len, p_ptr?
    // The table says 4 i64 params; emitter pushes (idx, d_len, d_ptr, p_len, p_ptr)?
    // Keep 4: (idx, payload_len, payload_ptr, _pad) — see emitter's actual pushes.
    let pyr = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![ValType::I64]),
        move |mut caller, args, results| {
            // ABI: (data_id_len, data_id_ptr, payload_len, payload_ptr) — the
            // emitter passes the data_id as a STRING ("yd:<idx>" or "<idx>")
            let data_id = mem_read_str(&mut caller, args[0].unwrap_i64(), args[1].unwrap_i64())
                .unwrap_or_default();
            let payload = mem_read_str(&mut caller, args[2].unwrap_i64(), args[3].unwrap_i64())
                .unwrap_or_default();
            let idx: usize = data_id
                .trim_start_matches("yd:")
                .parse()
                .unwrap_or(usize::MAX);
            let dag = PROMISE_DAG.with(|d| d.borrow().clone());
            let batch = match dag.get(idx) {
                Some(b) if b.is_yield => b.clone(),
                _ => {
                    // cross-process resume: the spec lives in the state file
                    let st = STATE_ARC.with(|s| s.borrow().clone());
                    let key = format!("\x00yield:{}", idx);
                    let spec = st
                        .as_ref()
                        .and_then(|st| st.lock().unwrap().storage.get(key.as_bytes()).cloned());
                    match spec {
                        Some(bytes) => {
                            let s = String::from_utf8_lossy(&bytes).to_string();
                            let parts: Vec<&str> = s.split('\x1f').collect();
                            if parts.len() == 4 {
                                PromiseBatch {
                                    deps: vec![],
                                    account: parts[0].to_string(),
                                    creator: parts[2].to_string(),
                                    actions: vec![PAction::FnCall {
                                        method: parts[1].to_string(),
                                        args: parts[3].as_bytes().to_vec(),
                                        gas: 0,
                                        dep: 0,
                                    }],
                                    is_yield: true,
                                }
                            } else {
                                eprintln!("  ⚠ yield_resume: bad persisted spec at {}", idx);
                                results[0] = Val::I64(0);
                                return Ok(());
                            }
                        }
                        None => {
                            eprintln!("  ⚠ yield_resume: idx {} is not a yield promise", idx);
                            results[0] = Val::I64(0);
                            return Ok(());
                        }
                    }
                }
            };
            eprintln!("  ⏵ yield_resume({}) payload={}", idx, payload);
            let (method, args_json, _) = match batch.actions.first() {
                Some(PAction::FnCall { method, args, gas, .. }) => (method.clone(), args.clone(), gas),
                _ => {
                    eprintln!("  ⚠ yield_resume: no callback action on idx {}", idx);
                    results[0] = Val::I64(0);
                    return Ok(());
                }
            };
            // Re-run the callback with the payload as the Successful result
            let saved = PROMISE_RESULTS.with(|r| std::mem::replace(
                &mut *r.borrow_mut(),
                vec![Some(payload.into_bytes())],
            ));
            let ret = sub_execute(&batch.account, &method, &args_json, &batch.creator, 0);
            PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
            // one-shot: consume the persisted yield handle
            {
                let st = STATE_ARC.with(|s| s.borrow().clone());
                if let Some(st) = st {
                    st.lock().unwrap().storage.remove(format!("\x00yield:{}", idx).as_bytes());
                }
            }
            match ret {
                Ok(Some(bytes)) => {
                    let s = String::from_utf8_lossy(&bytes);
                    if !s.is_empty() {
                        println!("📄 (yield) {}", s);
                    }
                }
                Ok(None) => eprintln!("  ⚠ yield callback trapped"),
                Err(e) => eprintln!("  ⚠ yield callback error: {}", e),
            }
            results[0] = Val::I64(1);
            Ok(())
        },
    );
    // 30 promise_create
    let pc = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 8], vec![ValType::I64]),
        move |mut caller, args, results| {
            let acct = mem_read_str(&mut caller, args[0].unwrap_i64(), args[1].unwrap_i64())
                .unwrap_or_default();
            let method = mem_read_str(&mut caller, args[2].unwrap_i64(), args[3].unwrap_i64())
                .unwrap_or_default();
            let args_json = mem_read_str(&mut caller, args[4].unwrap_i64(), args[5].unwrap_i64())
                .unwrap_or_default();
            let idx = dag_push(
                vec![],
                acct,
                vec![PAction::FnCall { method, args: args_json.into_bytes(), gas: args[7].unwrap_i64() as u64, dep: 0 }],
            );
            results[0] = Val::I64(idx as i64);
            Ok(())
        },
    );
    // 31 promise_then
    let pt = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![ValType::I64]),
        move |mut caller, args, results| {
            let idx = args[0].unwrap_i64() as usize;
            let acct = mem_read_str(&mut caller, args[1].unwrap_i64(), args[2].unwrap_i64())
                .unwrap_or_default();
            let method = mem_read_str(&mut caller, args[3].unwrap_i64(), args[4].unwrap_i64())
                .unwrap_or_default();
            let args_json = mem_read_str(&mut caller, args[5].unwrap_i64(), args[6].unwrap_i64())
                .unwrap_or_default();
            let new_idx = dag_push(
                vec![idx],
                acct,
                vec![PAction::FnCall { method, args: args_json.into_bytes(), gas: args[8].unwrap_i64() as u64, dep: 0 }],
            );
            results[0] = Val::I64(new_idx as i64);
            Ok(())
        },
    );
    // 32 promise_and(ptr, count) -> idx
    let pa = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let ptr = args[0].unwrap_i64() as usize;
            let count = args[1].unwrap_i64() as usize;
            let mut deps = Vec::new();
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                for i in 0..count {
                    let off = ptr + i * 8;
                    if off + 8 <= md.len() {
                        deps.push(u64::from_le_bytes(md[off..off + 8].try_into().unwrap()) as usize);
                    }
                }
            }
            results[0] = Val::I64(dag_push(deps, String::new(), vec![]) as i64);
            Ok(())
        },
    );
    // 33 promise_results_count
    let prc = Func::new(
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, results| {
            results[0] = Val::I64(PROMISE_RESULTS.with(|r| r.borrow().len() as i64));
            Ok(())
        },
    );
    // 34 promise_result(idx, reg) -> status
    let state_for_pr = STATE_ARC.with(|s| s.borrow().clone());
    let pr = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |_, args, results| {
            let idx = args[0].unwrap_i64() as usize;
            let rid = args[1].unwrap_i64() as u64;
            let entry = PROMISE_RESULTS.with(|r| r.borrow().get(idx).cloned());
            match entry {
                None => results[0] = Val::I64(0),
                Some(Some(bytes)) => {
                    if let Some(st) = &state_for_pr {
                        let mut st = st.lock().unwrap();
                        let _ = write_reg_checked(&mut st, rid, bytes);
                    }
                    results[0] = Val::I64(1);
                }
                Some(None) => results[0] = Val::I64(2),
            }
            Ok(())
        },
    );
    // 35 promise_return(idx)
    let pret = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        |_, args, _| {
            PENDING_RETURN.with(|p| *p.borrow_mut() = Some(args[0].unwrap_i64() as usize));
            eprintln!("  → promise_return({})", args[0].unwrap_i64());
            Ok(())
        },
    );
    Ok((pc, pt, pa, prc, pr, pret, pbc, pbt, pafc, pbat, pyc, pyr))
}

fn exec_ctx_view(state: &std::sync::Arc<Mutex<MockState>>) -> bool {
    EXEC_CTX.with(|c| c.borrow().as_ref().map(|x| x.view)).unwrap_or_else(|| state.lock().unwrap().view)
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("cross") {
        return run_cross(&args);
    }
    fn print_main_usage() {
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
        println!("  --state <path>        state file (default /tmp/near-mock-state.bin; = NEAR_MOCK_STATE)");
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
    // --state <path> mirrors the NEAR_MOCK_STATE env var (same single source
    // of truth); flag wins over a pre-set env value.
    if let Some(p) = flag_val("--state") {
        std::env::set_var("NEAR_MOCK_STATE", p.trim());
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

    let wasm_path = &args[1];
    let method = &args[2];
    // args: literal JSON, or "@file" to read raw bytes from a file (input
    // fuzzing needs NUL bytes / invalid UTF-8 / >100KB payloads that cannot
    // ride argv safely). Raw bytes: file content is passed through to the
    // input register EXACTLY as-is (no UTF-8 validation, unlike argv).
    let args_bytes: Vec<u8> = match args.get(3) {
        Some(s) if s.starts_with('@') => {
            std::fs::read(&s[1..]).unwrap_or_else(|e| {
                eprintln!("failed to read args file {}: {}", &s[1..], e);
                std::process::exit(2);
            })
        }
        other => other
            .cloned()
            .unwrap_or_else(|| "{}".to_string())
            .into_bytes(),
    };

    if method == "reset" {
        let _ = std::fs::remove_file(state_file());
        println!("🗑️  State cleared");
        return Ok(());
    }

    // near-mock <wasm> symbolicate <idx-or-name> [map-file]
    // Resolve a trap frame ("wasm function 22" or a name-section name like
    // "run:run") to its source form via the compile-time .wasm.map sidecar.
    // Also serves testnet traps: download the deployed wasm, keep the .wasm.map
    // you compiled with, and decode locally — same name section everywhere.
    if method == "symbolicate" {
        let target = args
            .get(3)
            .map(|s| s.trim_matches('"').to_string())
            .ok_or("symbolicate: need <idx-or-name> [map-file]?")?;
        let map_path = args
            .get(4)
            .map(|s| s.clone())
            .unwrap_or_else(|| format!("{}.map", wasm_path));
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&map_path).map_err(|e| {
                format!("cannot read sidecar {}: {} (compile with ./target/release/compile)", map_path, e)
            })?)
            .map_err(|e| format!("bad sidecar {}: {}", map_path, e))?;
        let wasm_bytes = std::fs::read(wasm_path)?;
        let names =
            lisp_rlm_wasm::wasm_emit::name_map::decode_function_names(&wasm_bytes)
                .unwrap_or_default();
        // Resolve: numeric index → name via the section; otherwise direct name
        // match ("run:run" or "run"); wrapper names match their inner fn.
        let key: Option<String> = if let Ok(idx) = target.parse::<u32>() {
            names
                .iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, n)| n.clone())
        } else {
            Some(target.to_string())
        };
        let resolve = |k: &str| -> Option<&str> {
            if let Some(v) = map.get(k) {
                return v.as_str();
            }
            if let Some((_, inner)) = k.split_once(':') {
                if let Some(v) = map.get(inner) {
                    return v.as_str();
                }
            }
            for (name, v) in map.iter() {
                if name.ends_with(&format!(":{}", k)) {
                    return v.as_str();
                }
            }
            None
        };
        match key.as_deref().and_then(resolve) {
            Some(form) => {
                println!("symbolicate: {} → {}", target, form);
            }
            None => {
                println!("symbolicate: {} → <no mapping>", target);
                println!("  known names: {:?}", names.iter().map(|(_, n)| n).collect::<Vec<_>>());
            }
        }
        return Ok(());
    }

    let wasm_bytes = std::fs::read(wasm_path)?;
    println!("📦 {} ({} bytes)", wasm_path, wasm_bytes.len());

    let mut fuel_cfg = Config::new();
    fuel_cfg.consume_fuel(true);
    // 2026-08-29: default wasm stack (~8MB) exhausts around 900 nested interpreted calls in
    // meta-circular interpreters; NEAR host allows much deeper. 64MB keeps near-mock from
    // being the bottleneck while validating real programs.
    fuel_cfg.max_wasm_stack(64 * 1024 * 1024);
    fuel_cfg.async_stack_size(64 * 1024 * 1024);
    let engine = Engine::new(&fuel_cfg)?;
    let module = Module::from_binary(&engine, &wasm_bytes)?;

    if method == "exports" {
        for exp in module.exports() {
            println!("  {} {:?}", exp.name(), exp.ty());
        }
        return Ok(());
    }
    if method == "imports" {
        for imp in module.imports() {
            println!("  {}::{} {:?}", imp.module(), imp.name(), imp.ty());
        }
        return Ok(());
    }

    // Load persisted storage
    let loaded_storage: HashMap<Vec<u8>, Vec<u8>> = std::fs::read(state_file())
        .ok()
        .and_then(|d| bincode::deserialize(&d).ok())
        .unwrap_or_default();
    if !loaded_storage.is_empty() {
        println!("📂 Loaded {} storage keys", loaded_storage.len());
    } else {
        println!("🆕 Fresh state");
    }

    // Shared mutable state
    let state: Arc<Mutex<MockState>> = Arc::new(Mutex::new(MockState {
        storage: loaded_storage,
        touched: Default::default(),
        registers: HashMap::new(),
        return_data: None,
        view: run_view,
    }));

    let mut store = Store::new(&engine, ());
    store.set_fuel(prepaid_g)?;
    PREPAID_FUEL.with(|f| *f.borrow_mut() = prepaid_g);
    // 1024 pages = 64MB initial memory. Enough that wee_alloc never needs memory_grow.


    // G-14 (2026-09-02): set the promise-host env BEFORE linking. This
    // driver never initialized STATE_ARC, so build_env_linker's is_some
    // check fell through and bound all 12 promise hosts to silent noops —
    // every fire-and-forget payout executed invisibly (the G-14 "dead
    // arms" were never dead). Same env the cross driver sets.
    //
    // contract stays EMPTY unless NEAR_MOCK_CONTRACT is set: hosts treat
    // empty as the "escrow.test.near" fixture default (current_account_id,
    // sig messages, storage prefixes all derive from it) — passing the
    // wasm file path here broke all 54 auth vectors before the sig-check
    // ordering even ran.
    ENGINE_TLS.with(|e| *e.borrow_mut() = Some(Rc::new(engine.clone())));
    STATE_ARC.with(|s| *s.borrow_mut() = Some(state.clone()));
    // signer default = the legacy exec_ctx_or_default value so tests that
    // never set NEAR_MOCK_SIGNER see the same identity as before the ctx
    // became explicit (the lending battery stamps `own:` from it).
    let signer = std::env::var("NEAR_MOCK_SIGNER").unwrap_or_else(|_| "owner.test.near".into());
    EXEC_CTX.with(|c| {
        *c.borrow_mut() = Some(ExecCtx {
            input: args_bytes.clone(),
            signer: signer.clone(),
            predecessor: signer.clone(),
            contract: std::env::var("NEAR_MOCK_CONTRACT").unwrap_or_default(),
            view: run_view,
        })
    });
    let linker = build_env_linker(&mut store, &engine, state.clone(), args_bytes.clone())?;
    let instance = linker.instantiate(&mut store, &module)?;

    // Check ACTUAL memory (WASM-defined, not our unused one)
    let real_mem = instance.get_memory(&mut store, "memory").unwrap();
    eprintln!(
        "  WASM memory: {} pages ({}/65536 bytes)",
        real_mem.data(&store).len() / 65536,
        real_mem.data(&store).len()
    );

    println!("✅ Instantiated");

    // Pre-access the HashMap to warm it (avoid first-access during host function)
    {
        let st = state.lock().unwrap();
        let _count = st.storage.len();
        for (k, v) in st.storage.iter() {
            let _ = k.len() + v.len(); // touch the data
        }
        eprintln!("  Pre-touched {} storage entries", _count);
    }

    // Call the target method
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
            "Method '{}' not found. Available exports:\n  {}",
            method,
            avail.join("\n  ")
        )
    })?;
    let args_display = if args_bytes == b"{}" {
        String::new()
    } else {
        String::from_utf8_lossy(&args_bytes).into_owned()
    };
    println!("▶ {}({})", method, args_display);
    // Single execution ONLY. The old warm-up call double-applied storage
    // effects (a mint persisted twice → supply 2000 after one 1000-mint).
    // JIT warm-up is pointless here since fuel resets before the measured
    // run anyway. --once is kept as an accepted no-op for script compat.
    let run_once = true;
    let _ = args.iter().any(|a| a == "--once");
    let result = if run_once {
        Ok(())
    } else {
        func.call(&mut store, &[], &mut [])
    };

    // Check memory before call
    if let Some(real_mem) = instance.get_memory(&mut store, "memory") {
        eprintln!(
            "  WASM memory before: {} pages",
            real_mem.data(&store).len() / 65536
        );
    }

    // Reset fuel for the measured run (warm-up, if any, burned fuel too)
    store.set_fuel(prepaid_g)?;
    // Reset trie-touch cache too: the measured run starts with a cold trie,
    // just like a real transaction would.
    state.lock().unwrap().touched.clear();
    // Use a thread with timeout
    // G-14: snapshot for receipt-chain rollback (same single-tx atomicity
    // rule the cross driver enforces).
    let tx_snapshot: HashMap<Vec<u8>, Vec<u8>> = state.lock().unwrap().storage.clone();
    let result = func.call(&mut store, &[], &mut []);

    // Check WASM's actual memory
    if let Some(real_mem) = instance.get_memory(&mut store, "memory") {
        eprintln!(
            "  WASM memory after: {} pages",
            real_mem.data(&store).len() / 65536
        );
    }

    let mut run_outcome: &str = "ok";
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
                if data.len() == 8 {
                    let val = i64::from_le_bytes(data[..8].try_into().unwrap());
                    // 8 bytes is ambiguous: i64 returns AND 8-char strings
                    // both land here — show the string interpretation when
                    // all bytes are printable ASCII (a JSON/str return),
                    // else the i64 view (2026-08-31: 8-char strings were
                    // mislabeled as garbage i64s during M2 object debugging)
                    let printable = data.iter().all(|b| (0x20..0x7f).contains(b));
                    if printable {
                        let s = String::from_utf8_lossy(data);
                        println!("📄 {:?} (8-byte str | i64 view: {})", s, val);
                    } else {
                        // Untag: remove low 3 tag bits
                        println!("📄 {} (raw i64, untagged: {})", val, val >> 3);
                    }
                } else {
                    let s = String::from_utf8_lossy(data);
                    if !s.is_empty() {
                        println!("📄 {}", s);
                    }
                }
            }
            if !st.storage.is_empty() {
                println!("\n📦 Storage ({} keys):", st.storage.len());
                for (k, v) in st.storage.iter().take(10) {
                    let ks = String::from_utf8_lossy(k);
                    let vs = String::from_utf8_lossy(v);
                    // char-boundary-safe truncation (byte-slicing panics on multibyte chars)
                    let kshow: String = ks.chars().take(20).collect();
                    let vshow: String = vs.chars().take(60).collect();
                    println!(
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
            // returned DAG first, then fire-and-forget orphans (their
            // failures do NOT roll back the parent tx: receipt independence).
            drop(st); // release the print-section guard; execute_promise relocks
            let pending = PENDING_RETURN.with(|p| *p.borrow());
            if let Some(idx) = pending {
                eprintln!("  ⛓ resolving promise DAG (root {})", idx);
                match execute_promise(idx) {
                    Err(e) => {
                        println!("❌ receipt chain failed: {}", e);
                        println!("   ↺ full rollback (single tx = atomic)");
                        state.lock().unwrap().storage = tx_snapshot.clone();
                    }
                    Ok(results) => {
                        let last = results.iter().rev().find_map(|r| r.as_ref().cloned());
                        if let Some(bytes) = last {
                            let s = String::from_utf8_lossy(&bytes);
                            if !s.is_empty() {
                                println!("📄 {}", s);
                            }
                        }
                    }
                }
            }
            loop {
                let next = PROMISE_DAG.with(|d| {
                    d.borrow()
                        .iter()
                        .enumerate()
                        .find(|(i, _)| !EXECUTED_PROMISES.with(|e| e.borrow().contains(i)))
                        .map(|(i, _)| i)
                });
                let Some(idx) = next else { break };
                eprintln!("  ⛓ orphan receipt {} (fire-and-forget)", idx);
                match execute_promise(idx) {
                    Ok(_) => {}
                    Err(e) => println!(
                        "❌ orphan receipt failed: {} (parent tx stays committed)",
                        e
                    ),
                }
            }
        }
        Err(e) => {
            // G-14: entry trapped — full rollback (single tx = atomic), and
            // queued promise batches die with the tx (never executed).
            state.lock().unwrap().storage = tx_snapshot.clone();
            let msg = format!("{}", e);
            if msg.contains("all fuel consumed") {
                run_outcome = "out_of_gas";
                println!("❌ OutOfGas — exceeded {:.6} Tgas prepaid", prepaid_tgas);
            } else {
                run_outcome = "trap";
                println!("❌ {}", e);
                // Surface the root host error (e.g. ProhibitedInView,
                // InvalidRegisterId) — wasmtime's display leads with the
                // backtrace and hides it.
                for c in e.chain().skip(1) {
                    println!("   ↳ caused by: {}", c);
                }
            }
        }
    }

    // Gas report (1 fuel = 1 gas unit; host-call table is indicative-legacy)
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
