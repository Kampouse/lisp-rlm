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

mod bls_validate;
mod bn254;
mod crypto_real;
mod ed25519;
mod gas;
mod hosts;
mod name_map;
mod promises;
mod schnorr;
mod state;

pub mod chain;
pub use chain::{CallBuilder, CallOutcome, ChainBuilder, MockChain};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasmtime::*;

pub(crate) use gas::{
    apply_staking_delta, locked_balance_for, splitmix64, trie_charge, trie_charge_write,
    GasSchedule, RunCfg,
};
pub(crate) use hosts::{build_env_linker, host_fn};

// ── mainnet gas/stack instrumentation (finite-wasm, PV155 costs) ──
pub(crate) mod instrument;
pub(crate) use instrument::REMAINING_GAS_EXPORT;

/// PV155 instruction costs (protocol-86 parameter snapshot, 2026-09-10).
pub(crate) const REGULAR_OP_COST: u64 = 822_756;
pub(crate) const LINEAR_OP_BASE_COST: u64 = 26_328_192;
pub(crate) const LINEAR_OP_UNIT_COST: u64 = 822_756;
/// max_stack_height (protocol-86): enforced by the instrumented stack budget.
pub(crate) const MAX_STACK_HEIGHT: u32 = 262_144;
/// Function-call action fee (execution side, protocol-86): burned by the
/// receipt itself on mainnet, on top of instruction + host gas.
pub(crate) const FUNCTION_CALL_BASE_GAS: u64 = 780_000_000_000;
pub(crate) const FUNCTION_CALL_BYTE_GAS: u64 = 2_235_934;
/// PV155 `base`: charged for EVERY host-function invocation.
pub(crate) const HOST_CALL_BASE_GAS: u64 = 264_768_111;
/// PV155 action-receipt creation fee (execution side) — every action receipt
/// burns this when applied, on top of per-action costs.
pub(crate) const ACTION_RECEIPT_CREATION_GAS: u64 = 108_059_500_000;
/// PV155 register/memory host costs (protocol-86 snapshot). The old
/// hardcoded numbers were ~100-275x low on bases — every receipt pays
/// input()/register costs, so this skewed ALL receipts slightly.
pub(crate) const WRITE_REGISTER_BASE_GAS: u64 = 2_865_522_486;
pub(crate) const WRITE_REGISTER_BYTE_GAS: u64 = 3_801_564;
pub(crate) const READ_MEMORY_BASE_GAS: u64 = 2_609_863_200;
pub(crate) const READ_MEMORY_BYTE_GAS: u64 = 3_801_333;
pub(crate) const WRITE_MEMORY_BASE_GAS: u64 = 2_803_794_861;
pub(crate) const WRITE_MEMORY_BYTE_GAS: u64 = 2_723_772;
pub(crate) const SHA256_BASE_GAS: u64 = 4_540_970_250;
pub(crate) const SHA256_BYTE_GAS: u64 = 24_117_351;
pub(crate) const KECCAK256_BASE_GAS: u64 = 5_879_491_275;
pub(crate) const KECCAK256_BYTE_GAS: u64 = 21_471_105;
pub(crate) const KECCAK512_BASE_GAS: u64 = 5_811_388_236;
pub(crate) const KECCAK512_BYTE_GAS: u64 = 36_649_701;
/// PV155 decoding + register costs (protocol-86).
pub(crate) const UTF8_DECODING_BASE_GAS: u64 = 3_111_779_061;
pub(crate) const UTF8_DECODING_BYTE_GAS: u64 = 291_580_479;
pub(crate) const UTF16_DECODING_BASE_GAS: u64 = 3_543_313_050;
pub(crate) const UTF16_DECODING_BYTE_GAS: u64 = 163_577_493;
pub(crate) const READ_REGISTER_BASE_GAS: u64 = 2_517_165_186;
/// PV155 promise host costs.
pub(crate) const PROMISE_AND_BASE_GAS: u64 = 1_465_013_400;
pub(crate) const PROMISE_AND_PER_GAS: u64 = 5_452_176;
pub(crate) const PROMISE_RETURN_GAS: u64 = 560_152_386;

/// Burn gas from an instance's instrumented remaining_gas global.
/// Returns the new remaining value.
pub(crate) fn burn_gas_global(
    store: &mut wasmtime::Store<StoreData>,
    instance: &wasmtime::Instance,
    amount: u64,
) -> u64 {
    let Some(g) = instance.get_global(&mut *store, REMAINING_GAS_EXPORT) else {
        return 0;
    };
    let cur = match g.get(&mut *store) {
        wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
        _ => return 0,
    };
    let next = cur.saturating_sub(amount);
    let _ = g.set(&mut *store, wasmtime::Val::I64(next as i64));
    next
}

/// Compile a contract mainnet-style: finite-wasm gas+stack instrumentation,
/// then wasmtime compilation. Every module loading path uses this.
pub(crate) fn compile_module(
    engine: &wasmtime::Engine,
    bytes: &[u8],
) -> Result<wasmtime::Module, Box<dyn std::error::Error>> {
    let prepared = instrument::instrument(bytes)?;
    Ok(wasmtime::Module::from_binary(engine, &prepared)?)
}
pub(crate) use promises::{
    dag_push, execute_promise, fail_receipts_any, fail_receipts_set, print_dag_map, sub_execute,
    PAction, PromiseBatch,
};
pub(crate) use state::{
    prefixed_key, restore_partition, snapshot_partition, state_file, write_reg_checked, MockState,
};

