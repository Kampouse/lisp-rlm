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
pub(crate) use hosts::{build_env_linker, host_fn};
pub(crate) use promises::{
    dag_push, execute_promise, fail_receipts_any, fail_receipts_set, print_dag_map, sub_execute,
    PAction, PromiseBatch,
};
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
            "Usage: near-mock cross <state.bin> <acct=wasm,...> <contract-acct> <method> [args-json] [--fail-receipt N]\n       near-mock scenario <file.json>  (multi-step runner; steps support view/expect/fail_receipt)",
        );
        std::process::exit(1);
    }
    // Flags may appear anywhere after the `cross` keyword: --fail-receipt N (repeatable).
    let mut fail_receipts: Vec<usize> = Vec::new();
    let mut pos: Vec<String> = Vec::new();
    {
        let mut it = args[2..].iter();
        while let Some(t) = it.next() {
            if t == "--fail-receipt" {
                let n = it.next().and_then(|x| x.parse::<usize>().ok())
                    .ok_or("--fail-receipt requires a receipt index N (see the [map] printout)")?;
                fail_receipts.push(n);
            } else {
                pos.push(t.clone());
            }
        }
    }
    if pos.len() < 4 {
        eprintln!("Usage: near-mock cross <state.bin> <acct=wasm,...> <contract-acct> <method> [args-json] [--fail-receipt N]");
        std::process::exit(1);
    }
    let state_path = &pos[0];
    let manifest = &pos[1];
    let contract_acct = &pos[2];
    let method = &pos[3];
    let args_json = pos.get(4).cloned().unwrap_or_else(|| "{}".into());
    let run_view = pos.iter().any(|a| a == "--view");

    let mut fuel_cfg = Config::new();
    fuel_cfg.consume_fuel(true);
    fuel_cfg.max_wasm_stack(64 * 1024 * 1024);
    fuel_cfg.async_stack_size(64 * 1024 * 1024);
    let sync_engine = wasmtime::Engine::new(&fuel_cfg)?;
    let engine = Rc::new(sync_engine);

    let state = init_sandbox(engine.clone(), manifest, state_path, run_view)?;
    if !fail_receipts.is_empty() {
        fail_receipts_set(&fail_receipts);
    }

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
                if fail_receipts_any() {
                    print_dag_map();
                }
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
            trace: std::env::var("NEAR_MOCK_TRACE").map(|v| v == "1").unwrap_or(false),
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

// ============ --trace: host-call timeline + per-host gas attribution ============
// Global (not TLS) on purpose: promise sub-execution runs on worker threads;
// their host calls must land in the same timeline as the entry call's.

#[derive(Clone)]
pub(crate) struct TraceEntry {
    pub(crate) seq: u64,
    pub(crate) name: String,
    /// Exact gas charged by THIS host invocation (fuel delta across its body).
    pub(crate) gas: u64,
    /// Host returned Err (wasm-level trap follows).
    pub(crate) err: bool,
}

static HOST_TRACE: std::sync::Mutex<Option<Vec<TraceEntry>>> = std::sync::Mutex::new(None);
static HOST_ORDER: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);
static HOST_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Record one host invocation. Called by the host_fn wrapper around every
/// host closure body when cfg.trace is on. Live line → stderr; buffered
/// entry for the summary/--json.
pub(crate) fn trace_host(name: &str, gas: u64, err: bool) {
    let seq = HOST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    eprintln!(
        "  🔍 [{seq:>4}] {name}  gas={:.3}G{}",
        gas as f64 / 1e9,
        if err { "  ❌err" } else { "" }
    );
    if let Ok(mut g) = HOST_TRACE.lock() {
        g.get_or_insert_with(Vec::new).push(TraceEntry {
            seq,
            name: name.to_string(),
            gas,
            err,
        });
    }
    if let Ok(mut o) = HOST_ORDER.lock() {
        o.get_or_insert_with(Vec::new).push(name.to_string());
    }
}

/// Clear the timeline (scenario runner: once per step so summaries are
/// per-step). Cheap no-op when tracing is off.
pub(crate) fn host_trace_reset() {
    if !mock_cfg().trace {
        return;
    }
    if let Ok(mut g) = HOST_TRACE.lock() {
        g.take();
    }
    if let Ok(mut o) = HOST_ORDER.lock() {
        o.take();
    }
}

