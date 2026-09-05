//! Promise DAG: batches, sub-execution, transfer/fn-call settlement.

use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use wasmtime::*;
use lisp_rlm_wasm::bls_validate;
use lisp_rlm_wasm::builtin_ed25519::ed25519_verify_impl;
use lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl;

#[derive(Clone)]
pub(crate) enum PAction {
    FnCall { method: String, args: Vec<u8>, gas: u64, dep: u128 },
    Transfer(u128),
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
        d.push(PromiseBatch { deps, account, creator, actions, is_yield: false });
        d.len() - 1
    })
}

/// Chaos testing: receipt indices forced to FAIL by --fail-receipt /
/// scenario step fail_receipt. Forced receipts execute NO actions and
/// return zero results, so dependents see promiseSucceeded=0 / empty
/// promise_result, exactly like a trapped receipt on-chain.
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
        let acct = if b.account.is_empty() { "(combinator)" } else { &b.account };
        let forced = FAIL_RECEIPTS.with(|f| f.borrow().contains(&i));
        eprintln!(
            "  [map] receipt {}: {} actions={} deps={:?}{}",
            i,
            acct,
            b.actions.len(),
            b.deps,
            if forced { " [FORCED-FAIL]" } else { "" }
        );
    }
}

pub(crate) fn sub_execute(
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
pub(crate) fn execute_promise(idx: usize) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>> {
    let batch = PROMISE_DAG.with(|d| d.borrow()[idx].clone());
    EXECUTED_PROMISES.with(|e| e.borrow_mut().insert(idx));
    let forced = FAIL_RECEIPTS.with(|f| f.borrow().contains(&idx));
    let mut dep_results: Vec<Option<Vec<u8>>> = Vec::new();
    if forced {
        eprintln!("  [boom] receipt {} FORCED to fail (--fail-receipt)", idx);
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