// Per-node memoized promise results (execute-once semantics, 2026-09-08).
// Keyed by receipt index; entries die with the run (TLS, cleared with the
// DAG). A node reachable through two parents (Burrow's swap receipt feeds
// both the resolve callback and the payout leg) used to EXECUTE TWICE —
// pass 2 saw pass 1's consumed state and trapped
// (`There is no action for the position`). Now the second visit replays the
// memoized result, exactly like a data receipt on-chain.
// (plain comment: rustdoc cannot attach docs to a macro invocation)
thread_local! {
    static PROMISE_OUTCOMES: std::cell::RefCell<std::collections::HashMap<usize, Vec<Option<Vec<u8>>>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Callback inputs visible through promise_results_count/promise_result —
/// the TLS mirror of the batch's dep_results (was a silent noop: real
/// near-sdk callbacks like Burrow's count-and-branch saw 0 results).
pub(crate) fn promise_results_tls() -> Vec<Option<Vec<u8>>> {
    PROMISE_RESULTS_CELL.with(|r| r.borrow().clone())
}

pub(crate) fn set_promise_results_tls(v: Vec<Option<Vec<u8>>>) {
    PROMISE_RESULTS_CELL.with(|r| *r.borrow_mut() = v);
}

thread_local! {
    static PROMISE_RESULTS_CELL: std::cell::RefCell<Vec<Option<Vec<u8>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Effective epoch height: NEAR_MOCK_EPOCH pin wins, else derives from the
/// clock — NEAR epochs are ~12h (43_200 s), so --now/--advance scenarios
/// advance epochs consistently with block_timestamp. Mirrors real NEAR where
/// epoch_height is a chain-level counter.
pub(crate) fn mock_epoch_height() -> u64 {
    if let Ok(s) = std::env::var("NEAR_MOCK_EPOCH") {
        if let Ok(v) = s.parse::<u64>() {
            return v;
        }
    }
    let c = mock_cfg();
    let base = c.base_ts.unwrap_or(0);
    if base == 0 && c.advance_secs == 0 {
        return 0; // unpinned, pre-genesis default (previous mock behavior)
    }
    ((base + c.advance_secs).max(0) as u64) / 43_200
}

/// Staking map for validator_stake/validator_total_stake hosts.
/// NEAR_MOCK_VALIDATORS='{"alice.pool.near": 1000000, ...}' (yoctoNEAR,
/// number or string) pins a custom set; unset → one deterministic validator
/// (`near-mock.pool.near`, 1M NEAR) so staking math never sees zeros.
pub(crate) fn validator_map() -> std::collections::BTreeMap<String, u128> {
    if let Ok(s) = std::env::var("NEAR_MOCK_VALIDATORS") {
        if let Ok(v) =
            serde_json::from_str::<std::collections::BTreeMap<String, serde_json::Value>>(&s)
        {
            return v
                .into_iter()
                .map(|(k, val)| {
                    let n = match val {
                        serde_json::Value::Number(x) => x.as_u64().unwrap_or(0) as u128,
                        serde_json::Value::String(x) => x.parse::<u128>().unwrap_or(0),
                        _ => 0,
                    };
                    (k, n)
                })
                .collect();
        }
    }
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "near-mock.pool.near".to_string(),
        1_000_000u128 * 10u128.pow(24),
    );
    m
}

// near-mock cross <state.bin> <acct=/path.wasm,...> <contract-acct> <method> [args-json]
//
// Multi-contract mode: manifest maps accounts to wasm files. Storage is
// per-account (prefixed keys in one state map). Promise DAGs execute
// synchronously after promise_return; sub-calls run in fresh Stores.
fn run_cross(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() < 5 {
        eprintln!(
            "Usage: near-mock cross <state.bin> <acct=wasm,...> <contract-acct> <method> [args-json] [--fail-receipt N] [--attach N] [--deposit N] [--view] [--json]\n       near-mock call   <state.bin> <acct=wasm,...> <contract> <method> [args] [--signer S] [--attach N] [--deposit N] [--view] [--json]\n       near-mock scenario <file.json>  (multi-step runner; steps support view/expect/fail_receipt)",
        );
        std::process::exit(1);
    }
    // Flags may appear anywhere after the `cross` keyword: --fail-receipt N
    // (repeatable), --signer/--attach/--deposit, --view, --json. Anything
    // else '-'-prefixed lands in `pos` and gets a loud warning — the silent
    // swallowing is how --deposit (0.1.7) and --json (0.1.8) broke here.
    let mut fail_receipts: Vec<usize> = Vec::new();
    let mut signer_flag: Option<String> = None;
    let mut attach_flag: Option<u128> = None;
    let mut run_view = false;
    let mut json_out = false;
    let mut pos: Vec<String> = Vec::new();
    {
        let mut it = args[2..].iter();
        while let Some(t) = it.next() {
            if t == "--fail-receipt" {
                let n = it
                    .next()
                    .and_then(|x| x.parse::<usize>().ok())
                    .ok_or("--fail-receipt requires a receipt index N (see the [map] printout)")?;
                fail_receipts.push(n);
            } else if t == "--signer" {
                signer_flag = Some(it.next().ok_or("--signer requires an account")?.clone());
            } else if t == "--attach" || t == "--deposit" {
                // --deposit is the spelling documented in --help FLAGS; it used
                // to fall through into `pos` here and be silently dropped, so
                // cross/call always saw attached_deposit() == 0.
                attach_flag = Some(
                    it.next()
                        .ok_or("--attach/--deposit requires decimal yocto")?
                        .parse()
                        .map_err(|_| "--attach/--deposit must be decimal yocto")?,
                );
            } else if t == "--view" {
                // consumed here (0.1.8): left in `pos` it could occupy the
                // args-json slot and silently swallow the real args
                run_view = true;
            } else if t == "--json" {
                json_out = true;
            } else if t == "--once" {
                // accepted no-op (single-wasm parity, script compat)
            } else {
                if t.starts_with('-') {
                    eprintln!("  ⚠ cross mode ignores unrecognized flag '{t}'");
                }
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
    let args_json = pos
        .get(4)
        .filter(|s| !s.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "{}".into());

    let engine = Rc::new(wasmtime::Engine::new(&base_engine_config())?);

    let state = init_sandbox(engine.clone(), manifest, state_path, run_view)?;
    // Pre-tx storage copy for the --json diff (execute_tx keeps its own
    // rollback snapshot internally; this one spans the whole CLI call,
    // attach credit included).
    let pre_state: HashMap<Vec<u8>, Vec<u8>> = state.lock().unwrap().storage.clone();
    let signer = signer_flag
        .or_else(|| std::env::var("NEAR_MOCK_SIGNER").ok())
        .unwrap_or_else(|| "caller.test.near".into());
    let attach: u128 = match attach_flag {
        Some(a) => a,
        None => std::env::var("NEAR_MOCK_ATTACH")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().parse())
            .transpose()
            .map_err(|_| "NEAR_MOCK_ATTACH must be decimal yocto")?
            .unwrap_or(0),
    };

    let outcome = execute_tx(
        &engine,
        &state,
        contract_acct,
        method,
        args_json.as_bytes(),
        &signer,
        attach,
        &fail_receipts,
        run_view,
    )?;
    print_outcome(&outcome);

    // Persist (library callers decide their own persistence; CLI writes the file).
    // Scoped: the --json block below takes its own lock.
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

    // --json (0.1.8): the same machine-readable contract as the single-wasm
    // runner (outcome/return/gas/storage/events), plus the cross-specific
    // receipt fields. Before 0.1.8 the flag fell into the positionals and
    // was silently ignored.
    if json_out {
        let hex_key = |k: &[u8]| -> String { k.iter().map(|b| format!("{b:02x}")).collect() };
        let st = state.lock().unwrap();
        let mut added: Vec<(String, usize)> = Vec::new();
        let mut changed: Vec<(String, usize, usize)> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for (k, v) in &st.storage {
            match pre_state.get(k) {
                None => added.push((hex_key(k), v.len())),
                Some(old) if old != v => changed.push((hex_key(k), old.len(), v.len())),
                _ => {}
            }
        }
        for k in pre_state.keys() {
            if !st.storage.contains_key(k) {
                removed.push(hex_key(k));
            }
        }
        let outcome_str = if outcome.ok {
            "ok"
        } else if outcome
            .error
            .as_deref()
            .map(|e| e.contains("all fuel consumed"))
            .unwrap_or(false)
        {
            "out_of_gas"
        } else {
            "trap"
        };
        let json_return = outcome.return_data.as_ref().map(|d| {
            serde_json::from_slice::<serde_json::Value>(d).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(d).into_owned())
            })
        });
        let prepaid = PREPAID_FUEL.with(|f| *f.borrow());
        let j = serde_json::json!({
            "outcome": outcome_str,
            "return": json_return,
            "error": outcome.error,
            "entry_trapped": outcome.entry_trapped,
            "receipts": outcome.receipt_results.len(),
            "orphan_failures": outcome.orphan_failures,
            "gas_burned_tgas": outcome.entry_gas_burned as f64 / 1e12,
            "gas_prepaid_tgas": prepaid as f64 / 1e12,
            "logs": LOG_COUNT.with(|l| *l.borrow()),
            "events": JSON_EVENTS.with(|e| e.borrow().clone()),
            "storage": {
                "keys_total": st.storage.len(),
                "added": added,
                "changed": changed,
                "removed": removed,
            },
            "host_trace": if mock_cfg().trace {
                Some(
                    host_trace_summary()
                        .1
                        .into_iter()
                        .map(|(n, c, e, g)| {
                            serde_json::json!({"host": n, "calls": c, "errors": e, "gas_tgas": g as f64 / 1e12})
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            },
        });
        println!("JSON {}", serde_json::to_string(&j).unwrap_or_default());
    }
    // CI contract: a failed tx exits nonzero (traps/out-of-gas/failed
    // receipts must never look green). Orphan receipt failures do NOT
    // flip the code: like real NEAR, they are visible in the outcome
    // but don't fail the transaction.
    if !outcome.ok {
        std::process::exit(1);
    }
    Ok(())
}

/// CLI skin: prints the outcome of a shared-core execution.
fn print_outcome(o: &TxOutcome) {
    if o.ok {
        println!("✅ Success");
    } else {
        println!("❌ {}", o.error.as_deref().unwrap_or("failed"));
        println!(
            "   ↺ {}rollback (single tx = atomic)",
            if o.entry_trapped {
                "entry trapped — full "
            } else {
                "full "
            }
        );
    }
    if let Some(d) = &o.return_data {
        let s = String::from_utf8_lossy(d);
        if !s.is_empty() {
            println!("📄 {s}");
        }
    }
}

/// Shared transaction-execution core behind `cross`/`call` (CLI) and
/// `MockChain::call` (library). Sets the exec context, credits any attached
/// deposit, runs the entry with a pre-call snapshot (NEAR tx atomicity),
/// resolves the promise DAG (root + fire-and-forget orphans, execute-once),
/// and rolls back atomically on entry trap or receipt-chain failure.
/// PERSISTENCE IS THE CALLER'S JOB — the core never touches the state file.
pub(crate) struct TxOutcome {
    /// Ok = entry (or final callback receipt) committed.
    pub ok: bool,
    /// Entry return-data when no promise was returned; otherwise the last
    /// receipt's result (NEAR tx semantics).
    pub return_data: Option<Vec<u8>>,
    /// Receipt results in completion order (None = that receipt failed).
    pub receipt_results: Vec<Option<Vec<u8>>>,
    /// True when the ENTRY trapped (vs a receipt-chain failure).
    pub entry_trapped: bool,
    /// Error text for the failure case (trap message / chain failure).
    pub error: Option<String>,
    /// Normalized panic message when the failure is a guest panic — exact
    /// nearcore ExecutionError format ("Smart contract panicked: <msg>"),
    /// extracted from the wasmtime error chain's `PANIC:` line. Lets library
    /// differs compare failure CLASSES against mainnet directly; `error`
    /// keeps the raw backtrace for debugging.
    pub panic: Option<String>,
    /// Normalized guest panics from PROMISE receipts of this tx, execution
    /// order ("Smart contract panicked: <msg>"). Empty when nothing trapped
    /// or the entry trapped before creating promises.
    pub receipt_failures: Vec<String>,
    /// Fire-and-forget receipts that failed (parent tx still commits).
    pub orphan_failures: usize,
    /// Gas burned by the ENTRY call (wasmtime fuel; PV155 1:1). Receipt gas
    /// burns in per-receipt stores and is not included here.
    pub entry_gas_burned: u64,
    /// Raw log lines emitted during this tx (byte-exact, no debug suffixes).
    pub logs: Vec<String>,
}

/// Fresh-tx hygiene (issue #1 L1): a previous call on this thread must not
/// leak its return_data, registers, or promise state into this one. Receipt
/// execution clears+restores around sub-calls; the entry call relied on
/// fresh processes (CLI) or per-step resets (scenario runner) — the
/// MockChain library path had neither, so call #2 saw call #1's data.
pub(crate) fn fresh_tx_state() {
    // Unit tests exercise the promise engine without an installed sandbox —
    // the state-scoped clears are simply skipped there.
    if let Some(state) = state_arc_tls() {
        let mut st = state.lock().unwrap();
        st.return_data = None;
        st.registers.clear();
    }
    PROMISE_DAG.with(|d| d.borrow_mut().clear());
    EXECUTED_PROMISES.with(|e| e.borrow_mut().clear());
    PROMISE_RESULTS.with(|r| *r.borrow_mut() = Vec::new());
    PENDING_RETURN.with(|p| *p.borrow_mut() = None);
    LOG_LINES.with(|l| l.borrow_mut().clear());
    crate::promises::receipt_traps_reset();
    // Execute-once memo is PER-DAG scope: indices restart at 0 every tx
    // (PROMISE_DAG cleared above), so an entry surviving from a previous tx
    // would alias THIS tx's receipt 0 — replaying stale results and skipping
    // the EXECUTED_PROMISES mark, which sent the orphan-drain loop into an
    // infinite spin (live-caught 2026-09-10: omft deposit flood, replayer
    // hung). Clearing restores the within-one-DAG memoization semantics.
    PROMISE_OUTCOMES.with(|o| o.borrow_mut().clear());
}

/// Store data for every execution: mainnet's instance/table/memory limits
/// (parameters snapshot, protocol 86: max 2048 memory pages = 128 MiB,
/// 1 table of ≤10k elements). Without a limiter the mock allowed memory to
/// grow to the full 4 GiB wasm ceiling where mainnet traps — found by the
/// wasmtime parity audit 2026-09-10.
pub(crate) type StoreData = wasmtime::StoreLimits;

pub(crate) const MAX_MEMORY_PAGES: u64 = 2048;

/// New store with mainnet resource limits installed.
pub(crate) fn new_store(engine: &wasmtime::Engine) -> wasmtime::Store<StoreData> {
    let limits = wasmtime::StoreLimitsBuilder::new()
        .instances(1)
        .memories(1)
        .memory_size((MAX_MEMORY_PAGES * 65536) as usize)
        .tables(1)
        .table_elements(10_000)
        .build();
    let mut store = wasmtime::Store::new(engine, limits);
    store.limiter(|l| l);
    store
}

/// Engine config shared by every entry point (library, CLI, cross, scenario).
/// Parity audit 2026-09-10 vs nearcore wasmtime_runner:
/// - NaN canonicalization ON — mainnet sets it (they ship a test asserting
///   canonical NaN payloads); wasmtime default is OFF, so float bit patterns
///   diverged silently.
/// - max_wasm_stack stays 64 MiB (mock's own headroom; mainnet uses 1 GiB
///   wasmtime headroom + a 262_144-frame instrumented limit we don't model).
pub(crate) fn base_engine_config() -> wasmtime::Config {
    let mut cfg = wasmtime::Config::new();
    // Fuel is OFF: gas is metered by finite-wasm instrumentation into the
    // remaining_gas global (mainnet mechanism), charged per-op with PV155
    // costs. Fuel previously double-counted with different weights.
    cfg.cranelift_nan_canonicalization(true);
    cfg.max_wasm_stack(64 * 1024 * 1024);
    cfg.async_stack_size(64 * 1024 * 1024);
    cfg
}

pub(crate) fn execute_tx(
    engine: &Rc<wasmtime::Engine>,
    state: &Arc<Mutex<MockState>>,
    contract_acct: &str,
    method: &str,
    args: &[u8],
    signer: &str,
    attach: u128,
    fail_receipts: &[usize],
    view: bool,
) -> Result<TxOutcome, Box<dyn std::error::Error>> {
    // Always set (not only when non-empty): a previous call on this thread
    // must not leak its forced-failure receipt indices into this one.
    fail_receipts_set(fail_receipts);
    fresh_tx_state();
    // The ENTRY receipt's deposit: attached_deposit() inside the contract
    // reads CURRENT_DEPOSIT (promise children get theirs from sub_execute).
    // Before 0.1.7 the cross/call path credited the balance but left the
    // host fn blind — the contract read 0 unless NEAR_MOCK_ATTACH happened
    // to be set (flag and env-var runs diverged on the same amount).
    CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = Some(attach));
    EXEC_CTX.with(|c| {
        *c.borrow_mut() = Some(ExecCtx {
            input: args.to_vec(),
            signer: signer.to_string(),
            predecessor: signer.to_string(),
            contract: contract_acct.to_string(),
            view,
        })
    });

    let module = MODULES
        .with(|m| m.borrow().as_ref().unwrap().get(contract_acct).cloned())
        .or_else(|| fork_get_module(engine, contract_acct))
        .ok_or(format!(
            "contract account {} not in manifest",
            contract_acct
        ))?;

    // Attached deposit (NEAR receipt semantics: value arrives before the
    // entry runs). The snapshot below includes it; a failed tx refunds.
    if attach > 0 {
        credit_attach(state, contract_acct, attach)?;
    }

    let mut store = new_store(&**engine);
    let prepaid = PREPAID_FUEL.with(|f| *f.borrow());
    let linker = build_env_linker(&mut store, &**engine, state.clone(), args.to_vec())?;
    let instance = linker.instantiate(&mut store, &module)?;

    // Gas budget lands in the instrumented global (fuel is disabled). Also
    // invoke the module's start function (exported, not a start section, per
    // nearcore prepare) BEFORE the entry — it's instrumented and charged.
    if let Some(g) = instance.get_global(&mut store, crate::REMAINING_GAS_EXPORT) {
        g.set(&mut store, wasmtime::Val::I64(prepaid as i64)).ok();
    }
    if let Some(start) = instance.get_func(&mut store, "start") {
        let _ = start.call(&mut store, &[], &mut []);
    }
    // Function-call action fee (execution side) — burned by the receipt on
    // mainnet before the contract runs; without it the differ's gas axis
    // would read ~0.8 Tgas systematically low.
    crate::burn_gas_global(
        &mut store,
        &instance,
        crate::ACTION_RECEIPT_CREATION_GAS
            + crate::FUNCTION_CALL_BASE_GAS
            + crate::FUNCTION_CALL_BYTE_GAS * args.len() as u64,
    );

    let func = instance
        .get_func(&mut store, method)
        .ok_or_else(|| format!("Method '{}' not found", method))?;

    // PRE-call copy for NEAR transaction atomicity: the snapshot must capture
    // state BEFORE the entry runs, or the Err-branch restore is a no-op (Ref
    // add_liquidity proved it 2026-09-06). Taken after the attach credit: the
    // deposit is part of the tx; the Err branch subtracts it back (refund).
    let mut tx_snapshot: HashMap<Vec<u8>, Vec<u8>> = state.lock().unwrap().storage.clone();
    let result = func.call(&mut store, &[], &mut []);
    let mut outcome = TxOutcome {
        ok: false,
        return_data: None,
        receipt_results: Vec::new(),
        entry_trapped: false,
        error: None,
        panic: None,
        receipt_failures: Vec::new(),
        orphan_failures: 0,
        entry_gas_burned: 0,
        logs: Vec::new(),
    };
    if result.is_err() {
        outcome.entry_trapped = true;
        // Full error chain, not just the top-level Display: wasmtime's
        // Display is the backtrace ("error while executing at wasm
        // backtrace: 0: …") and HIDES the root host error (e.g.
        // "BLS12381InvalidInput: …", "ProhibitedInView …") one+ levels
        // deep in the chain. Cross-mode consumers (print_outcome, --json,
        // the differ, test assertions on trap flavor) must see the cause.
        outcome.error = Some(error_chain(result.as_ref().err().unwrap()));
        outcome.panic = extract_panic(&outcome.error.clone().unwrap());
        // entry failed: snapshot WITHOUT the attach credit → full refund
        if attach > 0 {
            let key = prefixed_key(contract_acct, b"\x00near-bal");
            if let Some(v) = tx_snapshot.get(&key).cloned() {
                let bal: u128 = String::from_utf8_lossy(&v).trim().parse().unwrap_or(0);
                let pre_bal = bal.saturating_sub(attach);
                if pre_bal > 0 {
                    tx_snapshot.insert(key, pre_bal.to_string().into_bytes());
                } else {
                    tx_snapshot.remove(&key);
                }
            }
        }
    }

    match result {
        Ok(_) => {
            outcome.ok = true;
            // Entry return-data is authoritative ONLY when no promise was
            // returned; with a promise the callback's result is the tx result.
            let pending = PENDING_RETURN.with(|p| *p.borrow());
            if pending.is_none() {
                let st = state.lock().unwrap();
                if let Some(ref data) = st.return_data {
                    if !data.is_empty() {
                        outcome.return_data = Some(data.clone());
                    }
                }
            }
            // Deferred mode: commit the entry, queue the receipts, DON'T
            // execute — settle() delivers them later against current state.
            // The entry-level outcome is returned; the tx-final status
            // (final receipt) resolves at settle time.
            let deferred = DEFER_RECEIPTS.with(|d| d.replace(false));
            if deferred {
                let batches: Vec<PromiseBatch> =
                    PROMISE_DAG.with(|d| std::mem::take(&mut *d.borrow_mut()));
                RECEIPT_QUEUE.with(|q| {
                    q.borrow_mut().extend(batches);
                });
                PENDING_RETURN.with(|p| *p.borrow_mut() = None);
            }
            // Resolve the promise DAG returned by the entry.
            if !deferred && pending.is_some() {
                let idx = pending.unwrap();
                match execute_promise(idx) {
                    Err(e) => {
                        outcome.ok = false;
                        outcome.error = Some(format!("receipt chain failed: {e}"));
                        // full rollback (single tx = atomic)
                        let mut st = state.lock().unwrap();
                        st.storage = tx_snapshot;
                    }
                    Ok(results) => {
                        outcome.receipt_results = results.clone();
                        // NEAR tx semantics: the transaction's status follows
                        // the FINAL receipt of the returned chain. A trapped
                        // final callback ⇒ tx FAILED — but earlier receipts'
                        // state stays COMMITTED (no rollback: receipts are
                        // independent atomic units; 2026-09-10 fix — the old
                        // code reported ok=true for a failed final callback).
                        if results.last().is_some_and(|r| r.is_none()) {
                            outcome.ok = false;
                            let why = crate::promises::receipt_traps_peek_last()
                                .unwrap_or_else(|| "final receipt failed".into());
                            outcome.error =
                                Some(format!("receipt chain failed: final receipt: {why}"));
                            outcome.panic = Some(why);
                        }
                        if let Some(bytes) = results.iter().rev().find_map(|r| r.as_ref().cloned())
                        {
                            if !bytes.is_empty() {
                                outcome.return_data = Some(bytes);
                            }
                        }
                    }
                }
            }
            // Fire-and-forget receipts (2026-09-02): batches created but not
            // part of any returned DAG still execute as independent receipts;
            // their failures do NOT roll back the parent tx. (Skipped in
            // deferred mode — everything is queued.)
            while !deferred {
                let next = PROMISE_DAG.with(|d| {
                    d.borrow()
                        .iter()
                        .enumerate()
                        .find(|(i, _)| !EXECUTED_PROMISES.with(|e| e.borrow().contains(i)))
                        .map(|(i, _)| i)
                });
                let Some(idx) = next else { break };
                match execute_promise(idx) {
                    Ok(_) => {}
                    Err(_) => outcome.orphan_failures += 1,
                }
            }
        }
        Err(_) => {
            // entry trapped — full rollback (single tx = atomic)
            let mut st = state.lock().unwrap();
            st.storage = tx_snapshot;
        }
    }
    outcome.entry_gas_burned = {
        // burned = prepaid - remaining, from the instrumented gas global
        let remaining = instance
            .get_global(&mut store, crate::REMAINING_GAS_EXPORT)
            .map(|g| match g.get(&mut store) {
                wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
                _ => 0,
            })
            .unwrap_or(0);
        prepaid.saturating_sub(remaining)
    };
    // Receipt-chain failures carry nested guest panics too — normalize once
    // more so every !ok outcome has its class extracted if present.
    if !outcome.ok && outcome.panic.is_none() {
        if let Some(e) = outcome.error.as_deref() {
            outcome.panic = extract_panic(e);
        }
    }
    outcome.logs = LOG_LINES.with(|l| std::mem::take(&mut *l.borrow_mut()));
    outcome.receipt_failures = crate::promises::receipt_traps_drain();
    // Receipt hygiene: don't leak the entry's deposit into the next tx on
    // this thread (the library MockChain path runs many calls in-process).
    CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = None);
    Ok(outcome)
}

/// Pull the guest panic message out of a wasmtime error blob. The host's
/// `panic_utf8` error ("PANIC: <msg>") rides the error chain below the
/// backtrace; the message is the first line after the marker.
fn extract_panic(raw: &str) -> Option<String> {
    // Gas exhaustion: the instrumented hook errors with mainnet's receipt-
    // level message — surface it as the failure CLASS directly (mainnet
    // outcomes show ExecutionError "Exceeded the prepaid gas").
    if raw.contains("Exceeded the prepaid gas") {
        return Some("Exceeded the prepaid gas".to_string());
    }
    if raw.contains("WasmTrap: StackOverflow") {
        return Some("WasmTrap: StackOverflow".to_string());
    }
    // Bare traps (release builds with stripped messages, panic_immediate_abort,
    // custom wasm): classify by wasmtime trap text into nearcore's WasmTrap
    // taxonomy so failure CLASSES are comparable even without messages.
    for (needle, class) in [
        ("out of bounds memory access", "WasmTrap: MemoryOutOfBounds"),
        ("integer divide by zero", "WasmTrap: IntegerDivisionByZero"),
        ("integer overflow", "WasmTrap: IntegerOverflow"),
        ("indirect call to null", "WasmTrap: IndirectCallToNull"),
        (
            "signature mismatch",
            "WasmTrap: IncorrectCallIndirectSignature",
        ),
        ("unreachable", "WasmTrap: Unreachable"),
        ("call stack exhausted", "WasmTrap: StackOverflow"),
    ] {
        if raw.contains(needle) {
            return Some(class.to_string());
        }
    }
    let i = raw.find("PANIC: ")?;
    let rest = &raw[i + "PANIC: ".len()..];
    let line = rest.lines().next().unwrap_or("").trim_end();
    if line.is_empty() {
        return None;
    }
    Some(format!("Smart contract panicked: {line}"))
}

/// Full error text INCLUDING the source chain — wasmtime::Error's Display
/// shows only the outer message (backtrace header); root host errors
/// ("PANIC: ...", ProhibitedInView, ...) live in the chain links the CLI
/// walks via `e.chain().skip(1)`. wasmtime::Error doesn't impl std Error.
fn error_chain(e: &wasmtime::Error) -> String {
    let mut s = e.to_string();
    for c in e.chain().skip(1) {
        s.push('\n');
        s.push_str(&c.to_string());
    }
    s
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
            debug: std::env::var("NEAR_MOCK_DEBUG")
                .map(|v| v == "1")
                .unwrap_or(false),
            base_ts: std::env::var("NEAR_MOCK_NOW")
                .ok()
                .and_then(|s| s.parse().ok()),
            advance_secs: 0,
            trace: std::env::var("NEAR_MOCK_TRACE")
                .map(|v| v == "1")
                .unwrap_or(false),
            fork: None,
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

/// Per-host aggregation: (total_gas, [(host, calls, errors, gas)]) sorted
/// by gas desc. Returns zeros when tracing is off; drains the buffer.
/// The error count surfaces failed host calls (ProhibitedInView refusals,
/// host traps) in the summary — without it they were only visible by
/// scrolling the live timeline.
pub(crate) fn host_trace_summary() -> (u64, Vec<(String, u64, u64, u64)>) {
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
    let mut agg: HashMap<String, (u64, u64, u64)> = HashMap::new();
    for e in &entries {
        let slot = agg.entry(e.name.clone()).or_insert((0, 0, 0));
        slot.0 += 1;
        if e.err {
            slot.1 += 1;
        }
        slot.2 += e.gas;
    }
    let mut rows: Vec<(String, u64, u64, u64)> =
        agg.into_iter().map(|(k, (c, e, g))| (k, c, e, g)).collect();
    rows.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
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
        let err_note = if r.2 > 0 {
            format!("  ❌{}err", r.2)
        } else {
            String::new()
        };
        println!(
            "  {:>12.3}G  {:>4}×  {}{}",
            r.3 as f64 / 1e9,
            r.1,
            r.0,
            err_note
        );
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
    /// Raw log lines of the CURRENT tx (cleared per execute_tx) — byte-exact,
    /// no debug suffixes. Surfaces as TxOutcome.logs so library callers
    /// (replayers, differs) don't scrape stdout. CLI printing unaffected.
    static LOG_LINES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// NEAR_MOCK_QUIET=1: suppress per-log println + hosts.rs per-call stderr
/// traces (long-running replayers); capture into LOG_LINES still happens.
/// Cached — checked per log line / host call.
fn log_capture_quiet() -> bool {
    static Q: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *Q.get_or_init(|| {
        std::env::var("NEAR_MOCK_QUIET")
            .map(|v| v == "1")
            .unwrap_or(false)
    })
}

/// Shared gate for hosts.rs `htrace!` macro (same env var).
pub(crate) fn host_trace_quiet() -> bool {
    log_capture_quiet()
}

/// Route one decoded log line: NEP-297 EVENT_JSON: gets structured decoding,
/// everything else prints as LOG. Never panics on weird payloads.
fn handle_log_line(msg: &str, debug: bool, suffix: &str) {
    LOG_COUNT.with(|c| *c.borrow_mut() += 1);
    LOG_LINES.with(|l| l.borrow_mut().push(msg.to_string()));
    if log_capture_quiet() {
        return; // still captured above; just don't print
    }
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

/// Gated stderr trace (mod.rs library paths: promise hosts, attach credits,
/// DAG resolution). NEAR_MOCK_QUIET=1 silences per-receipt firehose; the CLI
/// default (no env) is unchanged.
macro_rules! mtrace {
    ($($arg:tt)*) => {
        if !log_capture_quiet() { eprintln!($($arg)*); }
    };
}

/// Run a pretty-printing section with panic containment: a reporting bug must
/// never eat a successful run (the 2026-09-05 storage-dump char-boundary
/// panic turned ✅ contract successes into exit 101).
fn safe_report<F: FnOnce()>(label: &str, f: F) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    if r.is_err() {
        mtrace!("⚠ {label}: reporting section panicked (contract result unaffected)");
    }
}

// (docs for sub_execute moved to its definition in promises.rs — a doc
// comment here would attach to the thread_local! macro invocation, which
// rustdoc cannot annotate)
thread_local! {
    /// The CURRENT receipt's attached deposit. Set by sub_execute for
    /// batch function-call children (was: silently dropped — dep_ptr was
    /// read nowhere; 2026-09-01). Top-level entries fall back to
    /// NEAR_MOCK_ATTACH via the host fn.
    static CURRENT_DEPOSIT: std::cell::RefCell<Option<u128>> = const { std::cell::RefCell::new(None) };
}

// ── real promise hosts (cross engine) ──
/// Strict promise-host read: OOB (None) traps like nearcore's
/// MemoryAccessViolation — the silent `.unwrap_or_default()` path let
/// corrupted tagged-string pointers (e.g. lisp-rlm double-eval heap drift)
/// produce empty-string receipts in the mock while the chain dropped or
/// trapped them, making compiler bugs invisible locally (2026-09-11).
fn mem_read_str_checked(
    caller: &mut wasmtime::Caller<'_, StoreData>,
    len: i64,
    ptr: i64,
    host: &str,
) -> Result<String, wasmtime::Error> {
    mem_read_str(caller, len, ptr).ok_or_else(|| {
        wasmtime::Error::msg(format!(
            "MemoryAccessViolation: {host} read (len={len} ptr={ptr}) out of bounds — nearcore traps here"
        ))
    })
}

fn mem_read_str(
    caller: &mut wasmtime::Caller<'_, StoreData>,
    len: i64,
    ptr: i64,
) -> Option<String> {
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
fn read_guest_bytes(
    caller: &mut wasmtime::Caller<'_, StoreData>,
    len: i64,
    ptr: i64,
) -> Option<Vec<u8>> {
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
    store: &mut wasmtime::Store<StoreData>,
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
    let pbc = host_fn(
        "promise_batch_create",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let acct_len = args[0].unwrap_i64() as u64 as u64;
            crate::hosts::charge_gas(
                &mut caller,
                crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * acct_len,
            )?;
            let acct = mem_read_str_checked(
                &mut caller,
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                "promise-host",
            )?;
            mtrace!("  → promise_batch_create({}) [dag]", acct);
            results[0] = Val::I64(dag_push(vec![], acct, vec![]) as i64);
            Ok(())
        },
    );
    // 40 promise_batch_then(idx, acct_len, acct_ptr) -> new idx
    let pbt = host_fn(
        "promise_batch_then",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
        move |mut caller, args, results| {
            let idx = args[0].unwrap_i64() as usize;
            let acct = mem_read_str_checked(
                &mut caller,
                args[1].unwrap_i64(),
                args[2].unwrap_i64(),
                "promise-host",
            )?;
            results[0] = Val::I64(dag_push(vec![idx], acct, vec![]) as i64);
            Ok(())
        },
    );
    // 43 promise_batch_action_function_call(idx, m_len, m_ptr, a_len, a_ptr, dep_ptr, gas)
    let pafc = host_fn(
        "promise_batch_action_function_call",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![]),
        move |mut caller, args, _| {
            let idx = args[0].unwrap_i64() as usize;
            let method = mem_read_str_checked(
                &mut caller,
                args[1].unwrap_i64(),
                args[2].unwrap_i64(),
                "promise-host",
            )?;
            let args_json = mem_read_str_checked(
                &mut caller,
                args[3].unwrap_i64(),
                args[4].unwrap_i64(),
                "promise-host",
            )?;
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
            mtrace!(
                "  → action_fn_call(idx={}, {} args={} dep={})",
                idx,
                method,
                args_json,
                dep
            );
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::FnCall {
                        method,
                        args: args_json.into_bytes(),
                        gas,
                        dep,
                    });
                }
            });
            Ok(())
        },
    );
    // 44 promise_batch_action_transfer(idx, amt_ptr) — u128 LE at ptr (16 bytes)
    let pbat = host_fn(
        "promise_batch_action_transfer",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |caller, args, _| {
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
    let pyc = host_fn(
        "promise_yield_create",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![ValType::I64]),
        move |mut caller, args, results| {
            let method = mem_read_str_checked(
                &mut caller,
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                "promise-host",
            )?;
            let args_json = mem_read_str_checked(
                &mut caller,
                args[2].unwrap_i64(),
                args[3].unwrap_i64(),
                "promise-host",
            )?;
            let reg = args[6].unwrap_i64() as u64;
            let contract = exec_ctx_or_default().contract;
            mtrace!(
                "  → promise_yield_create({} args={}) on {}",
                method,
                args_json,
                contract
            );
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
                let spec = format!(
                    "{}\x1f{}\x1f{}\x1f{}",
                    contract, method, batch_creator, args_json
                );
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
    let pyr = host_fn(
        "promise_yield_resume",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![ValType::I64]),
        move |mut caller, args, results| {
            // ABI: (data_id_len, data_id_ptr, payload_len, payload_ptr) — the
            // emitter passes the data_id as a STRING ("yd:<idx>" or "<idx>")
            let data_id = mem_read_str_checked(
                &mut caller,
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                "promise-host",
            )?;
            let payload = mem_read_str_checked(
                &mut caller,
                args[2].unwrap_i64(),
                args[3].unwrap_i64(),
                "promise-host",
            )?;
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
                                mtrace!("  ⚠ yield_resume: bad persisted spec at {}", idx);
                                results[0] = Val::I64(0);
                                return Ok(());
                            }
                        }
                        None => {
                            mtrace!("  ⚠ yield_resume: idx {} is not a yield promise", idx);
                            results[0] = Val::I64(0);
                            return Ok(());
                        }
                    }
                }
            };
            mtrace!("  ⏵ yield_resume({}) payload={}", idx, payload);
            let (method, args_json, _) = match batch.actions.first() {
                Some(PAction::FnCall {
                    method, args, gas, ..
                }) => (method.clone(), args.clone(), gas),
                _ => {
                    mtrace!("  ⚠ yield_resume: no callback action on idx {}", idx);
                    results[0] = Val::I64(0);
                    return Ok(());
                }
            };
            // Re-run the callback with the payload as the Successful result
            let saved = PROMISE_RESULTS.with(|r| {
                std::mem::replace(&mut *r.borrow_mut(), vec![Some(payload.into_bytes())])
            });
            let ret = sub_execute(&batch.account, &method, &args_json, &batch.creator, 0);
            PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
            // one-shot: consume the persisted yield handle
            {
                let st = STATE_ARC.with(|s| s.borrow().clone());
                if let Some(st) = st {
                    st.lock()
                        .unwrap()
                        .storage
                        .remove(format!("\x00yield:{}", idx).as_bytes());
                }
            }
            match ret {
                Ok(Some(bytes)) => {
                    let s = String::from_utf8_lossy(&bytes);
                    if !s.is_empty() {
                        println!("📄 (yield) {}", s);
                    }
                }
                Ok(None) => mtrace!("  ⚠ yield callback trapped"),
                Err(e) => mtrace!("  ⚠ yield callback error: {}", e),
            }
            results[0] = Val::I64(1);
            Ok(())
        },
    );
    // 30 promise_create
    let pc = host_fn(
        "promise_create",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 8], vec![ValType::I64]),
        move |mut caller, args, results| {
            let acct = mem_read_str_checked(
                &mut caller,
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                "promise-host",
            )?;
            let method = mem_read_str_checked(
                &mut caller,
                args[2].unwrap_i64(),
                args[3].unwrap_i64(),
                "promise-host",
            )?;
            let args_json = mem_read_str_checked(
                &mut caller,
                args[4].unwrap_i64(),
                args[5].unwrap_i64(),
                "promise-host",
            )?;
            let idx = dag_push(
                vec![],
                acct,
                vec![PAction::FnCall {
                    method,
                    args: args_json.into_bytes(),
                    gas: args[7].unwrap_i64() as u64,
                    dep: 0,
                }],
            );
            results[0] = Val::I64(idx as i64);
            Ok(())
        },
    );
    // 31 promise_then
    let pt = host_fn(
        "promise_then",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![ValType::I64]),
        move |mut caller, args, results| {
            let idx = args[0].unwrap_i64() as usize;
            let acct = mem_read_str_checked(
                &mut caller,
                args[1].unwrap_i64(),
                args[2].unwrap_i64(),
                "promise-host",
            )?;
            let method = mem_read_str_checked(
                &mut caller,
                args[3].unwrap_i64(),
                args[4].unwrap_i64(),
                "promise-host",
            )?;
            let args_json = mem_read_str_checked(
                &mut caller,
                args[5].unwrap_i64(),
                args[6].unwrap_i64(),
                "promise-host",
            )?;
            let new_idx = dag_push(
                vec![idx],
                acct,
                vec![PAction::FnCall {
                    method,
                    args: args_json.into_bytes(),
                    gas: args[8].unwrap_i64() as u64,
                    dep: 0,
                }],
            );
            results[0] = Val::I64(new_idx as i64);
            Ok(())
        },
    );
    // 32 promise_and(ptr, count) -> idx
    let pa = host_fn(
        "promise_and",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let count = args[1].unwrap_i64() as usize;
            crate::hosts::charge_gas(
                &mut caller,
                crate::PROMISE_AND_BASE_GAS + crate::PROMISE_AND_PER_GAS * count as u64,
            )?;
            let ptr = args[0].unwrap_i64() as usize;
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
    let prc = host_fn(
        "promise_results_count",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, results| {
            results[0] = Val::I64(PROMISE_RESULTS.with(|r| r.borrow().len() as i64));
            Ok(())
        },
    );
    // 34 promise_result(idx, reg) -> status
    let state_for_pr = STATE_ARC.with(|s| s.borrow().clone());
    let pr = host_fn(
        "promise_result",
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
                        // 2026-09-07 (intents ft_resolve_withdraw): a Successful
                        // result with EMPTY data must leave register_len == 0 —
                        // near-sdk's promise_result_checked skips the read then.
                        // The shared register table used to leak the parent's
                        // stale bytes (e.g. the 32B intent hash), which guests
                        // deserialized as garbage and treated as failure.
                        if bytes.is_empty() {
                            // empty-but-present: register_len == 0, read = empty
                            st.registers.insert(rid, Vec::new());
                        } else {
                            let _ = write_reg_checked(&mut st, rid, bytes);
                        }
                    }
                    results[0] = Val::I64(1);
                }
                // NEAR ABI (nearcore VmResultPromiseResult): 0 = Failed,
                // 1 = Successful, 2 = NotReady. A failed receipt (Some(None))
                // is FAILED = 0 — the mock returned 2 here, which near-sdk
                // parses as NotReady and recovery handlers never fired
                // (caught by the receipt-semantics regression 2026-09-10).
                Some(None) => results[0] = Val::I64(0),
            }
            Ok(())
        },
    );
    // 35 promise_return(idx)
    let pret = host_fn(
        "promise_return",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        |mut caller, args, _| {
            crate::hosts::charge_gas(&mut caller, crate::PROMISE_RETURN_GAS)?;
            PENDING_RETURN.with(|p| *p.borrow_mut() = Some(args[0].unwrap_i64() as usize));
            mtrace!("  → promise_return({})", args[0].unwrap_i64());
            Ok(())
        },
    );
    Ok((pc, pt, pa, prc, pr, pret, pbc, pbt, pafc, pbat, pyc, pyr))
}