/// Per-host aggregation: (total_gas, [(host, calls, gas)]) sorted by gas desc.
/// Returns zeros when tracing is off; drains the buffer.
pub(crate) fn host_trace_summary() -> (u64, Vec<(String, u64, u64)>) {
    let entries = match HOST_TRACE.lock() {
        Ok(mut g) => g.take().unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    if let Ok(mut o) = HOST_ORDER.lock() {
        o.take();
    }
    if entries.is_empty() {
        return (0, Vec::new());
    }
    let total: u64 = entries.iter().map(|e| e.gas).sum();
    let mut agg: HashMap<String, (u64, u64)> = HashMap::new();
    for e in &entries {
        let slot = agg.entry(e.name.clone()).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += e.gas;
    }
    let mut rows: Vec<(String, u64, u64)> =
        agg.into_iter().map(|(k, (c, g))| (k, c, g)).collect();
    rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    (total, rows)
}

/// Human-readable summary block (called by drivers after a call/step).
pub(crate) fn print_host_trace_summary() {
    if !mock_cfg().trace {
        return;
    }
    let (total, rows) = host_trace_summary();
    if rows.is_empty() {
        println!("🔍 trace: no host calls");
        return;
    }
    println!(
        "🔍 host trace — {} calls, {:.3}G gas total:",
        rows.iter().map(|r| r.1).sum::<u64>(),
        total as f64 / 1e9
    );
    for r in rows.iter().take(12) {
        println!("  {:>12.3}G  {:>4}×  {}", r.2 as f64 / 1e9, r.1, r.0);
    }
    if rows.len() > 12 {
        println!("  … {} more hosts", rows.len() - 12);
    }
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
    let pbc = host_fn("promise_batch_create",
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
    let pbt = host_fn("promise_batch_then",
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
    let pafc = host_fn("promise_batch_action_function_call",
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
    let pbat = host_fn("promise_batch_action_transfer",
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
    let pyc = host_fn("promise_yield_create",
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
    let pyr = host_fn("promise_yield_resume",
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
    let pc = host_fn("promise_create",
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
    let pt = host_fn("promise_then",
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
    let pa = host_fn("promise_and",
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
    let prc = host_fn("promise_results_count",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, results| {
            results[0] = Val::I64(PROMISE_RESULTS.with(|r| r.borrow().len() as i64));
            Ok(())
        },
    );
    // 34 promise_result(idx, reg) -> status
    let state_for_pr = STATE_ARC.with(|s| s.borrow().clone());
    let pr = host_fn("promise_result",
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
    let pret = host_fn("promise_return",
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


/// Shared bootstrap for `cross` and `scenario`: load a multi-contract
/// manifest (acct=path.wasm,...), load persistent state, install TLS.
pub(crate) fn init_sandbox(
    engine: Rc<wasmtime::Engine>,
    manifest: &str,
    state_path: &str,
    view: bool,
) -> Result<Arc<Mutex<MockState>>, Box<dyn std::error::Error>> {
    let mut modules: HashMap<String, wasmtime::Module> = HashMap::new();
    for pair in manifest.split(',') {
        let (acct, path) = pair
            .split_once('=')
            .ok_or("manifest entries must be acct=/path")?;
        let bytes = std::fs::read(path)?;
        eprintln!("📦 {} → {}", acct, path);
        modules.insert(
            acct.to_string(),
            wasmtime::Module::from_binary(&engine, &bytes)?,
        );
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
        view,
    }));

    MODULES.with(|m| *m.borrow_mut() = Some(Arc::new(modules)));
    STATE_ARC.with(|s| *s.borrow_mut() = Some(state.clone()));
    ENGINE_TLS.with(|e| *e.borrow_mut() = Some(engine.clone()));
    Ok(state)
}

/// Credit an attached deposit to the callee's NEAR balance (real receipt
/// semantics: value arrives before the entry runs). Shared by `cross` and
/// `scenario`.
pub(crate) fn credit_attach(
    state: &std::sync::Arc<std::sync::Mutex<MockState>>,
    contract_acct: &str,
    amt: u128,
) -> Result<(), String> {
    let mut st = state.lock().unwrap();
    let key = prefixed_key(contract_acct, b"\x00near-bal");
    let bal: u128 = st
        .storage
        .get(&key)
        .and_then(|v| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u128);
    st.storage.insert(key, (bal + amt).to_string().into_bytes());
    eprintln!(
        "  💰 attached {} yocto → {} (bal {})",
        amt,
        contract_acct,
        bal + amt
    );
    Ok(())
}

/// `scenario <file.json>` — multi-step multi-contract runner in ONE process.
/// One sandbox init, one DAG; failures recorded, run continues. Step fields:
///   method (req), args, contract (account), view, expect (substring of output),
///   fail_receipt (N — force receipt N to fail during this step).
///   gas (TGas cap on the entry call — die by out-of-gas instead of trap),
///   attach (decimal yocto deposited to the callee before the call),
///   as (signer account; predecessor defaults to it — a direct tx from
///   that account), predecessor (override — models a call arriving from
///   another contract), now (unix-secs time base), advance (secs ADDED to
///   the time base). Time is monotonic: clock changes persist for later
///   steps, like real blocks.
///   snapshot / restore (named full-storage forks),
///   expect_same_storage_as (storage must equal a snapshot — branch compare),
///   expect: "trap" requires the entry call to trap.
/// Compatible with examples/ft/tests/scenarios/*.json (name/steps/method/args/view/expect).
pub(crate) fn run_scenario(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?)
            .map_err(|e| format!("bad scenario JSON {path}: {e}"))?;
    let steps = spec
        .get("steps")
        .and_then(|s| s.as_array())
        .ok_or("scenario needs {\"name\":..., \"steps\":[...]}")?;

    // Sandbox defaults: state.bin next to the scenario file; single-contract
    // manifest = the FT convention (contract.wasm deployed at contract.acct,
    // defaulting to owner.test.near).
    let dir = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let state_path = match spec.get("state").and_then(|s| s.as_str()) {
        Some(p) => p.to_string(),
        None => dir.join("state.bin").to_string_lossy().into_owned(),
    };
    let wasm_path = dir.join("contract.wasm").to_string_lossy().into_owned();
    let manifest = spec
        .get("manifest")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("owner.test.near={}", wasm_path));
    let default_acct = manifest
        .split(',')
        .next()
        .and_then(|p| p.split_once('='))
        .map(|(a, _)| a.to_string())
        .unwrap_or_else(|| "owner.test.near".into());

    let mut fuel_cfg = Config::new();
    fuel_cfg.consume_fuel(true);
    // epoch_interruption MUST be on or set_epoch_deadline is inert —
    // no epoch checks get compiled into wasm, so spin loops run forever.
    fuel_cfg.epoch_interruption(true);
    fuel_cfg.max_wasm_stack(64 * 1024 * 1024);
    fuel_cfg.async_stack_size(64 * 1024 * 1024);
    let engine = Rc::new(wasmtime::Engine::new(&fuel_cfg)?);
    // Epoch ticker: 1 tick ≈ 1 ms ⇒ a `gas: T` step's deadline ≈ T ms of
    // wall clock (1 TGas ≈ 1 ms of NEAR compute). Wasmtime fuel alone can't
    // bound pure-compute loops at NEAR-honest rates (1 fuel/instr ⇒ 1 TGas
    // ≈ 16 min). Detached thread; process exits when the scenario ends.
    if steps.iter().any(|s| s.get("gas").is_some()) {
        // Clone the INNER engine — Rc<Engine> can't cross threads, but
        // wasmtime::Engine is internally Arc'd, so this shares the epoch.
        let tick: wasmtime::Engine = (*engine).clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(1));
            tick.increment_epoch();
        });
    }
    let state = init_sandbox(engine.clone(), &manifest, &state_path, false)?;

    let name = spec.get("name").and_then(|n| n.as_str()).unwrap_or(path);
    println!("🎬 scenario: {} ({} steps)", name, steps.len());

    let mut pass = 0u32;
    let mut fail = 0u32;
    let mut expect_out: Vec<String> = Vec::new();
    // named full-storage forks (snapshot/restore/expect_same_storage_as)
    let mut snapshots: HashMap<String, HashMap<Vec<u8>, Vec<u8>>> = HashMap::new();

    for (i, step) in steps.iter().enumerate() {
        let method: Option<String> =
            step.get("method").and_then(|m| m.as_str()).map(String::from);
        // Bookkeeping-only step (no `method`): fork operations on storage.
        let Some(method) = method else {
            let mut did = false;
            if let Some(name) = step.get("snapshot").and_then(|s| s.as_str()) {
                let snap = state.lock().unwrap().storage.clone();
                snapshots.insert(name.to_string(), snap);
                println!("  📸 snapshot '{}'", name);
                did = true;
            }
            if let Some(name) = step.get("restore").and_then(|s| s.as_str()) {
                let snap = snapshots
                    .get(name)
                    .ok_or(format!("step {}: unknown snapshot '{}'", i, name))?
                    .clone();
                state.lock().unwrap().storage = snap;
                println!("  ♻️ restored '{}'", name);
                did = true;
            }
            if let Some(other) = step.get("expect_same_storage_as").and_then(|s| s.as_str()) {
                let cur = state.lock().unwrap().storage.clone();
                let want = snapshots
                    .get(other)
                    .ok_or(format!("step {}: unknown snapshot '{}'", i, other))?;
                if &cur == want {
                    println!("  ✓ storage identical to snapshot '{}'", other);
                } else {
                    println!("  ✗ storage DIVERGED from snapshot '{}'", other);
                    fail += 1;
                }
                did = true;
            }
            if !did {
                return Err(format!("step {}: no method, no bookkeeping keys", i).into());
            }
            continue;
        };
        let args_json = step
            .get("args")
            .map(|a| a.to_string())
            .unwrap_or_else(|| "{}".into());
        let contract = step
            .get("contract")
            .and_then(|c| c.as_str())
            .unwrap_or(&default_acct)
            .to_string();
        let is_view = step.get("view").and_then(|v| v.as_bool()).unwrap_or(false);
        let fail_receipt = step
            .get("fail_receipt")
            .and_then(|f| f.as_u64())
            .map(|n| n as usize);
        // per-step identity: `as` sets signer; predecessor defaults to it
        // (direct tx from that account) — an explicit `predecessor` models
        // a call arriving from another contract. Overrides the runner
        // default (owner.test.near) for THIS step only.
        let step_signer = step
            .get("as")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "owner.test.near".into());
        let step_pred = step
            .get("predecessor")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| step_signer.clone());
        // per-step time travel: `now` jumps the chain clock to an absolute
        // unix-secs base, `advance` shifts it forward. CHAIN SEMANTICS:
        // time is monotonic — the mutation PERSISTS for later steps (like
        // real blocks; a timelock test sets advance once, later steps run
        // at the later time). String OR number accepted (see gas above).
        let step_now = step.get("now").and_then(|v| {
            v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        let step_advance = step.get("advance").and_then(|v| {
            v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        // gas budget: TGas cap on this step's entry call (out-of-gas ≠ trap)
        // accept number OR string ("2") — a stringy gas silently un-capping
        // the step would turn a chaos test into a 20s-per-spin hang
        let gas_cap_tgas = step
            .get("gas")
            .and_then(|g| g.as_u64().or_else(|| g.as_str().and_then(|s| s.parse().ok())));
        // attached deposit for this step (decimal yocto string, or number)
        let attach_amt: Option<u128> = match step.get("attach") {
            None => None,
            Some(serde_json::Value::String(s)) => Some(
                s.trim()
                    .parse()
                    .map_err(|_| format!("step {}: attach must be decimal yocto", i))?,
            ),
            Some(serde_json::Value::Number(n)) => Some(
                n.as_u64()
                    .ok_or_else(|| format!("step {}: attach too large", i))?
                    as u128,
            ),
            Some(_) => {
                return Err(format!("step {}: attach must be a string or number", i).into())
            }
        };
        let snap_name = step.get("snapshot").and_then(|s| s.as_str()).map(String::from);
        let restore_name = step
            .get("restore")
            .and_then(|s| s.as_str())
            .map(String::from);
        let same_as = step
            .get("expect_same_storage_as")
            .and_then(|s| s.as_str())
            .map(String::from);
        // expect:"trap" — chaos step that MUST revert
        let expect_trap = step.get("expect").and_then(|e| e.as_str()) == Some("trap");

        // per-step hygiene: a scenario runs many entries in ONE process,
        // so leftover TLS from step N-1 must not bleed into step N (the
        // cross runner never noticed — it's one call per process)
        expect_out.clear();
        state.lock().unwrap().return_data = None;
        PROMISE_RESULTS.with(|r| *r.borrow_mut() = Vec::new());
        PENDING_RETURN.with(|p| *p.borrow_mut() = None);

        // set view + forced receipts for this step
        let old_view = state.lock().unwrap().view;
        state.lock().unwrap().view = is_view;
        fail_receipts_set(&fail_receipt.iter().copied().collect::<Vec<usize>>());

        // forks + attached deposit, ordered like a real tx: restore lands
        // BEFORE the deposit credit, snapshot sees the pre-call state
        if let Some(name) = &restore_name {
            let snap = snapshots
                .get(name)
                .cloned()
                .ok_or_else(|| format!("step {}: unknown snapshot '{}'", i, name))?;
            state.lock().unwrap().storage = snap;
            println!("  ↩ restored storage fork '{}'", name);
        }
        if let Some(name) = &snap_name {
            let snap = state.lock().unwrap().storage.clone();
            snapshots.insert(name.clone(), snap);
            println!("  📸 snapshot '{}'", name);
        }
        // pre-call copy for NEAR transaction atomicity: a trapped entry
        // rolls back ALL of this step's writes (and its attached-deposit
        // credit — the deposit is refunded on failure), and its promises
        // are never scheduled.
        let pre_call = state.lock().unwrap().storage.clone();
        if let Some(amt) = attach_amt {
            credit_attach(&state, &contract, amt)
                .map_err(|e| format!("step {}: {}", i, e))?;
            CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = Some(amt));
        } else {
            CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = None);
        }

        println!("\n── step {} ▶ {}.{}{}{} ──", i, contract, method, if args_json == "{}" { "".into() } else { format!(" {}", args_json) }, if is_view { " (view)" } else { "" });
        if step_signer != "owner.test.near" || step_pred != step_signer {
            println!(
                "  👤 as {} (predecessor {})",
                step_signer, step_pred
            );
        }
        let module = MODULES
            .with(|m| m.borrow().as_ref().unwrap().get(&contract).cloned())
            .ok_or(format!("step {}: contract {} not in manifest", i, contract))?;
        EXEC_CTX.with(|c| {
            *c.borrow_mut() = Some(ExecCtx {
                input: args_json.clone().into_bytes(),
                signer: step_signer.clone(),
                predecessor: step_pred.clone(),
                contract: contract.clone(),
                view: is_view,
            })
        });
        // Apply the step's clock BEFORE the call (promise DAG resolution in
        // the Ok arm sees it too — receipts inherit the step's clock, like
        // real receipts). Scenario mode never populates RUN_CFG (flag
        // parsing lives in the single-call path), so get_or_insert a
        // default here — mock_now_nanos would otherwise read env-only
        // values and per-step now/advance would silently no-op.
        RUN_CFG.with(|c| {
            let mut slot = c.borrow_mut();
            let cfg = slot.get_or_insert_with(RunCfg::default);
            if let Some(n) = step_now {
                cfg.base_ts = Some(n);
            }
            if let Some(a) = step_advance {
                cfg.advance_secs += a; // accumulates — monotonic chain time
            }
        });
        // NEAR-scale fuel: 1 TGas = 1e12 fuel units, matching host-call
        // costs and the ⛽ reporting (÷1e12). Runaway compute is bounded by
        // an epoch deadline at ~1 ms per TGas (spawned below) — pure-wasm
        // fuel burns at only ~1/instr, so without it `gas: 1` would spin
        // for minutes instead of dying like NEAR's prepaid-gas exhaustion.
        let prepaid: u64 = gas_cap_tgas
            .map(|t| t.saturating_mul(1_000_000_000_000))
            .unwrap_or_else(|| PREPAID_FUEL.with(|f| *f.borrow()));
        let mut store = wasmtime::Store::new(&*engine, ());
        store.set_fuel(prepaid)?;
        // Epoch deadline: ~t ms for gas-capped steps, 20 s wall bound
        // otherwise (ticker ticks every 1 ms; epochs only advance when a
        // gas-capped scenario spawned the ticker — otherwise inert).
        store.set_epoch_deadline(gas_cap_tgas.unwrap_or(20_000).max(1));
        let linker = build_env_linker(&mut store, &*engine, state.clone(), args_json.into_bytes())?;
        let instance = linker.instantiate(&mut store, &module)?;
        let result = instance
            .get_func(&mut store, &method)
            .ok_or(format!("step {}: method '{}' not found on {}", i, method, contract))?
            .call(&mut store, &[], &mut []);
        let trapped = result.is_err();
        let mut step_failed = false;

        match &result {
            Ok(()) => {
                println!("✅ ok");
                // resolve the entry's promise chain (receipt DAG), if any
                let pending = PENDING_RETURN.with(|p| *p.borrow());
                if let Some(idx) = pending {
                    if fail_receipts_any() {
                        print_dag_map();
                    }
                    eprintln!("  ⛓ resolving promise DAG (root {})", idx);
                    match execute_promise(idx) {
                        Err(e) => {
                            println!("❌ receipt chain failed: {}", e);
                            step_failed = true;
                        }
                        Ok(results) => {
                            for (ri, r) in results.iter().enumerate() {
                                if let Some(bytes) = r {
                                    let s = String::from_utf8_lossy(bytes).into_owned();
                                    println!("  ⤷ result[{ri}]: {s}");
                                    expect_out.push(s);
                                }
                            }
                        }
                    }
                } else {
                    // no promise: surface the entry's own return data
                    let st = state.lock().unwrap();
                    if let Some(ref data) = st.return_data {
                        let s = String::from_utf8_lossy(data);
                        if !s.is_empty() {
                            println!("📄 {}", s);
                            expect_out.push(s.into_owned());
                        }
                    }
                }
            }
            Err(e) => {
                println!("❌ trap: {}", e);
                step_failed = true;
            }
        }

        // NEAR transaction atomicity: trapped entry ⇒ full revert (writes
        // discarded, deposit credit refunded, promises never scheduled).
        if trapped {
            state.lock().unwrap().storage = pre_call;
            PROMISE_DAG.with(|d| d.borrow_mut().clear());
            EXECUTED_PROMISES.with(|e| e.borrow_mut().clear());
        }

        // chaos semantics: expect:"trap" means the entry MUST have reverted
        // (consumes the expect field — not a string-expect)
        if expect_trap {
            if trapped {
                println!("✓ trap as expected (state rolled back)");
                step_failed = false; // the Err arm flagged it; un-flag
            } else {
                println!("✗ expected trap, call succeeded");
                step_failed = true;
            }
        }

        // drain orphaned receipts (fire-and-forget promises) — only for
        // SUCCESSFUL entries; a trapped tx never schedules its promises
        if !trapped {
            loop {
                let next = PROMISE_DAG.with(|d| {
                    d.borrow()
                        .iter()
                        .enumerate()
                        .find(|(i, _)| !EXECUTED_PROMISES.with(|e| e.borrow().contains(i)))
                        .map(|(i, _)| i)
                });
                let Some(idx) = next else { break };
                if let Err(e) = execute_promise(idx) {
                    println!("❌ orphan receipt {} failed: {}", idx, e);
                    step_failed = true;
                }
            }
        }
        PROMISE_DAG.with(|d| d.borrow_mut().clear());
        EXECUTED_PROMISES.with(|e| e.borrow_mut().clear());
        fail_receipts_set(&[]); // clear between steps
        PENDING_RETURN.with(|p| *p.borrow_mut() = None);
        state.lock().unwrap().view = old_view;
        // (clock intentionally NOT restored — monotonic chain time, see above)

        // checks: expect ("ok" = no trap / no receipt-chain failure, else
        // substring against 📄 outputs or storage) and contains (substring
        // against 📄 outputs or the step contract's storage partition —
        // callbacks express results via storage_write, and receipt result
        // buffers don't always carry them)
        let stored: Vec<String> = {
            let st = state.lock().unwrap();
            let pre = prefixed_key(&contract, b"");
            st.storage
                .iter()
                .filter(|(k, _)| k.starts_with(&pre))
                .map(|(k, v)| {
                    format!(
                        "{}={}",
                        String::from_utf8_lossy(&k[pre.len()..]),
                        String::from_utf8_lossy(v)
                    )
                })
                .collect()
        };
        if let Some(want) = step.get("expect").and_then(|e| e.as_str()) {
            if want == "trap" {
                // consumed by the chaos check above (verdict already printed)
            } else {
            let hit = if want == "ok" {
                !step_failed
            } else {
                expect_out.iter().any(|s| s.contains(want))
                    || stored.iter().any(|kv| kv.contains(want))
            };
            if hit {
                println!("✓ expect '{}' ✓", want);
            } else {
                println!("✗ expect '{}' — got {:?} storage {:?}", want, expect_out, stored);
                step_failed = true;
            }
            }
        }
        if let Some(want) = step.get("contains").and_then(|c| c.as_str()) {
            let in_storage = stored.iter().any(|kv| kv.contains(want));
            if expect_out.iter().any(|s| s.contains(want)) || in_storage {
                println!("✓ contains '{}' ✓", want);
            } else {
                println!("✗ contains '{}' — storage: {:?}", want, stored);
                step_failed = true;
            }
        }
        if let Some(want) = &same_as {
            let Some(snap) = snapshots.get(want) else {
                return Err(format!(
                    "step {}: expect_same_storage_as: unknown snapshot '{}'",
                    i, want
                )
                .into());
            };
            let equal = { state.lock().unwrap().storage == *snap };
            if equal {
                println!("✓ storage == snapshot '{}' ✓", want);
            } else {
                println!("✗ storage diverged from snapshot '{}'", want);
                step_failed = true;
            }
        }

        // --trace / NEAR_MOCK_TRACE: per-step host summary (timeline already
        // went to stderr during the step). Buffer resets so the next step's
        // totals stay isolated.
        if mock_cfg().trace {
            print_host_trace_summary();
            host_trace_reset();
        }

        if step_failed {
            fail += 1;
            println!("step {} ⇒ FAIL", i);
        } else {
            pass += 1;
        }
        expect_out.clear();
    }

    // persist final state (single write at end of scenario)
    let st = state.lock().unwrap();
    let encoded = bincode::serialize(&st.storage)?;
    std::fs::write(&state_path, encoded)?;
    println!("\n🎬 scenario {}: {} pass / {} fail — state → {}", name, pass, fail, state_path);
    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// `state` subcommand — offline manipulation of near-mock state files.
///
///   near-mock state import <state.bin> <dump.json>
///   near-mock state dump <state.bin> [account-prefix]
///
/// Import accepts the RPC-shaped JSON produced by
/// `scripts/fetch_near_state.sh <account> <out.json>`:
///   {"account": "...", "block_height": N, "values": [
///     {"key": "<base64>", "value": "<base64>"}, ...]}
/// Values merge into <state.bin> under the account's storage partition
/// (acct + 0x01 + key — same namespacing the host uses). Other accounts'
/// keys are untouched; `--replace-acct` drops this account's existing
/// partition first. Use `-` as <dump.json> to read stdin.
fn run_state_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: near-mock state import <state.bin> <dump.json> [--replace-acct]\n       near-mock state dump <state.bin> [account-prefix]";
    let sub = args.get(2).map(|s| s.as_str()).ok_or(usage)?;
    let replace_acct = args.iter().any(|a| a == "--replace-acct");

    // base64 (RPC wire format) -> raw trie bytes
    fn b64(s: &str) -> Result<Vec<u8>, String> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map_err(|e| format!("bad base64: {e}"))
    }

    match sub {
        "import" => {
            let state_path = args.get(3).ok_or(usage)?;
            let dump_path = args.get(4).ok_or(usage)?;

            let raw: Vec<u8> = if dump_path == "-" {
                use std::io::Read;
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf)?;
                buf
            } else {
                std::fs::read(dump_path)?
            };
            let dump: serde_json::Value = serde_json::from_slice(&raw)?;
            let account = dump
                .get("account")
                .and_then(|a| a.as_str())
                .ok_or("dump missing \"account\"")?
                .to_string();
            let arr = dump
                .get("values")
                .and_then(|v| v.as_array())
                .ok_or("dump missing \"values\" array")?;

            // parse every entry BEFORE touching the state file (atomic-ish:
            // a malformed dump never half-applies)
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                let k = v
                    .get("key")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| format!("values[{i}] missing key"))?;
                let val = v
                    .get("value")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| format!("values[{i}] missing value"))?;
                entries.push((b64(k)?, b64(val)?));
            }

            // load or start fresh state
            let mut map: std::collections::HashMap<Vec<u8>, Vec<u8>> = match std::fs::read(state_path) {
                Ok(d) => bincode::deserialize(&d)
                    .map_err(|e| format!("{}: not a near-mock state file ({e})", state_path))?,
                Err(_) => Default::default(),
            };

            let pre = prefixed_key(&account, b"");
            let before = map.len();
            if replace_acct {
                let stale: Vec<Vec<u8>> = map
                    .keys()
                    .filter(|k| k.len() > pre.len() && k.starts_with(&pre))
                    .cloned()
                    .collect();
                for k in stale {
                    map.remove(&k);
                }
            }
            for (k, v) in entries {
                map.insert(prefixed_key(&account, &k), v);
            }
            let _ = before; // informational only

            std::fs::write(state_path, bincode::serialize(&map)?)?;
            println!(
                "📥 imported {} keys for {} → {}{}",
                arr.len(),
                account,
                state_path,
                if replace_acct { " (partition replaced)" } else { "" }
            );
            Ok(())
        }
        "dump" => {
            let state_path = args.get(3).ok_or(usage)?;
            let prefix: Option<String> = args.get(4).cloned();
            let data = std::fs::read(state_path)
                .map_err(|e| format!("{}: {} (create one via a scenario or import)", state_path, e))?;
            let map: std::collections::HashMap<Vec<u8>, Vec<u8>> = bincode::deserialize(&data)
                .map_err(|e| format!("{}: not a near-mock state file ({e})", state_path))?;

            use base64::Engine;
            let mut rows: Vec<(String, String, String)> = map
                .iter()
                .filter_map(|(k, v)| {
                    // namespaced layout: acct + 0x01 + key
                    let sep = k.iter().position(|&b| b == 0x01)?;
                    let acct = String::from_utf8(k[..sep].to_vec()).ok()?;
                    if let Some(p) = &prefix {
                        if &acct != p && !acct.starts_with(p.as_str()) {
                            return None;
                        }
                    }
                    let key = String::from_utf8(k[sep + 1..].to_vec())
                        .unwrap_or_else(|_| "<binary>".to_string());
                    Some((acct, key, base64::engine::general_purpose::STANDARD.encode(v)))
                })
                .collect();
            rows.sort();

            println!("[");
            for (i, (acct, key, v)) in rows.iter().enumerate() {
                let comma = if i + 1 < rows.len() { "," } else { "" };
                println!(
                    "  {{\"account\": \"{}\", \"key\": \"{}\", \"value_b64\": \"{}\"}}{}",
                    acct, key, v, comma
                );
            }
            println!("]");
            println!("— {} keys{}", rows.len(), prefix.as_deref().map(|p| format!(" for '{p}'")).unwrap_or_default());
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("cross") {
        return run_cross(&args);
    }
    if args.get(1).map(|s| s.as_str()) == Some("scenario") {
        let path = args.get(2).ok_or("usage: near-mock scenario <file.json>")?;
        return run_scenario(path);
    }
    // `state import` / `state dump` — offline state file manipulation
    if args.get(1).map(|s| s.as_str()) == Some("state") {
        return run_state_cmd(&args);
    }
    fn print_main_usage() {
        println!("near-mock — local NEAR contract runner (wasmtime, no node)");
        println!();
        println!("USAGE:");
        println!("  near-mock <wasm> <method> [args-json] [flags]");
        println!("  near-mock <wasm> exports|imports|reset");
        println!("  near-mock <wasm> symbolicate <idx-or-name> [map-file]");
        println!("  near-mock cross <state.bin> <acct=/path.wasm,...> <contract-acct> <method> [args-json]");
        println!("  near-mock scenario <file.json>  (steps: method/contract/args/view/as/");
        println!("                                  predecessor/now/advance/gas/attach/expect/...)");
        println!("  near-mock state import <state.bin> <dump.json>");
        println!("  near-mock state dump <state.bin>");
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
        println!("  --trace              host-call timeline (stderr) + per-host gas totals");
        println!("  --debug              verbose host traces ([schnorr-dbg], ptr/len)");
        println!("  --once               accepted no-op (kept for script compat)");
        println!();
        println!("ENV:");
        println!("  NEAR_MOCK_STATE       state file path (default /tmp/near-mock-state.bin)");
        println!("  NEAR_MOCK_ATTACH      attached deposit (decimal yocto)");
        println!("  NEAR_MOCK_SIGNER      signer account (default owner.test.near)");
        println!("  NEAR_MOCK_TRACE       =1 enables --trace (works in scenario steps)");
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
    if args.iter().any(|a| a == "--trace") {
        cfg.trace = true;
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
            // --json --trace: per-host totals (top hosts first) for CI diffs
            "host_trace": if mock_cfg().trace {
                Some(
                    host_trace_summary()
                        .1
                        .into_iter()
                        .map(|(n, c, g)| {
                            serde_json::json!({"host": n, "calls": c, "gas_tgas": g as f64 / 1e12})
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            },
        });
        println!("JSON {}", serde_json::to_string(&j).unwrap_or_default());
    }

    // --trace human summary (JSON mode puts the same data in "host_trace").
    // The live per-call timeline already went to stderr during execution.
    if mock_cfg().trace && !json_out {
        print_host_trace_summary();
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
