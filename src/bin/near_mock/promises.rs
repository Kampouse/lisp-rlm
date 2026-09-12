//! Promise DAG: batches, sub-execution, transfer/fn-call settlement.

use super::*;
use wasmtime::*;

thread_local! {
    /// Normalized guest-panic messages from promise receipts executed during
    /// the CURRENT tx ("Smart contract panicked: <msg>", execution order).
    /// sub_execute's trap text used to go to stderr only — differs/replayers
    /// need it in the outcome to compare against mainnet receipt failures.
    pub(crate) static RECEIPT_TRAPS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Gated stderr trace (promise engine): NEAR_MOCK_QUIET=1 silences the
/// per-receipt firehose (replayers); CLI default unchanged.
macro_rules! ptrace {
    ($($arg:tt)*) => {
        if !crate::host_trace_quiet() { eprintln!($($arg)*); }
    };
}

/// Clear per-tx receipt-trap capture (execute_tx hygiene).
pub(crate) fn receipt_traps_reset() {
    RECEIPT_TRAPS.with(|t| t.borrow_mut().clear());
}

/// Last captured trap reason (for final-callback status reporting).
pub(crate) fn receipt_traps_peek_last() -> Option<String> {
    RECEIPT_TRAPS.with(|t| t.borrow().last().cloned())
}

/// Take the captured receipt traps (called once per tx, after the DAG ran).
pub(crate) fn receipt_traps_drain() -> Vec<String> {
    RECEIPT_TRAPS.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

#[cfg(test)]
mod memo_tests {
    //! Regression: the execute-once memo must never outlive its DAG
    //! (2026-09-10 live-catch — cross-tx idx aliasing replayed stale results
    //! and spun the orphan-drain loop at 55M stderr lines/min).
    use super::*;

    /// A memo hit must mark the node EXECUTED like the uncached path —
    /// execute_tx's drain loop terminates only on that invariant.
    #[test]
    fn memo_hit_marks_executed() {
        // fresh thread ⇒ fresh TLS; build a trivial empty batch
        let idx = dag_push(vec![], "b.test.near".into(), vec![]);
        let first = execute_promise(idx).expect("first execution");
        assert!(first.is_empty(), "empty batch has no results");
        assert!(EXECUTED_PROMISES.with(|e| e.borrow().contains(&idx)));

        // simulate the stale-memo situation: new tx clears DAG/executed but
        // the memo survived (the old bug) — the hit must re-mark executed.
        PROMISE_DAG.with(|d| d.borrow_mut().clear());
        EXECUTED_PROMISES.with(|e| e.borrow_mut().clear());
        PROMISE_DAG.with(|d| {
            d.borrow_mut().push(crate::PromiseBatch {
                deps: vec![],
                account: "b.test.near".into(),
                creator: "a.test.near".into(),
                actions: vec![],
                is_yield: false,
            })
        });
        let replayed = execute_promise(0).expect("memo-hit replay");
        assert!(replayed.is_empty());
        assert!(
            EXECUTED_PROMISES.with(|e| e.borrow().contains(&0)),
            "memo hit MUST mark the node executed — drain-loop termination"
        );
    }

    /// fresh_tx_state() (execute_tx hygiene) must clear the memo so idx 0 of
    /// tx N+1 can never alias tx N's memoized receipt 0.
    #[test]
    fn fresh_tx_state_clears_memo() {
        let idx = dag_push(vec![], "b.test.near".into(), vec![]);
        execute_promise(idx).expect("execute");
        assert!(!PROMISE_OUTCOMES.with(|o| o.borrow().is_empty()));
        crate::fresh_tx_state();
        assert!(
            PROMISE_OUTCOMES.with(|o| o.borrow().is_empty()),
            "per-tx hygiene must reset the execute-once memo"
        );
    }
}

#[derive(Clone)]
pub(crate) enum PAction {
    FnCall {
        method: String,
        args: Vec<u8>,
        gas: u64,
        dep: u128,
    },
    Transfer(u128),
    /// promise_batch_action_stake: move `amount` from liquid to staked.
    /// (Unstake = stake(0); the mock tracks the staked balance but pays no
    /// rewards — a validator-rewards simulator is out of scope.)
    Stake {
        amount: u128,
    },
    /// add_key (full-access or function-call): record the ED25519 public key
    /// under the batch account's 0x02 key namespace.
    AddKey {
        pk: Vec<u8>,
    },
    /// delete_key: remove a previously added access key.
    DeleteKey {
        pk: Vec<u8>,
    },
    /// delete_account: wipe the batch account's whole partition and credit
    /// its remaining liquid balance to the beneficiary.
    DeleteAccount {
        beneficiary: String,
    },
    /// promise_batch_action_transfer_to_gas_key (protocol 72+ / near-sdk 5):
    /// transfer to the gas-key holder's implicit account. The mock treats it
    /// as a Transfer (no staking-gas-key account model) — semantics are
    /// receipt-level identical for balance purposes.
    TransferToGasKey {
        amount: u128,
    },
    /// add_gas_key_with_full_access / add_gas_key_with_function_call
    /// (protocol 72+): restricted access keys for gas payments. Recorded;
    /// the mock's key model is ED25519-existence-only, so no enforcement.
    AddGasKey {
        pk: Vec<u8>,
    },
    /// deploy/use global contract (protocol 69): contract-code sharing across
    /// accounts. The mock has no global-contract cache — deploying records
    /// DeployGlobal (treated as DeployContract of the same bytes would need a
    /// code copy the driver doesn't model); use_* records UseGlobal. Contracts
    /// depending on global-contract EXECUTION semantics will not behave.
    DeployGlobalContract {
        code: Vec<u8>,
    },
    UseGlobalContract {
        account_id: String,
    },
}

#[derive(Clone)]
pub(crate) struct PromiseBatch {
    pub(crate) deps: Vec<usize>,
    pub(crate) account: String,
    pub(crate) creator: String,
    pub(crate) actions: Vec<PAction>,
    /// Yield promise (host 82): the callback executes once with a NotReady
    /// result, then re-executes with the payload when host 83 resumes it.
    pub(crate) is_yield: bool,
}

pub(crate) fn dag_push(deps: Vec<usize>, account: String, actions: Vec<PAction>) -> usize {
    let creator = exec_ctx_or_default().contract;
    PROMISE_DAG.with(|d| {
        let mut d = d.borrow_mut();
        d.push(PromiseBatch {
            deps,
            account,
            creator,
            actions,
            is_yield: false,
        });
        d.len() - 1
    })
}

// Chaos testing: receipt indices forced to FAIL by --fail-receipt /
// scenario step fail_receipt. Forced receipts execute NO actions and
// return zero results, so dependents see promiseSucceeded=0 / empty
// promise_result, exactly like a trapped receipt on-chain.
// (plain comment: rustdoc cannot attach docs to a macro invocation)
thread_local! {
    static FAIL_RECEIPTS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Install the forced-failure set (CLI --fail-receipt / scenario fail_receipt).
pub(crate) fn fail_receipts_set(v: &[usize]) {
    FAIL_RECEIPTS.with(|f| *f.borrow_mut() = v.iter().copied().collect());
}

/// True when at least one receipt is force-failed.
pub(crate) fn fail_receipts_any() -> bool {
    FAIL_RECEIPTS.with(|f| !f.borrow().is_empty())
}

/// Print the pending promise DAG (receipt map) so operators know which
/// N to target with --fail-receipt.
pub(crate) fn print_dag_map() {
    let dag = PROMISE_DAG.with(|d| d.borrow().clone());
    for (i, b) in dag.iter().enumerate() {
        let acct = if b.account.is_empty() {
            "(combinator)"
        } else {
            &b.account
        };
        let forced = FAIL_RECEIPTS.with(|f| f.borrow().contains(&i));
        ptrace!(
            "  [map] receipt {}: {} actions={} deps={:?}{}",
            i,
            acct,
            b.actions.len(),
            b.deps,
            if forced { " [FORCED-FAIL]" } else { "" }
        );
    }
}

/// Execute one function call on `account`'s contract in a FRESH Store
/// (never re-enter a live instance — the heap global would be clobbered).
/// Signer/predecessor = `predecessor` (promise calls aren't user-signed).
/// Returns Some(return-bytes) on success, None on trap (state reverted).
pub(crate) fn sub_execute(
    account: &str,
    method: &str,
    args: &[u8],
    predecessor: &str,
    deposit: u128,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let module = MODULES
        .with(|m| {
            m.borrow()
                .as_ref()
                .and_then(|map| map.get(account).cloned())
        })
        // fork-mode: unknown account → page code in from the fork RPC
        .or_else(|| {
            let engine = ENGINE_TLS
                .with(|e| e.borrow().clone())
                .expect("ENGINE_TLS set");
            crate::fork_get_module(&engine, account)
        });
    let Some(module) = module else {
        // Unknown-account FnCall receipts FAIL on-chain (AccountDoesNotExist)
        // — but ONLY the receipt fails: the parent tx COMMITS and any
        // callback receives a Failed promise_result (2026-09-10 receipt-
        // semantics fix; the 2026-09-02 hard-error overcorrected silent
        // Ok(None) into whole-tx rollback, which made on-chain recovery
        // paths — e.g. "MPC down → refund bets" — untestable).
        // Record the reason for outcome.receipt_failures, then Failed.
        ptrace!(
            "  ⚠ cross: FnCall to unknown account '{}' → receipt FAILED (parent commits)",
            account
        );
        RECEIPT_TRAPS.with(|t| {
            t.borrow_mut()
                .push(format!("AccountDoesNotExist: {account}"))
        });
        return Ok(None);
    };
    let state = STATE_ARC
        .with(|s| s.borrow().clone())
        .expect("STATE_ARC set");
    let engine = ENGINE_TLS
        .with(|e| e.borrow().clone())
        .expect("ENGINE_TLS set");

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
        let bal: u128 = st
            .storage
            .get(&credit_key)
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        st.storage
            .insert(credit_key.clone(), (bal + deposit).to_string().into_bytes());
        let debit_key = prefixed_key(predecessor, b"\x00near-bal");
        let sbal: u128 = st
            .storage
            .get(&debit_key)
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        st.storage.insert(
            debit_key,
            (sbal.saturating_sub(deposit)).to_string().into_bytes(),
        );
        ptrace!(
            "  💰 fn-call deposit {} yocto: {} → {}",
            deposit,
            predecessor,
            account
        );
    }
    let saved_dep = CURRENT_DEPOSIT.with(|d| d.borrow_mut().replace(deposit));

    ptrace!(
        "  ↳ cross: {}.{}({})",
        account,
        method,
        String::from_utf8_lossy(args)
    );
    let part_snap = { snapshot_partition(&state.lock().unwrap(), account) };

    let mut sub_store = crate::new_store(&*engine);
    // sub_execute runs on the promise worker thread, where the scenario's
    // epoch ticker never ticks — but wasmtime's DEFAULT epoch deadline is 0,
    // so under epoch_interruption the first epoch check (current >= deadline)
    // fires as soon as the main thread's ticker has advanced once → instant
    // Interrupt. Give receipt stores an unbounded epoch deadline; the
    // instrumented gas global bounds runaway child compute.
    sub_store.set_epoch_deadline(u64::MAX);
    let linker = build_env_linker(&mut sub_store, &*engine, state.clone(), Vec::new())?;
    let instance = linker.instantiate(&mut sub_store, &module)?;
    // Receipt gas budget into the instrumented global (entry-budget for the
    // sub-receipt; mainnet gives each promise receipt its own prepaid gas).
    {
        let prepaid = PREPAID_FUEL.with(|r| *r.borrow());
        if let Some(g) = instance.get_global(&mut sub_store, crate::REMAINING_GAS_EXPORT)
        {
            g.set(&mut sub_store, wasmtime::Val::I64(prepaid as i64))
                .ok();
        }
        if let Some(start) = instance.get_func(&mut sub_store, "start") {
            let _ = start.call(&mut sub_store, &[], &mut []);
        }
        // Promise FnCall receipts burn the same execution-side action fee.
        crate::burn_gas_global(
            &mut sub_store,
            &instance,
            crate::FUNCTION_CALL_BASE_GAS
                + crate::FUNCTION_CALL_BYTE_GAS * args.len() as u64,
        );
    }
    let ok = instance
        .get_func(&mut sub_store, method)
        .map(|f| f.call(&mut sub_store, &[], &mut []));
    let (trap, ret, trap_why) = match ok {
        None => (true, None, "missing method (failed receipt)".to_string()),
        Some(res) => match res {
            Ok(()) => (
                false,
                state.lock().unwrap().return_data.clone(),
                String::new(),
            ),
            Err(e) => {
                let code = e
                    .downcast_ref::<wasmtime::Trap>()
                    .map(|t| format!("{:?}", t))
                    .unwrap_or_else(|| "n/a".into());
                // Walk the whole chain: guest panics (panic_utf8) hide in
                // source() links, not the top-level backtrace Display.
                let mut why = format!("{}", e);
                for c in e.chain().skip(1) {
                    why.push_str(&format!("\n| caused: {}", c));
                }
                // Publish the guest panic in mainnet ExecutionError format so
                // library differs can compare failure classes per receipt.
                if let Some(i) = why.find("PANIC: ") {
                    let line = why[i + "PANIC: ".len()..]
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim_end();
                    if !line.is_empty() {
                        RECEIPT_TRAPS.with(|t| {
                            t.borrow_mut()
                                .push(format!("Smart contract panicked: {line}"))
                        });
                    }
                }
                (true, None, format!("{} [trap-code: {}]", why, code))
            }
        },
    };
    if trap {
        // Surface the guest panic (message hides mid-backtrace; lead with it).
        let pan: Vec<&str> = trap_why
            .lines()
            .skip_while(|l| !l.contains("panicked at"))
            .take(2)
            .collect();
        let why = if pan.is_empty() {
            trap_why.lines().last().unwrap_or("unknown").to_string()
        } else {
            pan.join(" | ")
        };
        ptrace!(
            "  ⚠ cross: {}.{} TRAPPED — reverting partition ({})",
            account,
            method,
            why
        );
        restore_partition(&mut state.lock().unwrap(), part_snap, account);
        if deposit > 0 {
            // failed receipt refunds its deposit to the sender
            let mut st = state.lock().unwrap();
            let rk = prefixed_key(predecessor, b"\x00near-bal");
            let b: u128 = st
                .storage
                .get(&rk)
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            st.storage
                .insert(rk, (b + deposit).to_string().into_bytes());
            ptrace!(
                "  💰 deposit {} refunded to {} (failed receipt)",
                deposit,
                predecessor
            );
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
    // 2026-09-07 (intents ft_resolve_withdraw): None must mean TRAP only.
    // A void method (no value_return) is Successful with EMPTY data on-chain —
    // near-sdk's promise_result_checked maps that to Ok(Some(vec![])), which
    // resolvers treat as success. Void receipts used to get None = Failed,
    // so every plain ft_withdraw "failed" and refunded.
    Ok(if trap {
        None
    } else {
        Some(ret.unwrap_or_default())
    })
}

/// Resolve a promise DAG node: deps first (their results, flattened,
/// become this batch's promise_results), then this batch's actions.
///
/// Execute-once (2026-09-08): a node reachable through two parents (Burrow's
/// swap receipt feeding both the resolve callback and the payout leg) used to
/// EXECUTE TWICE — pass 2 saw pass 1's consumed state and trapped (`There is
/// no action for the position`). First resolution memoizes the result in
/// PROMISE_OUTCOMES; every later visit replays it (data-receipt semantics).
pub(crate) fn execute_promise(
    idx: usize,
) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>> {
    if let Some(hit) = PROMISE_OUTCOMES.with(|o| o.borrow().get(&idx).cloned()) {
        ptrace!(
            "  ⛓ receipt {} — replaying memoized result (execute-once)",
            idx
        );
        // Termination guarantee (2026-09-10): a memo hit must mark the node
        // executed like the uncached path does, or execute_tx's orphan-drain
        // loop (find next unexecuted → execute_promise) rediscovers this idx
        // forever. The per-tx PROMISE_OUTCOMES clear makes hits rare (diamond
        // revisits only), but the drain loop must never depend on that.
        EXECUTED_PROMISES.with(|e| {
            e.borrow_mut().insert(idx);
        });
        return Ok(hit);
    }
    let out = execute_promise_uncached(idx)?;
    PROMISE_OUTCOMES.with(|o| o.borrow_mut().insert(idx, out.clone()));
    Ok(out)
}

fn execute_promise_uncached(
    idx: usize,
) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>> {
    let batch = PROMISE_DAG.with(|d| d.borrow()[idx].clone());
    EXECUTED_PROMISES.with(|e| e.borrow_mut().insert(idx));
    let forced = FAIL_RECEIPTS.with(|f| f.borrow().contains(&idx));
    let mut dep_results: Vec<Option<Vec<u8>>> = Vec::new();
    if forced {
        ptrace!("  [boom] receipt {} FORCED to fail (--fail-receipt)", idx);
        // Deps still execute: they are temporally earlier receipts and
        // commit on-chain even when this one fails. The actions of this
        // batch never run, so the result is FAILED.
        for dep in &batch.deps {
            dep_results.extend(execute_promise(*dep)?);
        }
        return Ok(Vec::new());
    }
    for dep in &batch.deps {
        dep_results.extend(execute_promise(*dep)?);
    }
    // Publish dep results for promise_results_count/promise_result — real
    // near-sdk callbacks (Burrow's count-and-branch resolvers) read these
    // BEFORE promise_result(0); the host used to be a silent noop → count 0.
    let old_mirror = promise_results_tls();
    set_promise_results_tls(dep_results.clone());
    let saved =
        PROMISE_RESULTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), dep_results.clone()));
    let mut out = Vec::new();
    let mut batch_touched: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    if batch.is_yield {
        // NEAR yield: the callback runs once NOW with a NotReady result
        // (promiseSucceeded(0)==0 → contract returns its pending path),
        // then re-runs with the payload when host 83 resumes the handle.
        let saved = PROMISE_RESULTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), vec![None]));
        set_promise_results_tls(vec![None]);
        for action in &batch.actions {
            if let PAction::FnCall {
                method, args, dep, ..
            } = action
            {
                match sub_execute(&batch.account, method, args, &batch.creator, *dep) {
                    Ok(ret) => out.push(ret),
                    Err(_) => out.push(None),
                }
            }
        }
        PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
        set_promise_results_tls(old_mirror);
        return Ok(out);
    }
    if !batch.account.is_empty() {
        for action in &batch.actions {
            match action {
                PAction::FnCall {
                    method, args, dep, ..
                } => {
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
                    let child_ret =
                        PENDING_RETURN.with(|p| std::mem::replace(&mut *p.borrow_mut(), outer_ret));
                    if let Some(ridx) = child_ret {
                        ptrace!("  ⛓ child returned promise {ridx} — resolving before dependents");
                        let cres = execute_promise(ridx)?;
                        // NOTE (2026-09-08): the child's results APPEND after the
                        // local result — do NOT replace slot 0. Tried strict
                        // promise_return substitution (pop local first): the
                        // payout-chain residue landed in promise_result(0),
                        // callback_dex_trade took its consume-branch early and
                        // burrowland's SwapReference handler (which owns the
                        // position_latest_actions.remove) panicked "There is no
                        // action for the position" → margin_open_failed. The
                        // append convention reproduces the mainnet-success end
                        // state; slot-exact substitution needs a study of real
                        // receipt ordering for returned-promise chains.
                        for r2 in cres {
                            out.push(r2);
                        }
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
                        ptrace!(
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
                    batch_touched.push((debit_key.clone(), st.storage.get(&debit_key).cloned()));
                    st.storage
                        .insert(debit_key, (bal - *amt).to_string().into_bytes());
                    let credit_key = prefixed_key(&batch.account, b"\x00near-bal");
                    batch_touched.push((credit_key.clone(), st.storage.get(&credit_key).cloned()));
                    let rbal: u128 = st
                        .storage
                        .get(&credit_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    let rbal = rbal + *amt;
                    st.storage.insert(credit_key, rbal.to_string().into_bytes());
                    ptrace!(
                        "  ↗ transfer {} yocto → {} (bal now {})",
                        amt,
                        batch.account,
                        rbal
                    );
                    // TRUE NEAR: successful transfer = Successful(empty).
                    // Contracts distinguish it from Failed via
                    // near/promise_succeeded (status probe). No more marker.
                    out.push(Some(vec![]));
                }
                PAction::TransferToGasKey { amount } => {
                    // protocol 72+ transfer_to_gas_key: the mock has no
                    // gas-key account model — apply it as a plain transfer
                    // (balance moves; the gas-key bookkeeping is out of
                    // scope). Loud note so nobody mistakes it for the real
                    // staking-gas mechanics.
                    ptrace!(
                        "  ⚠ transfer_to_gas_key {amount} yocto → {} (mock: applied as plain transfer; gas-key accounting not modeled)",
                        batch.account
                    );
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    let debit_key = prefixed_key(&batch.creator, b"\x00near-bal");
                    let bal: u128 = st
                        .storage
                        .get(&debit_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    if bal < *amount {
                        ptrace!("  ⚠ transfer_to_gas_key INSUFFICIENT (receipt reverts)");
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
                        out.push(None);
                        break;
                    }
                    batch_touched.push((debit_key.clone(), st.storage.get(&debit_key).cloned()));
                    st.storage
                        .insert(debit_key, (bal - *amount).to_string().into_bytes());
                    let credit_key = prefixed_key(&batch.account, b"\x00near-bal");
                    batch_touched.push((credit_key.clone(), st.storage.get(&credit_key).cloned()));
                    let rbal: u128 = st
                        .storage
                        .get(&credit_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    let rbal = rbal + *amount;
                    st.storage.insert(credit_key, rbal.to_string().into_bytes());
                    out.push(Some(vec![]));
                }
                PAction::AddGasKey { pk: _ } => {
                    // recorded for the promise trace; the mock's key model
                    // (ED25519 existence-only) applies — no enforcement.
                    ptrace!("  🔑 add_gas_key (recorded; mock applies no gas-key rules)");
                    out.push(Some(vec![]));
                }
                PAction::DeployGlobalContract { code: _ } => {
                    ptrace!(
                        "  ⚠ deploy_global_contract (recorded ONLY — the mock has no global-contract cache; use_* receipts will not execute code)"
                    );
                    out.push(Some(vec![]));
                }
                PAction::UseGlobalContract { account_id } => {
                    ptrace!(
                        "  ⚠ use_global_contract({account_id}) (no-op in mock — no global-contract cache)"
                    );
                    out.push(Some(vec![]));
                }
                PAction::Stake { amount } => {
                    // NEAR semantics: staking moves liquid → staked (locked).
                    // Over-stake beyond liquid = this receipt fails (reverts).
                    let amount = *amount;
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    let bal_key = prefixed_key(&batch.account, b"\x00near-bal");
                    let locked_key = prefixed_key(&batch.account, b"\x00near-locked");
                    let bal: u128 = st
                        .storage
                        .get(&bal_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    if amount > bal {
                        ptrace!(
                            "  ⚠ stake {} yocto: {} has only {} liquid — receipt reverts",
                            amount,
                            batch.account,
                            bal
                        );
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
                        out.push(None);
                        break;
                    }
                    batch_touched.push((bal_key.clone(), st.storage.get(&bal_key).cloned()));
                    st.storage
                        .insert(bal_key, (bal - amount).to_string().into_bytes());
                    let cur_locked: u128 = st
                        .storage
                        .get(&locked_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    st.storage
                        .insert(locked_key, (cur_locked + amount).to_string().into_bytes());
                    ptrace!(
                        "  ↗ staked {} yocto ({} total locked, {} liquid)",
                        amount,
                        cur_locked + amount,
                        bal - amount
                    );
                    out.push(Some(vec![]));
                }
                PAction::AddKey { pk } => {
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    // 0x02-prefixed access-key namespace: key = pk, value = kind
                    let _key = prefixed_key(&batch.account, &[0x02]);
                    let mut full = vec![0x02];
                    full.extend_from_slice(&pk);
                    let before = st.storage.insert(full.clone(), b"full".to_vec());
                    // storage growth reverts with the receipt on failure
                    if before.is_none() {
                        batch_touched.push((full, None));
                    }
                    ptrace!("  🔑 key added (full access)");
                    out.push(Some(vec![]));
                }
                PAction::DeleteKey { pk } => {
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    let mut full = vec![0x02];
                    full.extend_from_slice(&pk);
                    let key = prefixed_key(&batch.account, &full);
                    if st.storage.remove(&key).is_some() {
                        batch_touched.push((key, None)); // removal reverts on failure
                        ptrace!("  🗑 key deleted");
                    } else {
                        ptrace!("  ⚠ delete_key: unknown key — receipt reverts");
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
                        out.push(None);
                        break;
                    }
                    out.push(Some(vec![]));
                }
                PAction::DeleteAccount { beneficiary } => {
                    let state = STATE_ARC.with(|s| s.borrow().clone()).ok_or("no state")?;
                    let mut st = state.lock().unwrap();
                    let bal_key = prefixed_key(&batch.account, b"\x00near-bal");
                    let bal: u128 = st
                        .storage
                        .get(&bal_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    // credit everything to the beneficiary, then wipe partition
                    let ben_bal_key = prefixed_key(&beneficiary, b"\x00near-bal");
                    let ben_bal: u128 = st
                        .storage
                        .get(&ben_bal_key)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0u128);
                    st.storage
                        .insert(ben_bal_key, (ben_bal + bal).to_string().into_bytes());
                    let pre = prefixed_key(&batch.account, b"");
                    let removed: Vec<Vec<u8>> = st
                        .storage
                        .keys()
                        .filter(|k| k.starts_with(&pre) && k.len() > pre.len())
                        .cloned()
                        .collect();
                    for k in removed {
                        st.storage.remove(&k);
                    }
                    ptrace!(
                        "  💥 account {} deleted; {} yocto → {beneficiary}",
                        batch.account,
                        bal
                    );
                    out.push(Some(vec![]));
                }
            }
        }
    }
    PROMISE_RESULTS.with(|r| *r.borrow_mut() = saved);
    set_promise_results_tls(old_mirror);
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