fn exec_ctx_view(state: &std::sync::Arc<Mutex<MockState>>) -> bool {
    EXEC_CTX
        .with(|c| c.borrow().as_ref().map(|x| x.view))
        .unwrap_or_else(|| state.lock().unwrap().view)
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
        let bytes = std::fs::read(path).map_err(|e| {
            format!(
                "cannot read contract `{path}`: {e} (scenario default expects contract.wasm beside the scenario file; override with \"manifest\")"
            )
        })?;
        eprintln!("📦 {} → {}", acct, path);
        modules.insert(acct.to_string(), compile_module(&engine, &bytes)?);
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
    install_sandbox(engine, modules, loaded_storage, view)
}

/// TLS install shared by the CLI (`init_sandbox`) and the library
/// (`chain::MockChain`). One chain per thread — the engine's promise DAG and
/// module table are thread-locals, matching NEAR receipts being per-shard.
pub(crate) fn install_sandbox(
    engine: Rc<wasmtime::Engine>,
    modules: HashMap<String, wasmtime::Module>,
    storage: HashMap<Vec<u8>, Vec<u8>>,
    view: bool,
) -> Result<Arc<Mutex<MockState>>, Box<dyn std::error::Error>> {
    let state: Arc<Mutex<MockState>> = Arc::new(Mutex::new(MockState {
        storage,
        touched: Default::default(),
        registers: HashMap::new(),
        return_data: None,
        view,
    }));
    // Genesis protocol state: seed the validator map ONCE at chain install if
    // absent (NEAR_MOCK_VALIDATORS JSON or the mock pool default). Validators
    // exist before any tx on a real chain — never written mid-execution, so
    // trap rollbacks can't resurrect them (see hosts.rs validator comment).
    if !state
        .lock()
        .unwrap()
        .storage
        .contains_key(b"\x00validators".as_slice())
    {
        let vals: std::collections::BTreeMap<String, String> = validator_map()
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        let json = serde_json::to_string(&vals).unwrap_or_else(|_| "{}".into());
        state
            .lock()
            .unwrap()
            .storage
            .insert(b"\x00validators".to_vec(), json.into_bytes());
    }
    MODULES.with(|m| *m.borrow_mut() = Some(Arc::new(modules)));
    STATE_ARC.with(|s| *s.borrow_mut() = Some(state.clone()));
    ENGINE_TLS.with(|e| *e.borrow_mut() = Some(engine.clone()));
    Ok(state)
}

