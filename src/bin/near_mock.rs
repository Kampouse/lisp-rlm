//! NEAR contract mock runner with state persistence.
//! Warms up wee_alloc by calling a cheap init method first.
//!
//! Usage:
//!   cargo run --bin near-mock -- <wasm> <method> [args-json] [--once] [--view] [--prepaid <TGAS>]
//!   cargo run --bin near-mock -- <wasm> exports|imports|reset
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

// State file: /tmp/near-mock-state.bin by default, overridable via
// NEAR_MOCK_STATE (single source of truth: lisp_rlm_wasm::near_mock_state_file)
// so parallel sessions / concurrent test runners never stomp each other.
fn state_file() -> String {
    // single source of truth lives in the library (tests use it too)
    lisp_rlm_wasm::near_mock_state_file()
}



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

#[derive(Clone)]
enum PAction {
    FnCall { method: String, args: Vec<u8>, gas: u64, dep: u128 },
    Transfer(u128),
}

#[derive(Clone)]
struct PromiseBatch {
    deps: Vec<usize>,
    account: String,
    creator: String,
    actions: Vec<PAction>,
    /// Yield promise (host 82): the callback executes once with a NotReady
    /// result, then re-executes with the payload when host 83 resumes it.
    is_yield: bool,
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

fn prefixed_key(acct: &str, key: &[u8]) -> Vec<u8> {
    let mut k = acct.as_bytes().to_vec();
    k.push(0x01);
    k.extend_from_slice(key);
    k
}

fn dag_push(deps: Vec<usize>, account: String, actions: Vec<PAction>) -> usize {
    let creator = exec_ctx_or_default().contract;
    PROMISE_DAG.with(|d| {
        let mut d = d.borrow_mut();
        d.push(PromiseBatch { deps, account, creator, actions, is_yield: false });
        d.len() - 1
    })
}

// batches already resolved this run (returned-DAG traversal + orphan drain)
thread_local! {
    static EXECUTED_PROMISES: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Snapshot + revert one account's storage partition (failed receipts).
fn snapshot_partition(st: &MockState, acct: &str) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let pre = prefixed_key(acct, b"");
    st.storage
        .iter()
        .filter(|(k, _)| k.len() > pre.len() && k.starts_with(&pre))
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect()
}

fn restore_partition(st: &mut MockState, snap: Vec<(Vec<u8>, Option<Vec<u8>>)>, acct: &str) {
    let pre = prefixed_key(acct, b"");
    let keys: Vec<Vec<u8>> = st
        .storage
        .keys()
        .filter(|k| k.len() > pre.len() && k.starts_with(&pre))
        .cloned()
        .collect();
    for k in keys {
        st.storage.remove(&k);
    }
    for (k, v) in snap {
        if let Some(v) = v {
            st.storage.insert(k, v);
        }
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

fn sub_execute(
    account: &str,
    method: &str,
    args: &[u8],
    predecessor: &str,
    deposit: u128,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let module = MODULES.with(|m| {
        m.borrow()
            .as_ref()
            .and_then(|map| map.get(account).cloned())
    });
    let Some(module) = module else {
        // 2026-09-02 live-caught (nostr-gov tk="nil"): unknown-account FnCall
        // receipts FAIL on-chain (AccountDoesNotExist). The old silent
        // Ok(None) let gauntlets pass while every payout routed to a
        // phantom contract. Hard-error so the step shows the failure.
        return Err(format!(
            "MOCK-CHAIN-FAILURE: promise FnCall to unknown account '{}' (on-chain: AccountDoesNotExist)",
            account
        )
        .into());
    };
    let state = STATE_ARC.with(|s| s.borrow().clone()).expect("STATE_ARC set");
    let engine = ENGINE_TLS.with(|e| e.borrow().clone()).expect("ENGINE_TLS set");

    // Snapshot isolation context
    let old_ctx = EXEC_CTX.with(|c| c.borrow().clone());
    let (old_regs, old_ret) = {
        let st = state.lock().unwrap();
        (st.registers.clone(), st.return_data.clone())
    };
    {
        let mut st = state.lock().unwrap();
        st.registers.clear();
        st.return_data = None;
    }
    EXEC_CTX.with(|c| {
        *c.borrow_mut() = Some(ExecCtx {
            input: args.to_vec(),
            signer: predecessor.to_string(),
            predecessor: predecessor.to_string(),
            contract: account.to_string(),
            view: false,
        })
    });

    // Receipt value: debit the SENDER (predecessor), credit the callee.
    // On trap, partition restore below undoes the callee's credit; the
    // sender's debit must also unwind → do the debit AFTER the child's
    // snapshot decision — simplest: perform both, and on trap manually
    // refund the sender (real NEAR: failed receipt refunds its deposit).
    if deposit > 0 {
        let mut st = state.lock().unwrap();
        let credit_key = prefixed_key(account, b"\x00near-bal");
        let bal: u128 = st.storage.get(&credit_key)
            .and_then(|v| std::str::from_utf8(v).ok()).and_then(|s| s.parse().ok()).unwrap_or(0);
        st.storage.insert(credit_key.clone(), (bal + deposit).to_string().into_bytes());
        let debit_key = prefixed_key(predecessor, b"\x00near-bal");
        let sbal: u128 = st.storage.get(&debit_key)
            .and_then(|v| std::str::from_utf8(v).ok()).and_then(|s| s.parse().ok()).unwrap_or(0);
        st.storage.insert(debit_key, (sbal.saturating_sub(deposit)).to_string().into_bytes());
        eprintln!("  💰 fn-call deposit {} yocto: {} → {}", deposit, predecessor, account);
    }
    let saved_dep = CURRENT_DEPOSIT.with(|d| d.borrow_mut().replace(deposit));

    eprintln!("  ↳ cross: {}.{}({})", account, method, String::from_utf8_lossy(args));
    let part_snap = { snapshot_partition(&state.lock().unwrap(), account) };

    let mut sub_store = wasmtime::Store::new(&*engine, ());
    sub_store.set_fuel(PREPAID_FUEL.with(|f| *f.borrow()))?;
    let linker = build_env_linker(&mut sub_store, &*engine, state.clone(), Vec::new())?;
    let instance = linker.instantiate(&mut sub_store, &module)?;
    let ok = instance
        .get_func(&mut sub_store, method)
        .map(|f| f.call(&mut sub_store, &[], &mut []));
    let (trap, ret) = match ok {
        None => (true, None), // missing method = failed receipt
        Some(res) => match res {
            Ok(()) => (false, state.lock().unwrap().return_data.clone()),
            Err(_) => (true, None),
        },
    };
    if trap {
        eprintln!("  ⚠ cross: {}.{} TRAPPED — reverting partition", account, method);
        restore_partition(&mut state.lock().unwrap(), part_snap, account);
        if deposit > 0 {
            // failed receipt refunds its deposit to the sender
            let mut st = state.lock().unwrap();
            let rk = prefixed_key(predecessor, b"\x00near-bal");
            let b: u128 = st.storage.get(&rk)
                .and_then(|v| std::str::from_utf8(v).ok()).and_then(|s| s.parse().ok()).unwrap_or(0);
            st.storage.insert(rk, (b + deposit).to_string().into_bytes());
            eprintln!("  💰 deposit {} refunded to {} (failed receipt)", deposit, predecessor);
        }
    }
    CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = saved_dep);

    // Restore isolation context
    EXEC_CTX.with(|c| *c.borrow_mut() = old_ctx);
    {
        let mut st = state.lock().unwrap();
        st.registers = old_regs;
        st.return_data = old_ret;
    }
    Ok(if trap { None } else { ret })
}

/// Resolve a promise DAG node: deps first (their results, flattened,
/// become this batch's promise_results), then this batch's actions.
fn execute_promise(idx: usize) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>> {
    let batch = PROMISE_DAG.with(|d| d.borrow()[idx].clone());
    EXECUTED_PROMISES.with(|e| e.borrow_mut().insert(idx));
    let mut dep_results: Vec<Option<Vec<u8>>> = Vec::new();
    for dep in &batch.deps {
        dep_results.extend(execute_promise(*dep)?);
    }
    let saved =
        PROMISE_RESULTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), dep_results.clone()));
    let mut out = Vec::new();
    let mut batch_touched: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    if batch.is_yield {
        // NEAR yield: the callback runs once NOW with a NotReady result
        // (promiseSucceeded(0)==0 → contract returns its pending path),
        // then re-runs with the payload when host 83 resumes the handle.
        let saved = PROMISE_RESULTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), vec![None]));
        for action in &batch.actions {
            if let PAction::FnCall { method, args, dep, .. } = action {
                match sub_execute(&batch.account, method, args, &batch.creator, *dep) {
                    Ok(ret) => out.push(ret),
                    Err(_) => out.push(None),
                }
            }
        }
        PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
        return Ok(out);
    }
    if !batch.account.is_empty() {
        for action in &batch.actions {
            match action {
                PAction::FnCall { method, args, dep, .. } => {
                    // NEAR receipt ordering: if the child RETURNS a promise
                    // (promise_return), its receipts execute BEFORE this
                    // batch's dependents — the mock used to skip them, so
                    // a flash-loan settle ran before the borrower's repay
                    // transfer landed (flashpool protocol, 2026-09-01).
                    // CLEAR first: PENDING_RETURN is process-wide TLS — an
                    // ancestor's entry promise_return would leak in and we'd
                    // re-execute the WHOLE returned subtree inside the child
                    // (double transfers, phantom settles).
                    let outer_ret = PENDING_RETURN.with(|p| p.borrow_mut().take());
                    let r = sub_execute(&batch.account, method, args, &batch.creator, *dep)?;
                    // NEAR receipt semantics (matches the airdrop suite):
                    // a trapped FnCall reverts ITSELF only — its result is
                    // Failed for descendants (promiseSucceeded=0) and
                    // SIBLINGS stay committed. The flashloan lesson: the
                    // transfer-out receipt COMMITS; a stiff borrower keeps
                    // the funds; the settle aborts fail-closed. This is
                    // exactly why real pools whitelist borrowers.
                    out.push(r);
                    let child_ret = PENDING_RETURN.with(|p| std::mem::replace(&mut *p.borrow_mut(), outer_ret));
                    if let Some(ridx) = child_ret {
                        eprintln!("  ⛓ child returned promise {} — resolving before dependents", ridx);
                        let cres = execute_promise(ridx)?;
                        for r in cres { out.push(r); }
                    }
                }
                PAction::Transfer(amt) => {
                    // NEAR semantics: transfers carry REAL value; a receipt is
                    // atomic but SIBLING receipts commit independently. On
                    // insufficient balance THIS receipt reverts (only the
                    // partitions it touched) and yields a FAILED promise
                    // result — the callback decides (fail-closed pattern).
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    let debit_key = prefixed_key(&batch.creator, b"\x00near-bal");
                    let bal: u128 = st
                        .storage
                        .get(&debit_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    if bal < *amt {
                        eprintln!(
                            "  ⚠ transfer {} yocto → {}: creator {} has {} — INSUFFICIENT (this receipt reverts)",
                            amt, batch.account, batch.creator, bal
                        );
                        // revert everything this batch touched so far
                        for (k, v) in batch_touched.iter() {
                            match v {
                                Some(val) => {
                                    st.storage.insert(k.clone(), val.clone());
                                }
                                None => {
                                    st.storage.remove(k);
                                }
                            }
                        }
                        out.push(None); // FAILED promise result
                        break; // skip the rest of THIS receipt only
                    }
                    batch_touched.push((
                        debit_key.clone(),
                        st.storage.get(&debit_key).cloned(),
                    ));
                    st.storage.insert(debit_key, (bal - *amt).to_string().into_bytes());
                    let credit_key = prefixed_key(&batch.account, b"\x00near-bal");
                    batch_touched.push((
                        credit_key.clone(),
                        st.storage.get(&credit_key).cloned(),
                    ));
                    let rbal: u128 = st
                        .storage
                        .get(&credit_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    let rbal = rbal + *amt;
                    st.storage.insert(credit_key, rbal.to_string().into_bytes());
                    eprintln!("  ↗ transfer {} yocto → {} (bal now {})", amt, batch.account, rbal);
                    // TRUE NEAR: successful transfer = Successful(empty).
                    // Contracts distinguish it from Failed via
                    // near/promise_succeeded (status probe). No more marker.
                    out.push(Some(vec![]));
                }
            }
        }
    }
    PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
    // Pure combinator (promise_and): its "results" ARE the flattened child
    // outputs — a promise_then on an and-node must see [p1_outs..., p2_outs...]
    // (NEAR semantics). Batches with an account return only their own
    // action outputs.
    if batch.account.is_empty() {
        Ok(dep_results)
    } else {
        Ok(out)
    }
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
                Some(PAction::FnCall { method, args, gas, .. }) => (method.clone(), args.clone(), *gas),
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
fn build_env_linker(
    store: &mut wasmtime::Store<()>,
    engine: &wasmtime::Engine,
    state: std::sync::Arc<Mutex<MockState>>,
    single_input: Vec<u8>,
) -> Result<wasmtime::Linker<()>, Box<dyn std::error::Error>> {
    let mut linker = wasmtime::Linker::new(engine);
    // === Host functions (all created before linking) ===

    let _s1 = state.clone();
    let log_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            // Indicative legacy fees: utf8 log base + per byte
            let cost = 13_181_732u64 + 19_335_348u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let msg = String::from_utf8_lossy(&data[ptr..ptr + len]).to_string();
                    println!("  LOG: {}  [debug len={} ptr={}]", msg, len, ptr);
                } else {
                    println!("  LOG: <out-of-range> [debug len={} ptr={}]", len, ptr);
                }
            }
            Ok(())
        },
    );

    let s2 = state.clone();
    let value_return_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            eprintln!("  → value_return(len={}, ptr={})", len, ptr);
            // Indicative legacy fees: read_memory base + per byte
            let cost = 4_141_250u64 + 3_574_166u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let mut st = s2.lock().unwrap();
                    if st.return_data.is_none() {
                        st.return_data = Some(data[ptr..ptr + len].to_vec());
                    }
                }
            }
            Ok(())
        },
    );

    let s3 = state.clone();
    let read_register_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (rid, ptr) = (args[0].unwrap_i64() as u64, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                if let Some(data) = s3.lock().unwrap().registers.get(&rid).cloned() {
                    // Indicative legacy fees: base + per byte
                    let cost = 24_108_449u64 + 3_574_166u64 * data.len() as u64;
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    let md = mem.data_mut(&mut caller);
                    if ptr + data.len() <= md.len() {
                        md[ptr..ptr + data.len()].copy_from_slice(&data);
                        eprintln!("  → read_register({}, ptr={}) ok {}b", rid, ptr, data.len());
                    } else {
                        eprintln!(
                            "  ⚠ read_register({}, ptr={}): {}b doesn't fit in mem({})",
                            rid,
                            ptr,
                            data.len(),
                            md.len()
                        );
                    }
                } else {
                    // near-core semantics: reading a missing register is a host
                    // error (InvalidRegisterId) — the contract traps.
                    eprintln!("  ⚠ read_register({}): not found → trap", rid);
                    return Err(wasmtime::Error::msg(format!("InvalidRegisterId {{ register_id: {} }}", rid)));
                }
            }
            Ok(())
        },
    );

    let s4 = state.clone();
    let register_len_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![ValType::I64]),
        move |mut caller, args, results| {
            let rid = args[0].unwrap_i64() as u64;
            // near-core: len of a missing register is u64::MAX sentinel
            // (not an error). Returned as i64 == -1.
            let len = s4
                .lock()
                .unwrap()
                .registers
                .get(&rid)
                .map(|d| d.len() as i64)
                .unwrap_or(-1);
            // Indicative legacy fee
            caller.set_fuel(caller.get_fuel()?.saturating_sub(21_165_243))?;
            eprintln!("  → register_len({}) = {}", rid, len);
            results[0] = Val::I64(len);
            Ok(())
        },
    );

    let s5 = state.clone();
    let input_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let single_input = single_input.clone();
            let rid = args[0].unwrap_i64() as u64;
            eprintln!("  → input(reg={})", rid);
            let bytes = EXEC_CTX
                .with(|c| c.borrow().as_ref().map(|x| x.input.clone()))
                .unwrap_or_else(|| single_input.clone());
            // Indicative legacy fee: write_register base + per byte
            let cost = 21_165_243u64 + 3_574_166u64 * bytes.len() as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            let mut st = s5.lock().unwrap();
            // Real NEAR semantics: input() ALWAYS writes the args into the
            // register, overwriting any prior value. The old contains_key
            // guard silently kept stale values (e.g. a predecessor_account_id
            // that had just used reg 0) — parsers then walked the wrong bytes.
            write_reg_checked(&mut st, rid, bytes).map_err(|e| wasmtime::Error::msg(e))?;
            Ok(())
        },
    );

    let s6 = state.clone();
    let storage_write_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 5], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp, vl, vp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
                args[4].unwrap_i64() as u64,
            );
            if exec_ctx_view(&s6) {
                return Err(wasmtime::Error::msg("ProhibitedInView: storage_write"));
            }
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() && vp + vl <= md.len() {
                    let raw_key = md[kp..kp + kl].to_vec();
                    let acct = exec_ctx_or_default().contract;
                    let key = prefixed_key(&acct, &raw_key);
                    let val = md[vp..vp + vl].to_vec();
                    eprintln!(
                        "  → storage_write(\"{}\") = {}b",
                        String::from_utf8_lossy(&raw_key),
                        vl
                    );
                    // Indicative legacy fees: base + key/value bytes
                    let cost = 64_000_000u64
                        + 90_563u64 * kl as u64
                        + 3_548_576u64 * vl as u64;
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    let mut st = s6.lock().unwrap();
                    let trie = trie_charge_write(&mut st, &key);
                    let old = st.storage.insert(key, val);
                    drop(st);
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(trie))?;
                    let mut st = s6.lock().unwrap();
                    if rid != u64::MAX {
                        if let Some(old) = old {
                            write_reg_checked(&mut st, rid, old)
                                .map_err(|e| wasmtime::Error::msg(e))?;
                        }
                    }
                }
            }
            results[0] = Val::I64(0);
            Ok(())
        },
    );

    let s7 = state.clone();
    let storage_read_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Step 1: read key from WASM memory (borrows caller)
            let key_from_mem: Option<Vec<u8>> = {
                if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let md = mem.data(&caller);
                    if kp + kl <= md.len() {
                        Some(md[kp..kp + kl].to_vec())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }; // caller borrow DROPPED here
            let key_from_mem = key_from_mem.map(|k| {
                let acct = exec_ctx_or_default().contract;
                prefixed_key(&acct, &k)
            });

            // Step 2: search HashMap (no caller borrow)
            let found = if let Some(key) = &key_from_mem {
                let mut st = s7.lock().unwrap();
                if let Some(val) = st.storage.get(key).cloned() {
                    eprintln!("  → storage_read found {}b", val.len());
                    // Indicative flat fees + production trie-node access
                    let trie = trie_charge(&mut st, key);
                    let cost = 56_356_995u64
                        + 81_569u64 * kl as u64
                        + 3_574_166u64 * val.len() as u64
                        + trie;
                    drop(st);
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    let mut st = s7.lock().unwrap();
                    write_reg_checked(&mut st, rid, val).map_err(|e| wasmtime::Error::msg(e))?;
                    true
                } else {
                    eprintln!(
                        "  → storage_read not found [{}]",
                        String::from_utf8_lossy(key)
                    );
                    // production charges the read base + trie walk even on miss
                    let trie = trie_charge(&mut st, key);
                    let cost = 56_356_995u64 + 81_569u64 * kl as u64 + trie;
                    drop(st);
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    false
                }
            } else {
                false
            };

            results[0] = Val::I64(if found { 1 } else { 0 });
            Ok(())
        },
    );

    let s8 = state.clone();
    let storage_remove_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            if exec_ctx_view(&s8) {
                return Err(wasmtime::Error::msg("ProhibitedInView: storage_remove"));
            }
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() {
                    let rkey = {
                        let raw = md[kp..kp + kl].to_vec();
                        let acct = exec_ctx_or_default().contract;
                        prefixed_key(&acct, &raw)
                    };
                    let (val, trie) = {
                        let mut st = s8.lock().unwrap();
                        (st.storage.remove(&rkey), trie_charge_write(&mut st, &rkey))
                    };
                    if let Some(val) = val {
                        // Indicative legacy fees: base + key bytes + trie access
                        let cost = 64_000_000u64 + 90_563u64 * kl as u64 + trie;
                        caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                        if rid != u64::MAX {
                            let mut st = s8.lock().unwrap();
                            write_reg_checked(&mut st, rid, val)
                                .map_err(|e| wasmtime::Error::msg(e))?;
                        }
                        results[0] = Val::I64(1);
                        return Ok(());
                    }
                }
            }
            results[0] = Val::I64(0);
            Ok(())
        },
    );

    let s9 = state.clone();
    let storage_has_key_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() {
                    let hkey = {
                        let raw = md[kp..kp + kl].to_vec();
                        let acct = exec_ctx_or_default().contract;
                        prefixed_key(&acct, &raw)
                    };
                    let (has, trie) = {
                        let mut st = s9.lock().unwrap();
                        (st.storage.contains_key(&hkey), trie_charge(&mut st, &hkey))
                    };
                    // Indicative legacy fees + trie-node access
                    let cost = 56_356_995u64 + 81_569u64 * kl as u64 + trie;
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    results[0] = Val::I64(if has { 1 } else { 0 });
                    return Ok(());
                }
            }
            results[0] = Val::I64(0);
            Ok(())
        },
    );

    let panic_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            let msg = if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    String::from_utf8_lossy(&data[ptr..ptr + len]).to_string()
                } else {
                    format!("(bad ptr {}/{})", ptr, len)
                }
            } else {
                "(no mem)".into()
            };
            Err(wasmtime::Error::msg(format!("PANIC: {}", msg)))
        },
    );

    let abort_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![]),
        |_, _, _| Err(wasmtime::Error::msg("ABORT")),
    );

    let s_ca = state.clone();
    let current_account_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            let acct = exec_ctx_or_default().contract;
            let acct = if acct.is_empty() { "escrow.test.near".to_string() } else { acct };
            s_ca.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, acct.into_bytes());
            Ok(())
        },
    );

    let s_sa = state.clone();
    let signer_account_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            // NEAR_MOCK_SIGNER overrides the tx signer — liquidation tests
            // need caller ≠ account owner (default stays owner.test.near).
            let signer = {
                let ctx = exec_ctx_or_default();
                if EXEC_CTX.with(|c| c.borrow().is_some()) {
                    ctx.signer
                } else {
                    std::env::var("NEAR_MOCK_SIGNER")
                        .unwrap_or_else(|_| "owner.test.near".into())
                }
            };
            s_sa.lock().unwrap().registers.insert(
                args[0].unwrap_i64() as u64,
                signer.into_bytes(),
            );
            Ok(())
        },
    );

    let s_pa = state.clone();
    let predecessor_account_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            let pred = {
                let ctx = exec_ctx_or_default();
                if EXEC_CTX.with(|c| c.borrow().is_some()) {
                    ctx.predecessor
                } else {
                    "owner.test.near".to_string()
                }
            };
            s_pa.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, pred.into_bytes());
            Ok(())
        },
    );

    let s_pk = state.clone();
    let signer_pk_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_pk.lock().unwrap().registers.insert(
                args[0].unwrap_i64() as u64,
                b"ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_vec(),
            );
            Ok(())
        },
    );

    let block_ts_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(
                // NEAR host returns NANoseconds — mock must match the real
                // scale (was millis: silent 1e6x unit divergence).
                // NEAR_MOCK_BLOCK_TS pins it for deterministic time tests
                // (interest accrual): real clock otherwise.
                std::env::var("NEAR_MOCK_BLOCK_TS")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos() as i64
                    }),
            );
            Ok(())
        },
    );

    let s_ab = state.clone();
    let account_balance_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            // ABI: args[0] = PTR (16-byte write target). Writes the
            // contract's real near-bal (was zeros; also hit a register-id
            // bug — flashpool settle, 2026-09-01).
            let contract = exec_ctx_or_default().contract;
            let amt: u128 = STATE_ARC
                .with(|s| s.borrow().clone())
                .and_then(|st| {
                    let st = st.lock().unwrap();
                    st.storage
                        .get(&prefixed_key(&contract, b"\x00near-bal"))
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(0);
            let ptr = args[0].unwrap_i64() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data_mut(&mut caller);
                if ptr + 16 <= md.len() {
                    md[ptr..ptr + 16].copy_from_slice(&amt.to_le_bytes());
                }
            }
            Ok(())
        },
    );

    let _s_ad = state.clone();
    let attached_deposit_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let ptr = args[0].unwrap_i64() as usize;
            // Real host shape: 16 LE bytes of THIS receipt's deposit.
            // Reads NEAR_MOCK_ATTACH (same var the balance-credit path
            // uses — was always 0: the auction protocol reads it, and
            // value-receiving entries silently saw nothing. 2026-09-01.)
            let amt: u128 = CURRENT_DEPOSIT.with(|d| *d.borrow())
                .or_else(|| std::env::var("NEAR_MOCK_ATTACH").ok().and_then(|s| s.trim().parse().ok()))
                .unwrap_or(0);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data_mut(&mut caller);
                if ptr + 16 <= md.len() {
                    md[ptr..ptr + 16].copy_from_slice(&amt.to_le_bytes());
                }
            }
            Ok(())
        },
    );

    // Noop stubs with correct arities
    // Real gas accounting: fuel consumed so far (used_gas)
    let used_gas_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        move |mut caller, _, results| {
            let remaining = caller.get_fuel().unwrap_or(PREPAID_FUEL.with(|f| *f.borrow()));
            results[0] = Val::I64(PREPAID_FUEL.with(|f| *f.borrow()).saturating_sub(remaining) as i64);
            Ok(())
        },
    );
    let prepaid_gas_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        move |_, _, results| {
            results[0] = Val::I64(PREPAID_FUEL.with(|f| *f.borrow()) as i64);
            Ok(())
        },
    );

    // sha256(len, ptr, rid) — real digest to register (was noop)
    let sg1 = state.clone();
    let sha256_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use sha2::{Digest, Sha256};
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Indicative legacy fees
            let cost = 45_760_404u64 + 18_217u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    let digest: Vec<u8> = Sha256::digest(&md[ptr..ptr + len]).to_vec();
                    let mut st = sg1.lock().unwrap();
                    write_reg_checked(&mut st, rid, digest).map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    // keccak256(len, ptr, rid)
    let sg2 = state.clone();
    let keccak256_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use sha3::Keccak256;
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Indicative legacy fees
            let cost = 45_760_404u64 + 18_217u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    use sha3::digest::Digest;
                    let digest: Vec<u8> = Keccak256::digest(&md[ptr..ptr + len]).to_vec();
                    let mut st = sg2.lock().unwrap();
                    write_reg_checked(&mut st, rid, digest).map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    // write_register(len, ptr, rid) — real checked write (was noop)
    let sg3 = state.clone();
    let write_register_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Indicative legacy fees
            let cost = 21_165_243u64 + 3_574_166u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    let data = md[ptr..ptr + len].to_vec();
                    let mut st = sg3.lock().unwrap();
                    write_reg_checked(&mut st, rid, data).map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );

    // === Exotic crypto hosts (2026-09-01, surface_tour2_exotic) ===
    // The exotic battery instantiates AND runs these — deterministic mock
    // digests (real crypto for keccak/ripemd; fixed-shape stubs for the
    // signature/precompile families that protocol #16 will exercise on
    // testnet). All digest hosts follow the (len, ptr, rid) register ABI.
    let sg_keccak512 = state.clone();
    let keccak512_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use sha3::digest::{ExtendableOutput, Update, XofReader};
            use sha3::Shake128;
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    // SHAKE128@64B stands in for Keccak-512 (same crate family,
                    // deterministic, 64-byte output — mock fidelity is shape)
                    let mut h = Shake128::default();
                    h.update(&md[ptr..ptr + len]);
                    let mut rd = h.finalize_xof();
                    let mut digest = vec![0u8; 64];
                    rd.read(&mut digest);
                    let mut st = sg_keccak512.lock().unwrap();
                    write_reg_checked(&mut st, rid, digest)
                        .map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    let sg_ripemd = state.clone();
    let ripemd160_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use ripemd::{Digest as RipemdDigest, Ripemd160};
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    let digest: Vec<u8> = Ripemd160::digest(&md[ptr..ptr + len]).to_vec();
                    let mut st = sg_ripemd.lock().unwrap();
                    write_reg_checked(&mut st, rid, digest)
                        .map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    // p256_verify(hash_len, hash_ptr, sig_len, sig_ptr, pk_len, pk_ptr) -> i64
    // (NEAR ABI: 6 i64 args → i64). Mock: shape-check then 1 (verify OK).
    let p256_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |_, args, results| {
            let hash_len = args[0].unwrap_i64() as usize;
            let sig_len = args[2].unwrap_i64() as usize;
            let pk_len = args[4].unwrap_i64() as usize;
            results[0] = if hash_len == 32 && sig_len == 64 && pk_len == 33 {
                Val::I64(1)
            } else {
                Val::I64(0)
            };
            Ok(())
        },
    );
    // ecrecover(7 args) -> i64 (value_return register id); mock writes a
    // 42-char hex address to the register named by the LAST arg and returns it
    let sg_ecr = state.clone();
    let ecrecover_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 7], vec![ValType::I64]),
        move |_, args, results| {
            let rid = args[6].unwrap_i64() as u64;
            let addr: Vec<u8> = b"0x1234567890abcdef1234567890abcdef12345678".to_vec();
            let mut st = sg_ecr.lock().unwrap();
            write_reg_checked(&mut st, rid, addr).map_err(|e| wasmtime::Error::msg(e))?;
            results[0] = Val::I64(args[6].unwrap_i64());
            Ok(())
        },
    );
    // alt_bn128 + bls12381 precompiles: (data_len, data_ptr, rid) → i64
    // Mock: register a fixed-shape blob so .length probes are deterministic;
    // return the register id (matches the compiler's read-to-register ABI).
    let precompile_targets: [(&str, i64, &str); 2] = [
        ("alt_bn128_g1_sum", 64, "g1sum"),
        ("alt_bn128_g1_multiexp", 64, "g1x"),
    ];
    // alt_bn128_g1_sum/g1_multiexp: (data_len, data_ptr, rid) — no return.
    // bls12381_*: same args → i64 (read-to-register ABI returns the rid).
    let mut precompile_fns = Vec::new();
    for (i, (_nm, out_len, tag)) in precompile_targets.iter().enumerate() {
        let st_g = state.clone();
        let out_len = *out_len;
        let tag = tag.as_bytes().to_vec();
        let returns = false; // alt_bn128_*: no return value
        precompile_fns.push(Func::new(
            &mut *store,
            FuncType::new(
                &engine,
                vec![ValType::I64; 3],
                if returns { vec![ValType::I64] } else { vec![] },
            ),
            move |_, args, results| {
                let rid = args[2].unwrap_i64() as u64;
                // pad/trim the tag to out_len — deterministic shape probe
                let mut blob = Vec::with_capacity(out_len as usize);
                while blob.len() < out_len as usize {
                    let take = (out_len as usize - blob.len()).min(tag.len());
                    blob.extend_from_slice(&tag[..take]);
                }
                let mut st = st_g.lock().unwrap();
                write_reg_checked(&mut st, rid, blob).map_err(|e| wasmtime::Error::msg(e))?;
                if returns {
                    results[0] = Val::I64(rid as i64);
                }
                Ok(())
            },
        ));
    }
    // bls12381_* — NEAR host ABI: 3-arg (len, ptr, rid) → i64 status.
    // Byte-faithful validation: verbatim port of nearcore's bls12381.rs
    // (real blst — on-curve, subgroup, canonical-encoding, sign-byte checks;
    // sign-free 96/192B outputs). Length errors are HOST ERRORS (trap),
    // matching nearcore's BLS12381InvalidInput; malformed points/signs →
    // ret 1 with the register untouched.
    let bls_targets: [(&str, u8); 8] = [
        ("bls12381_p1_sum", bls_validate::kind::P1_SUM),
        ("bls12381_p2_sum", bls_validate::kind::P2_SUM),
        ("bls12381_g1_multiexp", bls_validate::kind::G1_MULTIEXP),
        ("bls12381_g2_multiexp", bls_validate::kind::G2_MULTIEXP),
        ("bls12381_map_fp_to_g1", bls_validate::kind::MAP_FP_TO_G1),
        ("bls12381_map_fp2_to_g2", bls_validate::kind::MAP_FP2_TO_G2),
        ("bls12381_p1_decompress", bls_validate::kind::P1_DECOMPRESS),
        ("bls12381_p2_decompress", bls_validate::kind::P2_DECOMPRESS),
    ];
    let mut bls_fns = Vec::new();
    for (_nm, kind_id) in bls_targets.iter() {
        let st_g = state.clone();
        let kind_id = *kind_id;
        bls_fns.push(Func::new(
            &mut *store,
            FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
            move |mut caller, args, results| {
                let len = args[0].unwrap_i64();
                let ptr = args[1].unwrap_i64();
                let rid = args[2].unwrap_i64() as u64;
                let Some(data) = read_guest_bytes(&mut caller, len, ptr) else {
                    return Err(wasmtime::Error::msg(format!(
                        "MemoryAccessViolation: bls12381 host read {}b @ {:#x}",
                        len, ptr
                    )));
                };
                match bls_validate::eval(kind_id, &data) {
                    Err(e) => Err(wasmtime::Error::msg(e.to_string())),
                    Ok(None) => {
                        results[0] = Val::I64(1);
                        Ok(())
                    }
                    Ok(Some(out)) => {
                        let mut st = st_g.lock().unwrap();
                        write_reg_checked(&mut st, rid, out)
                            .map_err(|e| wasmtime::Error::msg(e))?;
                        results[0] = Val::I64(0);
                        Ok(())
                    }
                }
            },
        ));
    }
    // bls12381_pairing_check: (len, ptr) → i64. nearcore semantics:
    // 0 = check passed, 1 = malformed point/encoding, 2 = well-formed but
    // pairing ≠ 1. Empty input is vacuously true → 0. Bad total length is
    // a host error (trap), like BLS12381InvalidInput on testnet.
    let bls_pairing_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let len = args[0].unwrap_i64();
            let ptr = args[1].unwrap_i64();
            let Some(data) = read_guest_bytes(&mut caller, len, ptr) else {
                return Err(wasmtime::Error::msg(format!(
                    "MemoryAccessViolation: bls12381_pairing_check read {}b @ {:#x}",
                    len, ptr
                )));
            };
            match bls_validate::pairing_check(&data) {
                Err(e) => Err(wasmtime::Error::msg(e.to_string())),
                Ok(code) => {
                    results[0] = Val::I64(code as i64);
                    Ok(())
                }
            }
        },
    );

    let noop1 = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        |_, _, _| Ok(()),
    );
    let noop0r = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_2i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_3i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_3i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_2i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_4i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 4], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_6i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_7i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 7], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_7i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 7], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_8i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 8], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_9i = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 9], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_4i_i32 = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 4], vec![ValType::I32]),
        |_, _, r| {
            r[0] = Val::I32(0);
            Ok(())
        },
    );
    let noop_4i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 4], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_8i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 8], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_9i_1o = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 9], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );

    // === Link ===
    let env_memory = wasmtime::Memory::new(&mut *store, wasmtime::MemoryType::new(1024, None))?;
    linker.define(&*store, "env", "memory", env_memory)?;
    linker.define(&*store, "env", "log_utf8", log_fn)?;
    linker.define(&*store, "env", "value_return", value_return_fn)?;
    linker.define(&*store, "env", "read_register", read_register_fn)?;
    linker.define(&*store, "env", "register_len", register_len_fn)?;
    linker.define(&*store, "env", "input", input_fn)?;
    linker.define(&*store, "env", "storage_write", storage_write_fn)?;
    linker.define(&*store, "env", "storage_read", storage_read_fn)?;
    linker.define(&*store, "env", "storage_remove", storage_remove_fn)?;
    linker.define(&*store, "env", "storage_has_key", storage_has_key_fn)?;
    linker.define(&*store, "env", "panic_utf8", panic_fn)?;
    linker.define(&*store, "env", "panic", abort_fn.clone())?;
    linker.define(&*store, "env", "abort", abort_fn)?;
    linker.define(&*store, "env", "current_account_id", current_account_fn)?;
    linker.define(&*store, "env", "signer_account_id", signer_account_fn)?;
    linker.define(&*store, "env", "signer_account_pk", signer_pk_fn)?;
    linker.define(
        &*store,
        "env",
        "predecessor_account_id",
        predecessor_account_fn,
    )?;
    let block_index_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(
                // NEAR_MOCK_BLOCK_HEIGHT pins it for deterministic
                // block-conditioned protocols (auction deadlines); a real
                // chain height otherwise (mock: fixed genesis-ish 1000).
                std::env::var("NEAR_MOCK_BLOCK_HEIGHT")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1000),
            );
            Ok(())
        },
    );
    linker.define(&*store, "env", "block_index", block_index_fn)?;
    linker.define(&*store, "env", "block_timestamp", block_ts_fn)?;
    linker.define(&*store, "env", "account_balance", account_balance_fn)?;
    linker.define(&*store, "env", "attached_deposit", attached_deposit_fn)?;
    linker.define(&*store, "env", "used_gas", used_gas_fn)?;
    linker.define(&*store, "env", "prepaid_gas", prepaid_gas_fn)?;
    // random_seed(register_id) — writes 32 bytes to the register (real NEAR
    // contract). Was noop → read_register trapped on the missing register
    // (caught by the API sweep 2026-08-31). Deterministic per-run seed.
    let rs1 = state.clone();
    let random_seed_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_caller, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            // Deterministic 32B seed → 64-char lowercase hex (real NEAR
            // returns raw bytes; the compiler's read_to_register path keeps
            // bytes, but the TS surface stringifies as hex — parity with
            // the ctx battery's `seed.length == 64` probe).
            let seed: Vec<u8> = (0u32..8)
                .flat_map(|i| (0x5EED_0000u32.wrapping_add(i)).to_le_bytes())
                .collect();
            let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
            let mut st = rs1.lock().unwrap();
            write_reg_checked(&mut st, rid, hex.into_bytes())
                .map_err(|e| wasmtime::Error::msg(e))?;
            Ok(())
        },
    );
    linker.define(&*store, "env", "random_seed", random_seed_fn)?;
    linker.define(&*store, "env", "sha256", sha256_fn)?;
    // schnorr_verify_bip340(pk_ptr: i32, sig_ptr: i32, msg_ptr: i32, msg_len: i32) -> i32
    let schnorr_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |mut caller, params, results| {
            let pk_ptr = params[0].unwrap_i32() as usize;
            let sig_ptr = params[1].unwrap_i32() as usize;
            let msg_ptr = params[2].unwrap_i32() as usize;
            let msg_len = params[3].unwrap_i32() as usize;
            
            let mem = caller.get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("missing memory export");
            let data = mem.data(&caller);
            
            eprintln!("[schnorr-dbg] entry pk_ptr={} sig_ptr={} msg_ptr={} msg_len={} mem_len={}", pk_ptr, sig_ptr, msg_ptr, msg_len, data.len());
            if pk_ptr + 32 > data.len() || sig_ptr + 64 > data.len() || msg_ptr + msg_len > data.len() {
                eprintln!("[schnorr-dbg] BOUNDS REJECT");
                results[0] = Val::I32(0);
                return Ok(());
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let pk: [u8; 32] = data[pk_ptr..pk_ptr+32].try_into().unwrap();
                let sig: [u8; 64] = data[sig_ptr..sig_ptr+64].try_into().unwrap();
                let msg = &data[msg_ptr..msg_ptr+msg_len];
                eprintln!("[schnorr-dbg] pk_ptr={} sig_ptr={} msg_ptr={} msg_len={}", pk_ptr, sig_ptr, msg_ptr, msg_len);
                eprintln!("[schnorr-dbg] pk[0..8]={:02x?} sig[0..8]={:02x?} msg[0..8]={:02x?}", &pk[0..8], &sig[0..8], &msg[msg.len().min(8)..msg.len().min(16).max(8)]);
                let r = schnorr_verify_impl(&pk, &sig, msg) as i32;
                eprintln!("[schnorr-dbg] result={}", r);
                r
            })).unwrap_or_else(|_| { eprintln!("[schnorr-dbg] PANIC"); 0 });
            
            results[0] = Val::I32(result);
            Ok(())
        },
    );
    linker.define(&*store, "env", "schnorr_verify_bip340", schnorr_fn)?;
    // ed25519_verify — real host ABI: (sig_len: i64, sig_ptr: i64,
    // msg_len: i64, msg_ptr: i64, pk_len: i64, pk_ptr: i64) -> i64 (1/0).
    // Signature is 64 bytes (R||s), pk 32 bytes; pk_len/sig_len must match
    // or reject, mirroring VMLogic's length checks.
    let ed25519_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |mut caller, params, results| {
            let sig_len = params[0].unwrap_i64() as usize;
            let sig_ptr = params[1].unwrap_i64() as usize;
            let msg_len = params[2].unwrap_i64() as usize;
            let msg_ptr = params[3].unwrap_i64() as usize;
            let pk_len = params[4].unwrap_i64() as usize;
            let pk_ptr = params[5].unwrap_i64() as usize;
            let mem = caller.get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("missing memory export");
            let data = mem.data(&caller);
            if pk_len != 32 || sig_len != 64
                || pk_ptr + 32 > data.len()
                || sig_ptr + 64 > data.len()
                || msg_ptr + msg_len > data.len()
            {
                results[0] = Val::I64(0);
                return Ok(());
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let pk: [u8; 32] = data[pk_ptr..pk_ptr + 32].try_into().unwrap();
                let sig: [u8; 64] = data[sig_ptr..sig_ptr + 64].try_into().unwrap();
                let msg = &data[msg_ptr..msg_ptr + msg_len];
                ed25519_verify_impl(&pk, &sig, msg) as i64
            }))
            .unwrap_or(0);
            results[0] = Val::I64(result);
            Ok(())
        },
    );
    linker.define(&*store, "env", "ed25519_verify", ed25519_fn)?;
    // log_utf16(len: i64, ptr: i64) — utf16 log; mock decodes lossily for
    // display (same fee model as log_utf8).
    let log_utf16_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            let cost = 13_181_732u64 + 19_335_348u64 * len as u64;
            caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let units: Vec<u16> = data[ptr..ptr + len]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let msg = String::from_utf16_lossy(&units);
                    println!("  LOG: {}  [debug len={} ptr={}] (utf16)", msg, len, ptr);
                } else {
                    println!("  LOG: <out-of-range> [debug len={} ptr={}] (utf16)", len, ptr);
                }
            }
            Ok(())
        },
    );
    linker.define(&*store, "env", "log_utf16", log_utf16_fn)?;
    linker.define(&*store, "env", "keccak256", keccak256_fn)?;
    linker.define(&*store, "env", "log", noop1.clone())?;
    linker.define(&*store, "env", "validator_stake", noop_3i.clone())?;
    linker.define(&*store, "env", "validator_total_stake", noop1.clone())?;
    linker.define(&*store, "env", "alt_bn128_g1_multiexp", precompile_fns[1].clone())?;
    linker.define(&*store, "env", "alt_bn128_g1_sum", precompile_fns[0].clone())?;
    // alt_bn128_pairing_check(data_len, data_ptr) -> i64 (0 = pairing OK per
    // NEAR ABI convention on the mock's fixed-shape inputs)
    let pairing_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        |_, _, results| {
            results[0] = Val::I64(0);
            Ok(())
        },
    );
    linker.define(&*store, "env", "alt_bn128_pairing_check", pairing_fn)?;
    linker.define(&*store, "env", "keccak512", keccak512_fn)?;
    linker.define(&*store, "env", "ripemd160", ripemd160_fn)?;
    linker.define(&*store, "env", "p256_verify", p256_fn)?;
    linker.define(&*store, "env", "ecrecover", ecrecover_fn)?;
    linker.define(&*store, "env", "bls12381_p1_sum", bls_fns[0].clone())?;
    linker.define(&*store, "env", "bls12381_p2_sum", bls_fns[1].clone())?;
    linker.define(&*store, "env", "bls12381_g1_multiexp", bls_fns[2].clone())?;
    linker.define(&*store, "env", "bls12381_g2_multiexp", bls_fns[3].clone())?;
    linker.define(&*store, "env", "bls12381_map_fp_to_g1", bls_fns[4].clone())?;
    linker.define(&*store, "env", "bls12381_map_fp2_to_g2", bls_fns[5].clone())?;
    linker.define(&*store, "env", "bls12381_p1_decompress", bls_fns[6].clone())?;
    linker.define(&*store, "env", "bls12381_p2_decompress", bls_fns[7].clone())?;
    // bls12381_pairing_check: define the NEAR-native ABI stub built above
    // (288B pairs, ret 0 = identity / 1 = bad). Until 2026-09-02 a stale
    // EIP-2537 define sat here (384B pairs, ret 1 = ok) — it was the ONLY
    // live define (the new stub was built but never registered), so the
    // gate ran inverted: any non-384-multiple "passed" (TASK-json-bug.md
    // gate tests caught it via the 512B short-H(m) case succeeding).
    linker.define(&*store, "env", "bls12381_pairing_check", bls_pairing_fn)?;

    linker.define(&*store, "env", "epoch_height", noop0r.clone())?;
    linker.define(&*store, "env", "storage_usage", noop0r.clone())?;
    linker.define(&*store, "env", "log_s", noop1.clone())?;
    linker.define(&*store, "env", "validator_account_id", noop1.clone())?;
    linker.define(&*store, "env", "promise_results", noop1.clone())?;
    // (yield hosts defined below — cross engine or noop, never twice)
    linker.define(&*store, "env", "account_locked_balance", noop1.clone())?;
    linker.define(&*store, "env", "storage_iter_prefix", noop_2i_1o.clone())?;
    linker.define(&*store, "env", "storage_iter_range", noop_4i_1o.clone())?;
    linker.define(&*store, "env", "storage_iter_next", noop_3i_1o.clone())?;
    linker.define(&*store, "env", "write_register", write_register_fn)?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_create_account",
        noop1.clone(),
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_deploy_contract",
        noop_3i.clone(),
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_function_call_weight",
        noop_8i,
    )?;

    linker.define(&*store, "env", "promise_batch_action_stake", noop_4i.clone())?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_key_with_full_access",
        noop_4i,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_key_with_function_call",
        noop_9i,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_delete_key",
        noop_3i.clone(),
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_delete_account",
        noop_3i,
    )?;

    // Real promise hosts (cross engine) — override the noops. STATE_ARC is
    // set by the drivers; when unset (defensive), noops remain.
    if STATE_ARC.with(|s| s.borrow().is_some()) {
        let (pc, pt, pa, prc, pr, pret, pbc, pbt, pafc, pbat, pyc, pyr) =
            build_promise_hosts(&mut *store, engine)?;
        linker.define(&*store, "env", "promise_create", pc)?;
        linker.define(&*store, "env", "promise_then", pt)?;
        linker.define(&*store, "env", "promise_and", pa)?;
        linker.define(&*store, "env", "promise_results_count", prc)?;
        linker.define(&*store, "env", "promise_result", pr)?;
        linker.define(&*store, "env", "promise_return", pret)?;
        linker.define(&*store, "env", "promise_batch_create", pbc)?;
        linker.define(&*store, "env", "promise_batch_then", pbt)?;
        linker.define(&*store, "env", "promise_batch_action_function_call", pafc)?;
        linker.define(&*store, "env", "promise_batch_action_transfer", pbat)?;
        linker.define(&*store, "env", "promise_yield_create", pyc)?;
        linker.define(&*store, "env", "promise_yield_resume", pyr)?;
    } else {
        linker.define(&*store, "env", "promise_create", noop_8i_1o.clone())?;
        linker.define(&*store, "env", "promise_then", noop_9i_1o.clone())?;
        linker.define(&*store, "env", "promise_and", noop_2i_1o.clone())?;
        linker.define(&*store, "env", "promise_batch_create", noop_2i_1o.clone())?;
        linker.define(&*store, "env", "promise_batch_then", noop_3i_1o.clone())?;
        linker.define(&*store, "env", "promise_results_count", noop0r.clone())?;
        linker.define(&*store, "env", "promise_result", noop_2i_1o.clone())?;
        linker.define(&*store, "env", "promise_return", noop1.clone())?;
        linker.define(&*store, "env", "promise_batch_action_function_call", noop_7i.clone())?;
        linker.define(&*store, "env", "promise_yield_create", noop_7i_1o)?;
        linker.define(&*store, "env", "promise_yield_resume", noop_4i_1o)?;
        linker.define(&*store, "env", "promise_batch_action_transfer", noop_2i.clone())?;
    }

    Ok(linker)
}