// ── Library-API helpers (used by chain.rs; TLS is the engine's home) ──

/// The installed shared state, if any (`MockChain`).
pub(crate) fn state_arc_tls() -> Option<Arc<Mutex<MockState>>> {
    STATE_ARC.with(|s| s.borrow().clone())
}

/// The installed engine, if any (`MockChain`).
pub(crate) fn engine_tls() -> Option<std::rc::Rc<wasmtime::Engine>> {
    ENGINE_TLS.with(|e| e.borrow().clone())
}

/// Read the sandbox view flag (`MockChain::view`).
pub(crate) fn mock_state_view_get(state: &Arc<Mutex<MockState>>) -> bool {
    state.lock().unwrap().view
}

/// Write the sandbox view flag (`MockChain::view`).
pub(crate) fn mock_state_view_set(state: &Arc<Mutex<MockState>>, v: bool) {
    state.lock().unwrap().view = v;
}

/// Pin the deterministic clock base (unix seconds) — library parity of
/// `--now` / `NEAR_MOCK_NOW`.
pub(crate) fn set_time_base(unix_secs: i64) {
    RUN_CFG.with(|c| {
        let mut cfg = c.borrow_mut();
        let cfg = cfg.get_or_insert_with(RunCfg::default);
        cfg.base_ts = Some(unix_secs);
    });
}

/// Advance the deterministic clock by `secs` — library parity of `--advance`.
pub(crate) fn advance_time(secs: i64) {
    RUN_CFG.with(|c| {
        let mut cfg = c.borrow_mut();
        let cfg = cfg.get_or_insert_with(RunCfg::default);
        cfg.advance_secs += secs;
    });
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
    mtrace!(
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

    let mut fuel_cfg = base_engine_config();
    // epoch_interruption MUST be on or set_epoch_deadline is inert —
    // no epoch checks get compiled into wasm, so spin loops run forever.
    fuel_cfg.epoch_interruption(true);
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
        let method: Option<String> = step
            .get("method")
            .and_then(|m| m.as_str())
            .map(String::from);
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
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        let step_advance = step.get("advance").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        // gas budget: TGas cap on this step's entry call (out-of-gas ≠ trap)
        // accept number OR string ("2") — a stringy gas silently un-capping
        // the step would turn a chaos test into a 20s-per-spin hang
        let gas_cap_tgas = step.get("gas").and_then(|g| {
            g.as_u64()
                .or_else(|| g.as_str().and_then(|s| s.parse().ok()))
        });
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
            Some(_) => return Err(format!("step {}: attach must be a string or number", i).into()),
        };
        let snap_name = step
            .get("snapshot")
            .and_then(|s| s.as_str())
            .map(String::from);
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
            credit_attach(&state, &contract, amt).map_err(|e| format!("step {}: {}", i, e))?;
            CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = Some(amt));
        } else {
            CURRENT_DEPOSIT.with(|d| *d.borrow_mut() = None);
        }

        println!(
            "\n── step {} ▶ {}.{}{}{} ──",
            i,
            contract,
            method,
            if args_json == "{}" {
                "".into()
            } else {
                format!(" {}", args_json)
            },
            if is_view { " (view)" } else { "" }
        );
        if step_signer != "owner.test.near" || step_pred != step_signer {
            println!("  👤 as {} (predecessor {})", step_signer, step_pred);
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
        let mut store = new_store(&*engine);
        // Epoch deadline: ~t ms for gas-capped steps, 20 s wall bound
        // otherwise (ticker ticks every 1 ms; epochs only advance when a
        // gas-capped scenario spawned the ticker — otherwise inert).
        store.set_epoch_deadline(gas_cap_tgas.unwrap_or(20_000).max(1));
        let linker = build_env_linker(&mut store, &*engine, state.clone(), args_json.into_bytes())?;
        let instance = linker.instantiate(&mut store, &module)?;
        if let Some(g) = instance.get_global(&mut store, crate::REMAINING_GAS_EXPORT) {
            g.set(&mut store, wasmtime::Val::I64(prepaid as i64)).ok();
        }
        if let Some(start) = instance.get_func(&mut store, "start") {
            let _ = start.call(&mut store, &[], &mut []);
        }
        let result = instance
            .get_func(&mut store, &method)
            .ok_or(format!(
                "step {}: method '{}' not found on {}",
                i, method, contract
            ))?
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
                    mtrace!("  ⛓ resolving promise DAG (root {})", idx);
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
                    println!(
                        "✗ expect '{}' — got {:?} storage {:?}",
                        want, expect_out, stored
                    );
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
    println!(
        "\n🎬 scenario {}: {} pass / {} fail — state → {}",
        name, pass, fail, state_path
    );
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
// One JSON-RPC `query` via curl (no new deps; curl is guaranteed on macOS).
// Err = the raw JSON-RPC error object, so callers can branch on error names
// (e.g. TOO_LARGE_CONTRACT_STATE) and pull structured info (block hints).

// ═══════════════════════════════════════════════════════════════════
// Fork-mode: lazy code + state paging from an archival RPC ("anvil
// --fork-url" for NEAR). Storage reads that miss locally page the key
// in from the pinned block; contract code is fetched on first call.
// Writes/deletes land locally; tombstones keep deletions from
// resurrecting on later reads.
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// Deferred receipts: fire_deferred() commits the entry and leaves the
// promise DAG as a QUEUE (chain model: receipts are delivered in later
// blocks). settle() delivers them in causal order, each atomically.
// Between the two, the "stuck" intermediate state is observable — the
// incident-forensics primitive (round stuck in Rolling, MPC silent).
// ═══════════════════════════════════════════════════════════════════

thread_local! {
    /// Receipts queued by fire_deferred(), awaiting settle(). Survives
    /// across fire() calls (chain: receipts execute against CURRENT state).
    static RECEIPT_QUEUE: std::cell::RefCell<Vec<PromiseBatch>> =
        std::cell::RefCell::new(Vec::new());
    /// One-shot: fire_deferred() sets it; execute_tx consumes it.
    static DEFER_RECEIPTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn set_defer_receipts(v: bool) {
    DEFER_RECEIPTS.with(|d| d.set(v));
}

pub(crate) fn pending_receipt_count() -> usize {
    RECEIPT_QUEUE.with(|q| q.borrow().len())
}

/// Deliver queued receipts (from fire_deferred) in causal order, each
/// atomically (partition rollback on trap; Failed flows to dependents).
/// Mirrors on-chain delivery: receipts execute against CURRENT state,
/// possibly after further transactions have fired.
pub(crate) fn settle_receipts() -> Result<SettleReport, Box<dyn std::error::Error>> {
    let mut report = SettleReport::default();
    let queue: Vec<PromiseBatch> = RECEIPT_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if queue.is_empty() {
        return Ok(report);
    }

    // fresh execution context for the delivery round (memo/dag/marks),
    // then re-insert the batches with remapped dep indices.
    PROMISE_DAG.with(|d| d.borrow_mut().clear());
    EXECUTED_PROMISES.with(|e| e.borrow_mut().clear());
    PROMISE_OUTCOMES.with(|o| o.borrow_mut().clear());
    crate::promises::receipt_traps_reset();
    LOG_LINES.with(|l| l.borrow_mut().clear());

    let mut remap: Vec<usize> = Vec::with_capacity(queue.len());
    PROMISE_DAG.with(|d| {
        let mut dag = d.borrow_mut();
        for batch in queue {
            let new_idx = dag.len();
            let batch = PromiseBatch {
                deps: batch.deps.iter().map(|dep| remap[*dep]).collect(),
                ..batch
            };
            remap.push(new_idx); // queue order preserved → old idx ↦ new idx
            dag.push(batch);
        }
    });

    // deliver in causal order: a batch is ready when all deps executed
    loop {
        let next = PROMISE_DAG.with(|d| {
            d.borrow()
                .iter()
                .enumerate()
                .find(|(i, b)| {
                    !EXECUTED_PROMISES.with(|e| e.borrow().contains(i))
                        && b.deps
                            .iter()
                            .all(|dep| EXECUTED_PROMISES.with(|e| e.borrow().contains(dep)))
                })
                .map(|(i, _)| i)
        });
        let Some(idx) = next else { break };
        match execute_promise(idx) {
            Ok(results) => {
                report.delivered += 1;
                report.receipt_results.extend(results);
            }
            Err(_) => {
                // infra-level failure of one receipt: count and continue
                // (chain: a broken receipt doesn't stop other deliveries)
                report.delivered += 1;
                report.receipt_results.push(None);
            }
        }
    }

    report.failures = crate::promises::receipt_traps_drain();
    report.logs = LOG_LINES.with(|l| std::mem::take(&mut *l.borrow_mut()));
    Ok(report)
}

/// Result of settle(): receipts delivered in causal order.
#[derive(Debug, Default)]
pub struct SettleReport {
    /// Batches delivered.
    pub delivered: usize,
    /// Per-batch results in delivery order (None = that receipt failed).
    pub receipt_results: Vec<Option<Vec<u8>>>,
    /// Failure reasons (panics, AccountDoesNotExist) across deliveries.
    pub failures: Vec<String>,
    /// Logs emitted by settled receipts.
    pub logs: Vec<String>,
}

thread_local! {
    /// Prefixed keys known absent on the fork (fetched-miss or deleted
    /// locally). Survives across txs within a session — a delete must not
    /// resurrect via re-fetch. Not persisted with state files (session-scoped).
    static FORK_ABSENT: std::cell::RefCell<std::collections::HashSet<Vec<u8>>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// Modules fetched+compiled at runtime (MODULES is Arc-frozen at install).
    static FORK_MODULES: std::cell::RefCell<HashMap<String, wasmtime::Module>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Plain POST JSON (FastNear tx API etc. — not a JSON-RPC envelope).
fn http_post_json(url: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &body.to_string(),
            url,
        ])
        .output()
        .map_err(|e| format!("curl spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exit {}", out.status));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad JSON: {e}"))
}

/// Snapshot of this tx's host-call trace (for --json consumers).
pub(crate) fn host_trace_entries() -> Vec<(String, u64, bool)> {
    match HOST_TRACE.lock() {
        Ok(g) => g
            .as_ref()
            .map(|v| v.iter().map(|e| (e.name.clone(), e.gas, e.err)).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Install fork config into RUN_CFG (library builder + CLI both use this).
pub(crate) fn set_fork_cfg(rpc: String, block: Option<u64>) {
    RUN_CFG.with(|c| {
        let mut slot = c.borrow_mut();
        let mut cfg = slot.take().unwrap_or_default();
        cfg.fork = Some(crate::gas::ForkCfg { rpc, block });
        *slot = Some(cfg);
    });
}

/// Latest block height from `status` (used when no --fork-block is given).
pub(crate) fn fork_latest_block(rpc: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": "dontcare", "method": "status",
        "params": [None::<String>]});
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(2u64 << (attempt - 1)));
        }
        let out = std::process::Command::new("curl")
            .args([
                "-s",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body.to_string(),
                rpc,
            ])
            .output()
            .map_err(|e| format!("curl spawn: {e}"))?;
        if !out.status.success() {
            last = format!("curl exit {}", out.status);
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            if let Some(h) = v
                .pointer("/result/sync_info/latest_block_height")
                .and_then(|x| x.as_u64())
            {
                return Ok(h);
            }
            last = v.to_string().chars().take(120).collect();
        }
    }
    Err(format!("fork_latest_block failed: {last}").into())
}

/// Fork endpoint fallbacks (public RPCs rate-limit/deprecate; try in order).
pub(crate) const FORK_RPC_FALLBACKS: &[&str] = &[
    "https://archival-rpc.mainnet.near.org",
    "https://rpc.mainnet.fastnear.com",
    "https://rpc.mainnet.near.org",
];

/// Try a fork query against the primary RPC, then the fallback chain.
pub(crate) fn fork_rpc_query(
    primary: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let mut last = match rpc_query(primary, params.clone()) {
        Ok(r) => return Ok(r),
        Err(e) => e,
    };
    for url in FORK_RPC_FALLBACKS {
        if *url == primary {
            continue;
        }
        match rpc_query(url, params.clone()) {
            Ok(r) => return Ok(r),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The block reference for fork queries: pinned height or "final".
pub(crate) fn fork_block_ref(cfg: &crate::gas::ForkCfg) -> serde_json::Value {
    match cfg.block {
        Some(b) => serde_json::json!({ "block_id": b }),
        None => serde_json::json!({ "finality": "final" }),
    }
}

const FORK_ABSENT_KEY: &[u8] = b"\x00\x00__near_mock_fork_absent__";

/// Persist tombstones into storage so `MockChain::save()` carries them.
pub(crate) fn fork_sync_absents_to_storage(state: &Arc<Mutex<MockState>>) {
    let absents: Vec<Vec<u8>> = FORK_ABSENT.with(|a| a.borrow().iter().cloned().collect());
    if absents.is_empty() {
        return;
    }
    if let Ok(enc) = bincode::serialize(&absents) {
        state
            .lock()
            .unwrap()
            .storage
            .insert(FORK_ABSENT_KEY.to_vec(), enc);
    }
}

/// Seed tombstones from storage (after a state-file load). Called on fork init.
pub(crate) fn fork_restore_absents_from_storage(state: &Arc<Mutex<MockState>>) {
    let enc = state.lock().unwrap().storage.remove(FORK_ABSENT_KEY);
    if let Some(enc) = enc {
        if let Ok(absents) = bincode::deserialize::<Vec<Vec<u8>>>(&enc) {
            FORK_ABSENT.with(|a| {
                for k in absents {
                    a.borrow_mut().insert(k);
                }
            });
        }
    }
}

pub(crate) fn fork_enabled() -> bool {
    mock_cfg().fork.is_some()
}

/// Resolve a module for `account`: manifest first, then fork cache, then
/// fetch code from the fork RPC + compile (instrumented).
pub(crate) fn fork_get_module(
    engine: &Rc<wasmtime::Engine>,
    account: &str,
) -> Option<wasmtime::Module> {
    if let Some(m) = FORK_MODULES.with(|m| m.borrow().get(account).cloned()) {
        return Some(m);
    }
    let cfg = mock_cfg().fork?;
    let mut params = serde_json::json!({
        "request_type": "view_code", "account_id": account
    });
    if let (k, v) = fork_block_ref(&cfg)
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap()
    {
        params[k] = v.clone();
    }
    let res = match fork_rpc_query(&cfg.rpc, params) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "  🍴 fork: view_code for {account} failed: {e} — old block? use an archival --url"
            );
            return None;
        }
    };
    let code_b64 = res.get("code_base64")?.as_str()?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(code_b64)
        .ok()?;
    let module = compile_module(engine, &bytes).ok()?;
    FORK_MODULES.with(|m| {
        m.borrow_mut().insert(account.to_string(), module.clone());
    });
    mtrace!(
        "  🍴 fork: fetched+compiled {} bytes of code for {} @ block {}",
        bytes.len(),
        account,
        cfg.block
            .map(|b| b.to_string())
            .unwrap_or_else(|| "final".into())
    );
    Some(module)
}