fn exec_ctx_view(state: &std::sync::Arc<Mutex<MockState>>) -> bool {
    EXEC_CTX.with(|c| c.borrow().as_ref().map(|x| x.view)).unwrap_or_else(|| state.lock().unwrap().view)
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("cross") {
        return run_cross(&args);
    }
    if args.len() < 3 {
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
    let func = instance
        .get_func(&mut store, method)
        .ok_or_else(|| format!("Method '{}' not found", method))?;
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

    match result {
        Ok(_) => {
            println!("✅ Success");
            let st = state.lock().unwrap();
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
                    println!(
                        "  [{}b]={} → [{}b]={}",
                        k.len(),
                        &ks[..ks.len().min(20)],
                        v.len(),
                        &vs[..vs.len().min(60)]
                    );
                }
            }
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
                println!("❌ OutOfGas — exceeded {:.6} Tgas prepaid", prepaid_tgas);
            } else {
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

struct MockState {
    storage: HashMap<Vec<u8>, Vec<u8>>,
    registers: HashMap<u64, Vec<u8>>,
    return_data: Option<Vec<u8>>,
    view: bool,
    /// keys already trie-touched this invocation (cached thereafter)
    touched: std::collections::HashSet<Vec<u8>>,
}

/// Register write with near-core limit semantics (logic/tests/registers.rs):
/// max 100 registers, max 1MiB per register.

/// Production trie-access charging (testnet PV85, EXPERIMENTAL_protocol_config
/// at block 266,843,869, fetched 2026-09-02):
///   touching_trie_node    = 2_280_000_000 gas / node
///   read_cached_trie_node = 2_280_000_000 gas / node (no read discount at PV85)
/// First touch of a key walks ~16 trie nodes (32-byte key depth in the mock
/// trie); repeats charge at the cached-read rate. Calibrated against the
/// near-vm-run oracle: view reads land within ~10% of production.
fn trie_charge(st: &mut MockState, key: &[u8]) -> u64 {
    if st.touched.insert(key.to_vec()) {
        16 * 2_280_000_000
    } else {
        2_280_000_000
    }
}

/// Writes re-walk the trie unconditionally (locate node + persist mutation) —
/// the read cache never subsidizes a write.
fn trie_charge_write(st: &mut MockState, key: &[u8]) -> u64 {
    st.touched.insert(key.to_vec());
    16 * 2_280_000_000
}

fn write_reg_checked(st: &mut MockState, rid: u64, data: Vec<u8>) -> Result<(), String> {
    const MAX_REGS: usize = 100;
    const MAX_REG_SIZE: usize = 1 << 20;
    if data.len() > MAX_REG_SIZE {
        return Err(format!(
            "MemoryAccessViolation: register {} value {}b exceeds max {}b",
            rid, data.len(), MAX_REG_SIZE
        ));
    }
    if rid != u64::MAX && !st.registers.contains_key(&rid) && st.registers.len() >= MAX_REGS {
        return Err(format!(
            "MemoryAccessViolation: register limit {} exceeded",
            MAX_REGS
        ));
    }
    st.registers.insert(rid, data);
    Ok(())
}