/// Page in storage entries under `raw_key` prefix (usually the exact key)
/// from the fork block. Returns true if the exact `prefixed` key now exists.
/// Caller must NOT hold the state lock (this takes it to insert).
pub(crate) fn fork_page_in(
    state: &Arc<Mutex<MockState>>,
    contract: &str,
    raw_key: &[u8],
    prefixed: &[u8],
) -> bool {
    let cfg = match mock_cfg().fork {
        Some(c) => c,
        None => return false,
    };
    if FORK_ABSENT.with(|a| a.borrow().contains(prefixed)) {
        return false;
    }
    use base64::Engine;
    let prefix_b64 = base64::engine::general_purpose::STANDARD.encode(raw_key);
    // Unpaginated view_state refuses contracts whose TOTAL state is large
    // (TOO_LARGE_CONTRACT_STATE — wrap.near, sweat, intents...). The
    // paginated path (limit + after_key, current nearcore) has no such
    // check: pages of ≤PAGE keys always serve. We fetch exact-key-prefix
    // pages and follow the cursor while keys still extend the prefix.
    let mut found_exact = false;
    let mut fetched = 0usize;
    let mut cursor: Option<Vec<u8>> = None;
    for _page in 0..8 {
        let mut params = serde_json::json!({
            "request_type": "view_state", "account_id": contract,
            "prefix_base64": prefix_b64,
            "limit": 100u32,
        });
        if let (k, v) = fork_block_ref(&cfg)
            .as_object()
            .unwrap()
            .iter()
            .next()
            .unwrap()
        {
            params[k] = v.clone();
        }
        if let Some(after) = &cursor {
            use base64::Engine;
            params["after_key_base64"] =
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(after));
        }
        let res = match fork_rpc_query(&cfg.rpc, params) {
            Ok(r) => r,
            Err(e) => {
                mtrace!("  🍴 fork: view_state failed: {}", e);
                return found_exact;
            }
        };
        let values: Vec<(Vec<u8>, Vec<u8>)> = res
            .get("values")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|entry| {
                        let k = entry.get("key")?.as_str()?;
                        let v = entry.get("value")?.as_str()?;
                        let rk = base64::engine::general_purpose::STANDARD.decode(k).ok()?;
                        let rv = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
                        Some((rk, rv))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if values.is_empty() {
            break;
        }
        let page_len = values.len();
        let last_key = values.last().map(|(k, _)| k.clone());
        {
            let mut st = state.lock().unwrap();
            for (raw_k, val) in values {
                let pk = prefixed_key(contract, &raw_k);
                if pk == prefixed {
                    found_exact = true;
                }
                st.storage.insert(pk, val);
                fetched += 1;
            }
        }
        // continue only while the page was full AND the last key extends the
        // prefix (more may follow under the same prefix)
        if page_len < 100 {
            break;
        }
        match last_key {
            Some(k) => cursor = Some(k),
            None => break,
        }
    }
    if !found_exact {
        // negative cache (keys extending the prefix were cached anyway)
        FORK_ABSENT.with(|a| {
            a.borrow_mut().insert(prefixed.to_vec());
        });
    }
    mtrace!(
        "  🍴 fork: paged {} keys for {} [{}] @ {} — exact={}",
        fetched,
        contract,
        crate::hosts::dbg_key(raw_key),
        cfg.block
            .map(|b| b.to_string())
            .unwrap_or_else(|| "final".into()),
        found_exact
    );
    found_exact
}

/// Record a local deletion so later reads don't resurrect it from the fork.
pub(crate) fn fork_tombstone(prefixed: &[u8]) {
    if fork_enabled() {
        FORK_ABSENT.with(|a| {
            a.borrow_mut().insert(prefixed.to_vec());
        });
    }
}

/// Clear a tombstone (a local write supersedes the chain's absence).
pub(crate) fn fork_untombstone(prefixed: &[u8]) {
    FORK_ABSENT.with(|a| {
        a.borrow_mut().remove(prefixed);
    });
}

fn rpc_query(rpc: &str, params: serde_json::Value) -> Result<serde_json::Value, serde_json::Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": "dontcare", "method": "query", "params": params
    });
    // Transport-level retry: a paginated crawl makes hundreds of reads, so
    // transient hiccups (rate limits, timeouts) must not kill the run. curl
    // retries HTTP-level failures natively and HONORS Retry-After headers
    // (intear sends "retry after N seconds"); the slim in-code loop only
    // covers 200-with-garbage-body cases curl won't retry. All queries here
    // are pure reads (idempotent). A real JSON-RPC error envelope is never
    // retried — the server answered.
    let mut last_transport: Option<serde_json::Value> = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(2_u64 << (attempt - 1)));
        }
        let out = match std::process::Command::new("curl")
            .args([
                "-s",
                "--max-time",
                "60",
                "--retry",
                "10",
                "--retry-all-errors",
                "--retry-connrefused",
                "--retry-max-time",
                "600",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body.to_string(),
                rpc,
            ])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                last_transport = Some(transport_err(&format!("curl spawn failed: {e}")));
                continue;
            }
        };
        if !out.status.success() {
            last_transport = Some(transport_err(&format!(
                "curl exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )));
            continue;
        }
        match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            Ok(v) => {
                if let Some(err) = v.get("error") {
                    return Err(err.clone());
                }
                return Ok(v["result"].clone());
            }
            Err(e) => {
                last_transport = Some(transport_err(&format!(
                    "bad RPC JSON: {e} (body starts {:?})",
                    String::from_utf8_lossy(&out.stdout[..out.stdout.len().min(80)])
                )));
            }
        }
    }
    return Err(last_transport.unwrap_or_else(|| transport_err("exhausted retries")));
}

const SNAPSHOT_PAGE_LIMIT: u64 = 10_000;
const SNAPSHOT_KEY_HARD_CAP: usize = 2_000_000;

/// One view_state page of a contract trie. Empty prefix = whole trie; with
/// `after` set this is a cursor page (nearcore resumes after that key,
/// exclusive). Servers cap page sizes individually — termination is always
/// "empty page", never a partial-count guess.
fn view_state_page(
    rpc: &str,
    account: &str,
    block: Option<u64>,
    after: Option<&[u8]>,
) -> Result<serde_json::Value, serde_json::Value> {
    use base64::Engine;
    let mut params = serde_json::json!({
        "request_type": "view_state",
        "account_id": account,
        "prefix_base64": "",
        "limit": SNAPSHOT_PAGE_LIMIT,
    });
    match block {
        Some(b) => params["block_id"] = serde_json::json!(b),
        None => params["finality"] = serde_json::json!("final"),
    }
    if let Some(a) = after {
        params["after_key_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(a));
    }
    rpc_query(rpc, params)
}

fn err_string(e: &serde_json::Value) -> String {
    e.to_string()
}

/// Synthesized error object for transport-level failures (no JSON-RPC
/// envelope arrived). Distinct `name` so callers never confuse it with a
/// real server error.
fn transport_err(msg: &str) -> serde_json::Value {
    serde_json::json!({ "name": "TRANSPORT_ERROR", "message": msg })
}

/// `snapshot <account> <state.bin>` — one-command live pull (promotes
/// scripts/fetch_near_state.sh): contract trie + wasm code from an RPC
/// endpoint into a near-mock state file, ready for `cross` immediately.
fn run_snapshot(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: near-mock snapshot <account> <state.bin> [--rpc <url>] [--replace-acct] [--no-code]";
    let account = args.get(2).ok_or(usage)?;
    let state_path = args.get(3).ok_or(usage)?;
    let rpc = args
        .iter()
        .position(|a| a == "--rpc")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("NEAR_RPC").ok())
        // Default chain: rpc.mainnet.near.org is deprecated (HTTP 429) and
        // fastnear mainnet doesn't serve `view_state` for all contracts —
        // archival works for both state and code, so it leads the fallbacks.
        .unwrap_or_else(|| "https://archival-rpc.mainnet.near.org".to_string());
    let replace_acct = args.iter().any(|a| a == "--replace-acct");
    let want_code = !args.iter().any(|a| a == "--no-code");

    use base64::Engine;
    let b64d = |s: &str| -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map_err(|e| format!("bad base64: {e}"))
    };

    // 1) contract trie, cursor-paginated. Page 1 uses finality to discover
    // the tip, and every page after runs against the PINNED block so the
    // snapshot is one consistent moment even mid-crawl.
    let first = match view_state_page(&rpc, account, None, None) {
        Ok(page) => page,
        Err(e) if err_string(&e).contains("TOO_LARGE_CONTRACT_STATE") => {
            // Older/small-cap nodes check total contract size before the
            // cursor path. The error carries the block that was too big —
            // retrying pinned to it sometimes lands on the bounded path.
            let hint = e["cause"]["info"]["block_height"].as_u64();
            match view_state_page(&rpc, account, hint, None) {
                Ok(page) => page,
                Err(e2) => {
                    return Err(format!(
                        "view_state failed on both tip and pinned block: {}\n\
                         hint: this RPC caps contract-state reads; a full-cap\n\
                         provider paginates big tries — try --rpc https://rpc.intea.rs",
                        err_string(&e2)
                    )
                    .into());
                }
            }
        }
        Err(e) => return Err(err_string(&e).into()),
    };
    let pinned_block = first.get("block_height").and_then(|b| b.as_u64());
    let mut values = first
        .get("values")
        .and_then(|v| v.as_array())
        .ok_or("view_state response missing values[]")?
        .clone();
    let block_hash = first
        .get("block_hash")
        .and_then(|b| b.as_str())
        .unwrap_or("?")
        .to_string();

    // 2) cursor walk: resume after the last key of the previous page until
    // the server returns an empty page (the ONLY reliable end marker —
    // servers cap page sizes, so a partial page means nothing).
    let mut pages = 1usize;
    while let Some(last) = values
        .last()
        .and_then(|v| v.get("key"))
        .and_then(|k| k.as_str())
        .map(|k| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(k.trim())
                .expect("rpc returned non-base64 key")
        })
    {
        if values.len() > SNAPSHOT_KEY_HARD_CAP {
            return Err(format!(
                "aborting at >{SNAPSHOT_KEY_HARD_CAP} keys: state larger than sane to mock; \
                 snapshot a narrower slice with state import instead"
            )
            .into());
        }
        let page = match view_state_page(&rpc, account, pinned_block, Some(&last)) {
            Ok(pg) => pg,
            Err(e) => return Err(err_string(&e).into()),
        };
        let batch = page
            .get("values")
            .and_then(|v| v.as_array())
            .ok_or("view_state page missing values[]")?;
        if batch.is_empty() {
            break;
        }
        pages += 1;
        values.extend(batch.iter().cloned());
        // Polite pacing: a full walk is hundreds of requests; hammering the
        // endpoint mid-crawl earns rate-limit responses for the rest of it.
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // Provenance: the pinned block (server-reported), '?' if the node omitted it.
    let height_s = pinned_block
        .map(|h| h.to_string())
        .unwrap_or_else(|| "?".to_string());

    // 3) contract code → <account>.wasm (so `cross` binds without extra steps)
    let mut code_path = String::new();
    if want_code {
        let code_res = rpc_query(
            &rpc,
            serde_json::json!({
                "request_type": "view_code",
                "finality": "final",
                "account_id": account,
            }),
        )
        .map_err(|e| err_string(&e))?;
        let code_b64 = code_res
            .get("code_base64")
            .and_then(|c| c.as_str())
            .ok_or("view_code response missing code_base64")?;
        let wasm = b64d(code_b64)?;
        code_path = format!("{account}.wasm");
        std::fs::write(&code_path, &wasm)?;
        println!("📄 code: {} bytes → {code_path}", wasm.len());
    }

    // 4) decode every entry BEFORE touching the state file (atomic-ish)
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(values.len());
    for (i, v) in values.iter().enumerate() {
        let k = v
            .get("key")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("values[{i}] missing key"))?;
        let val = v
            .get("value")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("values[{i}] missing value"))?;
        entries.push((b64d(k)?, b64d(val)?));
    }

    let mut map: std::collections::HashMap<Vec<u8>, Vec<u8>> = match std::fs::read(state_path) {
        Ok(d) => bincode::deserialize(&d)
            .map_err(|e| format!("{state_path}: not a near-mock state file ({e})"))?,
        Err(_) => Default::default(),
    };
    let pre = prefixed_key(account, b"");
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
        map.insert(prefixed_key(account, &k), v);
    }
    std::fs::write(state_path, bincode::serialize(&map)?)?;

    println!(
        "📸 {account}: {} keys @ block {height_s} ({block_hash}) → {state_path}{}",
        values.len(),
        if pages > 1 {
            format!(" [{pages} pages]")
        } else {
            String::new()
        }
    );
    if want_code {
        println!("next:");
        println!(
            "  near-mock cross {state_path} {account}={code_path} {account} <method> '<json>'"
        );
    } else {
        println!("next: bind code manually — near-mock cross {state_path} <acct>=<wasm> ...");
    }
    Ok(())
}

fn run_state_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: near-mock state import <state.bin> <dump.json|- > [--replace-acct]\n       near-mock state dump <state.bin> [account-prefix]  (stdout = JSON, summary on stderr)\n\nformat (both accepted): {\"account\":A,\"values\":[{\"key\":<b64>,\"value\":<b64>},...]}\n                       or flat rows [{\"account\":A,\"key\":<b64>,\"value\":<b64>},...]\n`state dump | state import - ` round-trips.";
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
            // Two accepted shapes (round-trips with `state dump`):
            //   canonical: {"account": A, "values":[{"key": <b64>, "value": <b64>}, ...]}
            //   legacy flat rows (also accepted, incl. "value_b64" alias):
            //              [{"account": A, "key": <b64>, "value": <b64>}, ...]
            fn row_kv(v: &serde_json::Value, i: usize) -> Result<(String, String), String> {
                let k = v
                    .get("key")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| format!("values[{i}] missing key"))?;
                let val = v
                    .get("value")
                    .or_else(|| v.get("value_b64"))
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| format!("values[{i}] missing value"))?;
                Ok((k.to_string(), val.to_string()))
            }
            // account -> entries, filled from whichever shape we got
            let mut per_account: std::collections::BTreeMap<String, Vec<(String, String)>> =
                Default::default();
            if let Some(arr) = dump.get("values").and_then(|v| v.as_array()) {
                let account = dump
                    .get("account")
                    .and_then(|a| a.as_str())
                    .ok_or("dump has \"values\" but missing \"account\"")?
                    .to_string();
                let e: Result<Vec<_>, String> =
                    arr.iter().enumerate().map(|(i, v)| row_kv(v, i)).collect();
                per_account.insert(account, e?);
            } else if let Some(rows) = dump.as_array() {
                for (i, v) in rows.iter().enumerate() {
                    let acct = v
                        .get("account")
                        .and_then(|a| a.as_str())
                        .ok_or_else(|| format!("rows[{i}] missing \"account\""))?
                        .to_string();
                    let (k, val) = row_kv(v, i)?;
                    per_account.entry(acct).or_default().push((k, val));
                }
            } else {
                return Err("unrecognized dump format: expected {\"account\",\"values\":[...]} or [{\"account\",\"key\",\"value\"},...]".into());
            }

            let mut map: std::collections::HashMap<Vec<u8>, Vec<u8>> =
                match std::fs::read(state_path) {
                    Ok(data) => bincode::deserialize(&data)
                        .map_err(|e| format!("{}: not a near-mock state file ({e})", state_path))?,
                    Err(_) => Default::default(), // new file: seed from scratch
                };

            let total: usize = per_account.values().map(|v| v.len()).sum();
            for (account, kvs) in &per_account {
                // decode this partition's entries BEFORE any write
                // (atomic-ish: a malformed dump never half-applies)
                let entries: Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error>> =
                    kvs.iter().map(|(k, v)| Ok((b64(k)?, b64(v)?))).collect();
                let entries = entries?;
                let pre = prefixed_key(account, b"");
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
                    map.insert(prefixed_key(account, &k), v);
                }
            }

            std::fs::write(state_path, bincode::serialize(&map)?)?;
            println!(
                "📥 imported {} keys across {} account(s) → {}{}",
                total,
                per_account.len(),
                state_path,
                if replace_acct {
                    " (partition(s) replaced)"
                } else {
                    ""
                }
            );
            Ok(())
        }
        "dump" => {
            let state_path = args.get(3).ok_or(usage)?;
            let prefix: Option<String> = args.get(4).cloned();
            let data = std::fs::read(state_path).map_err(|e| {
                format!(
                    "{}: {} (create one via a scenario or import)",
                    state_path, e
                )
            })?;
            let map: std::collections::HashMap<Vec<u8>, Vec<u8>> = bincode::deserialize(&data)
                .map_err(|e| format!("{}: not a near-mock state file ({e})", state_path))?;

            use base64::Engine;
            // stdout is PURE JSON (pipe into jq / `state import`). Human
            // summary goes to stderr. Keys/values are base64 so binary
            // state survives the trip — this exact shape is accepted back
            // by `state import` (dump | import round-trips).
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
                    let key_b64 = base64::engine::general_purpose::STANDARD.encode(&k[sep + 1..]);
                    let val_b64 = base64::engine::general_purpose::STANDARD.encode(v);
                    Some((acct, key_b64, val_b64))
                })
                .collect();
            rows.sort();

            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|(acct, key_b64, val_b64)| {
                    serde_json::json!({
                        "account": acct,
                        "key": key_b64,
                        "value": val_b64,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr)?);
            eprintln!(
                "— {} keys{}",
                rows.len(),
                prefix
                    .as_deref()
                    .map(|p| format!(" for '{p}'"))
                    .unwrap_or_default()
            );
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

// ═══════════════════════════════════════════════════════════════════
// near-mock fork — call a contract against real chain state, locally.
//
//   near-mock fork <account> <method> [args-json]
//       [--url RPC] [--block H] [--signer S] [--deposit YOCTO] [--view]
//
// Code and storage are paged in lazily from the archival RPC at the pinned
// block (latest if not given). Writes land locally; nothing is persisted
// unless NEAR_MOCK_STATE is set.
// ═══════════════════════════════════════════════════════════════════
fn run_fork(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: near-mock fork <account> <method> [args-json] \
        [--url RPC] [--block H] [--signer S] [--deposit YOCTO] [--view]";
    let flag = |name: &str| args.iter().position(|a| a == name);
    let val = |name: &str| -> Option<String> { flag(name).and_then(|i| args.get(i + 1)).cloned() };
    let account = args.get(2).ok_or(usage)?;
    let method = args.get(3).ok_or(usage)?;
    let args_json = args
        .get(4)
        .filter(|s| !s.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "{}".into());
    let rpc = val("--url").unwrap_or_else(|| "https://rpc.mainnet.fastnear.com".into());
    let signer = val("--signer");
    let deposit: u128 = val("--deposit").and_then(|d| d.parse().ok()).unwrap_or(0);
    let is_view = flag("--view").is_some();

    let json_mode = flag("--json").is_some();
    if json_mode {
        std::env::set_var("NEAR_MOCK_QUIET", "1");
    }
    match val("--block").as_deref() {
        Some("final") => {
            if !json_mode {
                println!("🍴 fork: {account} @ final (unpinned — state may drift mid-session)");
            }
            set_fork_cfg(rpc.clone(), None);
        }
        Some(s) => {
            let b: u64 = s.parse().map_err(|_| usage)?;
            if !json_mode {
                println!("🍴 fork: {account} @ block {b}");
            }
            set_fork_cfg(rpc.clone(), Some(b));
        }
        None => {
            let b = fork_latest_block(&rpc)?;
            if !json_mode {
                println!("🍴 fork: {account} @ latest block {b} (pinned)");
            }
            set_fork_cfg(rpc.clone(), Some(b));
        }
    }

    // The forked account is the default signer — its state is what we read.
    let signer = signer.unwrap_or_else(|| account.clone());
    let account_static: &str = Box::leak(account.clone().into_boxed_str());
    let method_static: &str = Box::leak(method.clone().into_boxed_str());

    // fork cfg was set explicitly above (pinned / final / latest-pinned);
    // the builder just installs the sandbox — .fork() would re-resolve.
    let chain = MockChain::builder().signer(&signer).build()?;
    if !json_mode {
        println!(
            "▶ {account}.{method}({args_json}){}",
            if is_view { " [view]" } else { "" }
        );
    }

    let mut call = if is_view {
        chain.view(account_static, method_static)
    } else {
        chain.call(account_static, method_static)
    };
    call = call.args(args_json.clone()).from(signer.clone());
    if deposit > 0 {
        call = call.attach(deposit);
    }
    let out = call.fire()?;

    if flag("--json").is_some() {
        let ret = out
            .return_data
            .as_ref()
            .map(|d| match std::str::from_utf8(d) {
                Ok(s) => serde_json::Value::String(s.to_string()),
                Err(_) => serde_json::Value::String(format!("<{} binary bytes>", d.len())),
            })
            .unwrap_or(serde_json::Value::Null);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "account": account, "method": method, "args": args_json,
                "signer": signer, "view": is_view,
                "ok": out.ok, "return": ret,
                "error": out.error, "panic": out.panic,
                "gas_burnt": out.gas_burned, "logs": out.logs,
            }))?
        );
    } else if out.ok {
        println!("✅ Success");
        if let Some(data) = &out.return_data {
            match std::str::from_utf8(data) {
                Ok(s) => println!("📄 {s}"),
                Err(_) => println!("📄 <{} binary bytes>", data.len()),
            }
        }
        for l in &out.logs {
            println!("  LOG: {l}");
        }
    } else {
        println!("❌ {}", out.error.as_deref().unwrap_or("failed"));
        if let Some(p) = &out.panic {
            println!("   class: {p}");
        }
        std::process::exit(1);
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// near-mock replay — re-execute a REAL mainnet transaction locally,
// against forked state at its block, with full internals.
//
//   near-mock replay <tx-hash> [--url RPC] [--json] [--trace] [--block H]
//
// The "why did my tx fail?" command: fetches the tx from FastNear, finds
// the first external function-call receipt, forks state at the block
// BEFORE it executed, replays it with the real predecessor/args/deposit,
// and diffs the outcome against mainnet's recorded result.
// ═══════════════════════════════════════════════════════════════════
fn run_replay(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use base64::Engine;
    let usage = "usage: near-mock replay <tx-hash> [--url RPC] [--json] [--trace] [--block H] [--state FILE]";
    let flag = |name: &str| args.iter().position(|a| a == name);
    let val = |name: &str| -> Option<String> { flag(name).and_then(|i| args.get(i + 1)).cloned() };
    let json_out = flag("--json").is_some();
    if json_out {
        std::env::set_var("NEAR_MOCK_QUIET", "1");
    }
    if flag("--trace").is_some() {
        std::env::set_var("NEAR_MOCK_TRACE", "1");
    }
    let hash = args.get(2).ok_or(usage)?;
    // replay targets OLD blocks by nature — archival default (non-archival
    // answers GARBAGE_COLLECTED_BLOCK for anything older than ~5 epochs)
    let rpc = val("--url").unwrap_or_else(|| "https://archival-rpc.mainnet.near.org".into());
    let raw = http_post_json(
        "https://tx.main.fastnear.com/v0/transactions",
        &serde_json::json!({ "tx_hashes": [hash] }),
    )
    .map_err(|e| format!("tx fetch failed: {e} (is the hash right?)"))?;
    let tx = raw
        .get("transactions")
        .and_then(|t| t.as_array())
        .and_then(|t| t.first())
        .ok_or("transaction not found on FastNear (pruned or wrong hash)")?;

    // 2. first EXTERNAL function-call receipt (predecessor not the receiver;
    //    self-callbacks are promise machinery we execute internally)
    let mut chosen: Option<(String, String, Vec<u8>, u128, String, u64, u64)> = None;
    let mut receipts: Vec<&serde_json::Value> = tx
        .get("receipts")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    receipts.sort_by_key(|r| {
        (
            r.pointer("/receipt/block_height")
                .and_then(|b| b.as_u64())
                .unwrap_or(0),
            r.pointer("/receipt/receipt_index")
                .and_then(|b| b.as_u64())
                .unwrap_or(0),
        )
    });
    for r in &receipts {
        let inner = r.pointer("/receipt").unwrap_or(&serde_json::Value::Null);
        let receiver = inner
            .get("receiver_id")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let predecessor = inner
            .get("predecessor_id")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if receiver.is_empty() || predecessor == receiver {
            continue;
        }
        let Some(action) = inner.pointer("/receipt/Action") else {
            continue;
        };
        if action
            .get("input_data_ids")
            .and_then(|d| d.as_array())
            .map(|d| !d.is_empty())
            .unwrap_or(false)
        {
            continue; // promise-result callback — needs internal context
        }
        let Some(fc) = action
            .get("actions")
            .and_then(|a| a.as_array())
            .and_then(|a| a.iter().find(|x| x.get("FunctionCall").is_some()))
            .and_then(|x| x.get("FunctionCall"))
        else {
            continue;
        };
        let method = fc
            .get("method_name")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let args_b64 = fc
            .get("args")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        let args = base64::engine::general_purpose::STANDARD
            .decode(&args_b64)
            .unwrap_or_default();
        let deposit: u128 = fc
            .get("deposit")
            .and_then(|d| d.as_str())
            .and_then(|d| d.parse().ok())
            .unwrap_or(0);
        let block = inner
            .get("block_height")
            .and_then(|b| b.as_u64())
            .unwrap_or(0);
        let ridx = inner
            .get("receipt_index")
            .and_then(|b| b.as_u64())
            .unwrap_or(0);
        chosen = Some((
            receiver.to_string(),
            method,
            args,
            deposit,
            predecessor.to_string(),
            block,
            ridx,
        ));
        break;
    }
    let Some((receiver, method, args, deposit, predecessor, exec_block, ridx)) = chosen else {
        return Err("no replayable external function-call receipt in this tx \
(pure transfer / callback-only / data receipts)"
            .into());
    };

    // 3. fork at the block BEFORE execution (pre-tx state)
    let pin = val("--block")
        .and_then(|b| b.parse::<u64>().ok())
        .unwrap_or(exec_block.saturating_sub(1));
    set_fork_cfg(rpc.clone(), Some(pin));

    // 4. mainnet ground truth for this receipt
    let eo = receipts
        .iter()
        .find(|r| {
            r.pointer("/receipt/receipt_index").and_then(|b| b.as_u64()) == Some(ridx)
                && r.pointer("/receipt/block_height").and_then(|b| b.as_u64()) == Some(exec_block)
        })
        .and_then(|r| r.pointer("/execution_outcome/outcome"));
    let mn_success = eo
        .and_then(|o| o.get("status"))
        .map(|st| !st.get("Failure").is_some())
        .unwrap_or(false);
    let mn_failure = eo
        .and_then(|o| {
            o.pointer("/status/Failure/ActionError/kind/FunctionCallError/ExecutionError")
        })
        .and_then(|e| e.as_str())
        .map(String::from);
    let mn_gas = eo
        .and_then(|o| o.get("gas_burnt"))
        .and_then(|g| g.as_u64())
        .unwrap_or(0);
    let mn_logs: Vec<String> = eo
        .and_then(|o| o.get("logs"))
        .and_then(|l| l.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // 5. replay locally
    let receiver_static: &str = Box::leak(receiver.clone().into_boxed_str());
    let method_static: &str = Box::leak(method.clone().into_boxed_str());
    let state_file = val("--state");
    let mut builder = MockChain::builder();
    if let Some(f) = &state_file {
        builder = builder.state_path(f);
    }
    let chain = builder.signer(&predecessor).build()?;
    crate::fork_restore_absents_from_storage(&state_arc_tls().unwrap());
    let out = chain
        .call(receiver_static, method_static)
        .args_bytes(args)
        .from(predecessor.clone())
        .attach(deposit)
        .fire()?;
    if let Some(f) = &state_file {
        crate::fork_sync_absents_to_storage(&state_arc_tls().unwrap());
        chain.save().ok();
    }

    let logs_match = out.logs == mn_logs;
    let gas_ratio = if mn_gas > 0 {
        Some(out.gas_burned as f64 / mn_gas as f64)
    } else {
        None
    };

    if json_out {
        let j = serde_json::json!({
            "tx": hash, "block": pin, "receipt": {
                "receiver": receiver, "method": method, "predecessor": predecessor,
                "deposit": deposit.to_string(), "exec_block": exec_block, "receipt_index": ridx,
            },
            "mainnet": { "success": mn_success, "failure": mn_failure,
                         "gas_burnt": mn_gas, "logs": mn_logs },
            "mock": { "ok": out.ok, "error": out.error, "panic": out.panic,
                      "gas_burnt": out.gas_burned, "logs": out.logs,
                      "receipt_failures": out.receipt_failures },
            "match": { "status": out.ok == mn_success, "logs_exact": logs_match,
                       "gas_ratio": gas_ratio },
            "host_trace": if flag("--trace").is_some() {
                serde_json::Value::Array(host_trace_entries().into_iter()
                    .map(|(n, g, e)| serde_json::json!({"host": n, "gas": g, "err": e}))
                    .collect())
            } else { serde_json::Value::Null },
        });
        println!("{}", serde_json::to_string_pretty(&j)?);
    } else {
        println!("🔁 replay {hash} (fork @ block {pin}, receipt executed @ {exec_block})");
        println!("   {predecessor} → {receiver}.{method} (deposit {deposit})");
        println!(
            "   mainnet : {} gas={mn_gas} {}",
            if mn_success { "SUCCESS" } else { "FAIL" },
            mn_failure
                .as_deref()
                .map(|f| format!("— {f}"))
                .unwrap_or_default()
        );
        println!(
            "   mock    : {} gas={} {}",
            if out.ok { "SUCCESS" } else { "FAIL" },
            out.gas_burned,
            out.panic
                .as_deref()
                .map(|p| format!("— {p}"))
                .unwrap_or_default()
        );
        println!(
            "   logs {} · gas {}",
            if logs_match { "MATCH ✓" } else { "differ" },
            gas_ratio
                .map(|r| format!("{r:.2}x"))
                .unwrap_or_else(|| "n/a".into())
        );
        if out.ok != mn_success || (!mn_success && out.panic.as_deref() != mn_failure.as_deref()) {
            println!("   ⚠ outcome diverges from mainnet — see --json for details");
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if matches!(
        args.get(1).map(|s| s.as_str()),
        Some("cross") | Some("call")
    ) {
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
    // `snapshot <account> <state.bin>` — live pull: contract state + wasm via RPC
    if args.get(1).map(|s| s.as_str()) == Some("snapshot") {
        return run_snapshot(&args);
    }
    // `skill [--stdout|--force]` — install the AI-agent skill into this
    // project (.agents/skills/near-mock/). Embedded via include_str! —
    // self-contained binary, same pattern as near-compile 0.1.2.
    if args.get(1).map(|s| s.as_str()) == Some("fork") {
        return run_fork(&args);
    }
    if args.get(1).map(|s| s.as_str()) == Some("replay") {
        return run_replay(&args);
    }
    if args.get(1).map(|s| s.as_str()) == Some("skill") {
        const SKILL_MD: &str = include_str!("../../../skills/SKILL.md");
        const SKILL_SCENARIO: &str = include_str!("../../../skills/example-scenario.json");
        if args.iter().skip(2).any(|a| a == "--stdout") {
            print!("{}", SKILL_MD);
            return Ok(());
        }
        let force = args.iter().skip(2).any(|a| a == "--force");
        let dir = std::path::Path::new(".agents/skills/near-mock");
        std::fs::create_dir_all(dir)?;
        let mut written = Vec::new();
        for (name, content) in [
            ("SKILL.md", SKILL_MD),
            ("example-scenario.json", SKILL_SCENARIO),
        ] {
            let path = dir.join(name);
            if path.exists() && !force {
                continue;
            }
            std::fs::write(&path, content)?;
            written.push(path.display().to_string());
        }
        if written.is_empty() {
            println!("skill already present (use --force to overwrite)");
        } else {
            for w in &written {
                println!("✅ {}", w);
            }
            println!("agents working in this project will now discover it");
        }
        return Ok(());
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
        println!(
            "                                  predecessor/now/advance/gas/attach/expect/...)"
        );
        println!("  near-mock state import <state.bin> <dump.json|- > [--replace-acct]");
        println!("  near-mock state dump <state.bin> [account-prefix]  (stdout = JSON)");
        println!("  near-mock snapshot <account> <state.bin>  (pull live wasm+state via RPC)");
        println!("  near-mock skill [--stdout|--force]  (install the AI-agent skill here)");
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
    }

    fn print_gas_schedule_default() {
        println!("{}", GasSchedule::default().to_json());
    }

    if args.iter().skip(1).any(|a| a == "--version" || a == "-V") {
        // cli convention: --version wins even alongside other flags; the
        // version comes from Cargo.toml so releases can't drift from it
        println!("near-mock {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
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
    // --deposit <yocto> for the single-wasm runner: exported through the
    // env var the tx core reads (NEAR_MOCK_ATTACH). cross/call return from
    // main() before reaching this — they parse --deposit natively in
    // run_cross's flag loop.
    if let Some(d) = flag_val("--deposit") {
        std::env::set_var("NEAR_MOCK_ATTACH", d.trim());
    }
    // --signer <account> — same env-mirror pattern as --deposit. The bare
    // wasm runner used to silently DROP --signer (only the cross/call paths
    // parsed it), so predecessor_account_id() stayed owner.test.near and
    // deposits minted under the wrong token id (nep141:<predecessor>).
    // Found running the deployed intents.near wasm (2026-09-10).
    if let Some(s) = flag_val("--signer") {
        std::env::set_var("NEAR_MOCK_SIGNER", s.trim());
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
        Some(s) if s.starts_with('@') => std::fs::read(&s[1..]).unwrap_or_else(|e| {
            eprintln!("failed to read args file {}: {}", &s[1..], e);
            std::process::exit(2);
        }),
        other => other
            .map(|s| s.as_str())
            .filter(|s| !s.starts_with('-'))
            .map(|s| s.to_string())
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
                format!(
                    "cannot read sidecar {}: {} (compile with ./target/release/compile)",
                    map_path, e
                )
            })?)
            .map_err(|e| format!("bad sidecar {}: {}", map_path, e))?;
        let wasm_bytes = std::fs::read(wasm_path)?;
        let names = crate::name_map::decode_function_names(&wasm_bytes).unwrap_or_default();
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
                println!(
                    "  known names: {:?}",
                    names.iter().map(|(_, n)| n).collect::<Vec<_>>()
                );
            }
        }
        return Ok(());
    }

    let wasm_bytes = std::fs::read(wasm_path)?;
    println!("📦 {} ({} bytes)", wasm_path, wasm_bytes.len());

    // (stack headroom rationale: 2026-08-29 — default ~8MB exhausts around 900
    // nested interpreted calls; 64MB keeps near-mock from being the bottleneck)
    let engine = Engine::new(&base_engine_config())?;
    let module = compile_module(&engine, &wasm_bytes)?;

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

    let mut store = new_store(&engine);
    PREPAID_FUEL.with(|f| *f.borrow_mut() = prepaid_g);
    // 1024 pages = 64MB initial memory. Enough that wee_alloc never needs memory_grow.

    // G-14 (2026-09-02): set the promise-host env BEFORE linking. This
    // driver never initialized STATE_ARC, so build_env_linker's is_some
    // check fell through and bound all 12 promise hosts to silent noops —
    // every fire-and-forget payout executed invisibly (the G-14 "dead
    // arms" were never dead). Same env the cross driver sets.
    //
    // contract defaults to the DOCUMENTED account (help: "NEAR_MOCK_CONTRACT
    // default escrow.test.near") — never the wasm path (that broke all 54
    // auth vectors: sig messages embedded the path). Identity output is
    // byte-identical to the old empty-string + host-fallback behavior
    // (current_account_id already mapped "" → escrow.test.near); the only
    // change is the STORAGE partition: single-call state now lands under
    // escrow.test.near like cross/scenario/snapshot state always did, so
    // dump shows real accounts, the dump prefix filter works, and
    // single-call state is visible to cross/scenario runs of the same
    // account. Old ""-partitioned state files are not migrated.
    let contract_acct =
        std::env::var("NEAR_MOCK_CONTRACT").unwrap_or_else(|_| "escrow.test.near".into());
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
            contract: contract_acct,
            view: run_view,
        })
    });
    let linker = build_env_linker(&mut store, &engine, state.clone(), args_bytes.clone())?;
    let instance = linker.instantiate(&mut store, &module)?;

    // Gas budget into the instrumented global (fuel disabled); run the
    // exported start function if the instrumented module has one.
    if let Some(g) = instance.get_global(&mut store, crate::REMAINING_GAS_EXPORT) {
        g.set(&mut store, wasmtime::Val::I64(prepaid_g as i64)).ok();
    }
    if let Some(start) = instance.get_func(&mut store, "start") {
        let _ = start.call(&mut store, &[], &mut []);
    }

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
    let _result = if run_once {
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

    // Reset the gas budget for the measured run (warm-up burned gas too)
    if let Some(g) = instance.get_global(&mut store, crate::REMAINING_GAS_EXPORT) {
        g.set(&mut store, wasmtime::Val::I64(prepaid_g as i64)).ok();
    }
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
                        println!("  [{}b]={} → [{}b]={}", k.len(), kshow, v.len(), vshow);
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
                mtrace!("  ⛓ resolving promise DAG (root {})", idx);
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

    // Gas report (instrumented remaining_gas global; PV155 per-op costs)
    let gas_burnt = {
        let remaining = instance
            .get_global(&mut store, crate::REMAINING_GAS_EXPORT)
            .map(|g| match g.get(&mut store) {
                wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
                _ => 0,
            })
            .unwrap_or(0);
        prepaid_g.saturating_sub(remaining)
    };
    println!(
        "⛽ gas: {:.6} Tgas burnt / {:.6} Tgas prepaid",
        gas_burnt as f64 / 1e12,
        prepaid_tgas
    );

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
                        .map(|(n, c, e, g)| {
                            serde_json::json!({"host": n, "calls": c, "errors": e, "gas_tgas": g as f64 / 1e12})
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

    // Same CI contract as call/cross: trap/out-of-gas => exit 1.
    if run_outcome != "ok" {
        std::process::exit(1);
    }
    Ok(())
}

/// CLI entrypoint (the bin crate calls this; keeps parsing in the library).
pub fn main_entry(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    main()
}
