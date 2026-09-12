//! build_env_linker: all 92 NEAR host functions (storage, registers,
//! context, crypto, promises, precompiles).

use super::*;
use crate::bls_validate;
use crate::ed25519::ed25519_verify_impl;
use crate::schnorr::schnorr_verify_impl;
use std::collections::HashMap;
use std::sync::Mutex;
use wasmtime::*;

/// Debug-escape a raw trie key for trace output: printable ASCII as-is,
/// everything else as \xNN so borsh prefixes and account separators stay
/// legible (e.g. "\x04\x0ftoken.chat.near").
pub(crate) fn dbg_key(k: &[u8]) -> String {
    let mut s = String::with_capacity(k.len() + 2);
    for &b in k {
        if (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\' {
            s.push(b as char);
        } else {
            s.push_str(&format!("\\x{b:02x}"));
        }
    }
    s
}

/// Wrap a host closure with --trace instrumentation: one timeline entry
/// (seq, name, exact fuel delta across the host body, err flag) per
/// invocation when tracing is on; a TLS read + branch otherwise.
/// The fuel delta is EXACT: read before the body, after the body — captures
/// whatever the host charged via set_fuel, regardless of schedule.

/// Charge gas against the instrumented `remaining_gas` global (the single
/// gas counter since the finite-wasm instrumentation landed — wasmtime fuel
/// is disabled). Errors with mainnet's message when exhausted.
pub(crate) fn charge_gas(
    caller: &mut wasmtime::Caller<'_, crate::StoreData>,
    amount: u64,
) -> Result<(), wasmtime::Error> {
    use wasmtime::AsContextMut;
    let Some(g) = caller
        .get_export(crate::REMAINING_GAS_EXPORT)
        .and_then(|e| e.into_global())
    else {
        return Err(wasmtime::Error::msg(
            "gas global missing (module not instrumented)",
        ));
    };
    let cur = match g.get(caller.as_context_mut()) {
        wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
        _ => return Err(wasmtime::Error::msg("gas global wrong type")),
    };
    if cur < amount {
        return Err(wasmtime::Error::msg("Exceeded the prepaid gas"));
    }
    g.set(
        caller.as_context_mut(),
        wasmtime::Val::I64((cur - amount) as i64),
    )
    .map_err(wasmtime::Error::from)?;
    Ok(())
}

pub(crate) fn host_fn(
    name: &'static str,
    store: &mut wasmtime::Store<crate::StoreData>,
    ty: wasmtime::FuncType,
    f: impl Fn(
            &mut wasmtime::Caller<'_, crate::StoreData>,
            &[wasmtime::Val],
            &mut [wasmtime::Val],
        ) -> Result<(), wasmtime::Error>
        + Send
        + Sync
        + 'static,
) -> wasmtime::Func {
    wasmtime::Func::new(store, ty, move |mut caller, args, results| {
        // PV155 base cost of ANY host-function invocation (264,768,111 gas,
        // protocol-86 snapshot) — charged here at the single chokepoint every
        // host passes through; named per-host costs are charged inside `f`.
        crate::hosts::charge_gas(&mut caller, crate::HOST_CALL_BASE_GAS)?;
        // mock_cfg() falls back to RunCfg::default() on worker threads, so
        // NEAR_MOCK_TRACE=1 reaches promise sub-execution without TLS setup.
        let on = mock_cfg().trace;
        let gas_now = |c: &mut wasmtime::Caller<'_, crate::StoreData>| -> u64 {
            c.get_export(crate::REMAINING_GAS_EXPORT)
                .and_then(|e| e.into_global())
                .map(|g| match g.get(c) {
                    wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
                    _ => 0,
                })
                .unwrap_or(0)
        };
        let before = if on { gas_now(&mut caller) } else { 0 };
        let r = f(&mut caller, args, results);
        if on {
            let after = gas_now(&mut caller);
            trace_host(name, before.saturating_sub(after), r.is_err());
        }
        r
    })
}

/// Gated stderr trace: every host call prints by default (CLI UX), but
/// long-running library consumers (stream replayers) set NEAR_MOCK_QUIET=1
/// to silence the per-call firehose. Behavior identical otherwise.
macro_rules! htrace {
    ($($arg:tt)*) => {
        if !crate::host_trace_quiet() { eprintln!($($arg)*); }
    };
}

pub(crate) fn build_env_linker(
    store: &mut wasmtime::Store<crate::StoreData>,
    engine: &wasmtime::Engine,
    state: std::sync::Arc<Mutex<MockState>>,
    single_input: Vec<u8>,
) -> Result<wasmtime::Linker<crate::StoreData>, Box<dyn std::error::Error>> {
    let mut linker = wasmtime::Linker::new(engine);
    // === Host functions (all created before linking) ===

    let _s1 = state.clone();
    let log_fn = host_fn(
        "log_utf8",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            // PV155 composite: read_memory + utf8_decoding + log
            let cost = mock_cfg().gas.log_base + mock_cfg().gas.log_byte * len as u64
                + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * len as u64
                + crate::UTF8_DECODING_BASE_GAS + crate::UTF8_DECODING_BYTE_GAS * len as u64;
            charge_gas(&mut caller, cost)?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let msg = String::from_utf8_lossy(&data[ptr..ptr + len]).to_string();
                    // NEP-297 EVENT_JSON decoded; suffix shows ptr/len in --debug
                    handle_log_line(
                        &msg,
                        mock_cfg().debug,
                        &format!("  [debug len={ptr} ptr={len}]"),
                    );
                } else {
                    println!("  LOG: <out-of-range> [debug len={} ptr={}]", len, ptr);
                }
            }
            Ok(())
        },
    );

    let s2 = state.clone();
    let value_return_fn = host_fn(
        "value_return",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            htrace!("  → value_return(len={}, ptr={})", len, ptr);
            // Fee schedule: read_memory base + per byte
            let cost =
                mock_cfg().gas.value_return_base + mock_cfg().gas.value_return_byte * len as u64;
            charge_gas(&mut caller, cost)?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let mut st = s2.lock().unwrap();
                    // LAST-write-wins — nearcore semantics. The old
                    // first-write guard diverged from the chain: a contract
                    // calling value_return twice (e.g. jsonReturnStr("1")
                    // followed by an export-level `return 0`, which the TS
                    // frontend also lowers to value_return) returned the
                    // FIRST value on the mock and the LAST on-chain
                    // (dogfooded live via registry-nostrgov.testnet
                    // 2026-09-09: chain said "0", mock said "1" — the
                    // divergence masked a real contract bug). Receipt
                    // isolation is unaffected: sub_execute saves/clears/
                    // restores return_data around sub-calls structurally.
                    st.return_data = Some(data[ptr..ptr + len].to_vec());
                }
            }
            Ok(())
        },
    );

    let s3 = state.clone();
    let read_register_fn = host_fn(
        "read_register",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (rid, ptr) = (args[0].unwrap_i64() as u64, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                if let Some(data) = s3.lock().unwrap().registers.get(&rid).cloned() {
                    // PV155 composite: read_register + write_memory (guest copy)
                    let cost = mock_cfg().gas.read_register_base
                        + mock_cfg().gas.read_register_byte * data.len() as u64
                        + crate::WRITE_MEMORY_BASE_GAS
                        + crate::WRITE_MEMORY_BYTE_GAS * data.len() as u64;
                    charge_gas(&mut caller, cost)?;
                    let md = mem.data_mut(&mut caller);
                    if ptr + data.len() <= md.len() {
                        md[ptr..ptr + data.len()].copy_from_slice(&data);
                        htrace!("  → read_register({}, ptr={}) ok {}b", rid, ptr, data.len());
                    } else {
                        htrace!(
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
                    htrace!("  ⚠ read_register({}): not found → trap", rid);
                    return Err(wasmtime::Error::msg(format!(
                        "InvalidRegisterId {{ register_id: {} }}",
                        rid
                    )));
                }
            }
            Ok(())
        },
    );

    let s4 = state.clone();
    let register_len_fn = host_fn(
        "register_len",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![ValType::I64]),
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
            charge_gas(&mut caller, crate::READ_MEMORY_BASE_GAS)?;
            htrace!("  → register_len({}) = {}", rid, len);
            results[0] = Val::I64(len);
            Ok(())
        },
    );

    let s5 = state.clone();
    let input_fn = host_fn(
        "input",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let single_input = single_input.clone();
            let rid = args[0].unwrap_i64() as u64;
            htrace!("  → input(reg={})", rid);
            let bytes = EXEC_CTX
                .with(|c| c.borrow().as_ref().map(|x| x.input.clone()))
                .unwrap_or_else(|| single_input.clone());
            // Indicative legacy fee: write_register base + per byte
            let cost = crate::WRITE_REGISTER_BASE_GAS
                + crate::WRITE_REGISTER_BYTE_GAS * bytes.len() as u64;
            charge_gas(&mut caller, cost)?;
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
    let storage_write_fn = host_fn(
        "storage_write",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 5], vec![ValType::I64]),
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
            let mut evicted = false;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() && vp + vl <= md.len() {
                    let raw_key = md[kp..kp + kl].to_vec();
                    let acct = exec_ctx_or_default().contract;
                    let key = prefixed_key(&acct, &raw_key);
                    let val = md[vp..vp + vl].to_vec();
                    htrace!("  → storage_write(\"{}\") = {}b", dbg_key(&raw_key), vl);
                    // PV155 composite: read_memory(key) + read_memory(value)
                    // + storage_write (evicted write_register added post-insert)
                    let gas = &mock_cfg().gas;
                    let cost = gas.storage_write_base
                        + gas.storage_write_key_byte * kl as u64
                        + gas.storage_write_value_byte * vl as u64
                        + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * kl as u64
                        + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * vl as u64;
                    charge_gas(&mut caller, cost)?;
                    let mut st = s6.lock().unwrap();
                    let trie = trie_charge_write(&mut st, &key);
                    let (klen, vlen) = (key.len(), val.len());
                    let old = st.storage.insert(key.clone(), val);
                    crate::fork_untombstone(&key);
                    evicted = old.is_some();
                    // Storage staking: lock for net new bytes (refund replaced).
                    // Prefixed key = acct + '\0' + raw key → raw key = klen - acct - 1.
                    let old_raw_len = old
                        .as_ref()
                        .map(|o| klen - acct.len() - 1 + o.len())
                        .unwrap_or(0);
                    apply_staking_delta(&mut st, &acct, (klen + vlen) as i64 - old_raw_len as i64);
                    drop(st);
                    charge_gas(&mut caller, trie)?;
                    let mut st = s6.lock().unwrap();
                    if rid != u64::MAX {
                        if let Some(old) = old {
                            drop(st);
                            charge_gas(
                                &mut caller,
                                crate::WRITE_REGISTER_BASE_GAS
                                    + crate::WRITE_REGISTER_BYTE_GAS * old.len() as u64,
                            )?;
                            let mut st = s6.lock().unwrap();
                            write_reg_checked(&mut st, rid, old)
                                .map_err(|e| wasmtime::Error::msg(e))?;
                        }
                    }
                }
            }
            // NEAR ABI: 1 = an old value was evicted (written to the evicted
            // register), 0 = key was absent. SDK 4.x collections branch on
            // this flag (Vector::replace_raw panics INCONSISTENT_STATE on 0).
            results[0] = Val::I64(if evicted { 1 } else { 0 });
            Ok(())
        },
    );

    let s7 = state.clone();
    let storage_read_fn = host_fn(
        "storage_read",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
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
                    htrace!("  → storage_read found {}b", val.len());
                    if std::env::var("NEAR_MOCK_KEYS").is_ok() {
                        let acct = exec_ctx_or_default().contract;
                        let raw = if key.len() > acct.len() {
                            &key[acct.len() + 1..]
                        } else {
                            &key[..]
                        };
                        htrace!("    🔑 key [{}]", dbg_key(raw));
                    }
                    // PV155 composite: read_memory(key) + storage_read +
                    // trie + write_register(value → register)
                    let gas = &mock_cfg().gas;
                    let trie = trie_charge(&mut st, key);
                    let cost = gas.storage_read_base
                        + gas.storage_read_key_byte * kl as u64
                        + gas.storage_read_value_byte * val.len() as u64
                        + trie
                        + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * kl as u64
                        + crate::WRITE_REGISTER_BASE_GAS
                        + crate::WRITE_REGISTER_BYTE_GAS * val.len() as u64;
                    drop(st);
                    charge_gas(&mut caller, cost)?;
                    let mut st = s7.lock().unwrap();
                    write_reg_checked(&mut st, rid, val).map_err(|e| wasmtime::Error::msg(e))?;
                    true
                } else if crate::fork_enabled() {
                    // fork-mode: local miss → page the key in from the fork
                    // block, then re-check (found path re-runs below by
                    // re-locking; keep it simple: recursive-free re-lookup)
                    drop(st);
                    let contract = exec_ctx_or_default().contract;
                    let raw = &key[contract.len() + 1..];
                    let hit = crate::fork_page_in(&s7, &contract, raw, key);
                    if hit {
                        let val = {
                            let mut st = s7.lock().unwrap();
                            let v = st.storage.get(key).cloned();
                            let trie = trie_charge(&mut st, key);
                            let gas = &mock_cfg().gas;
                            let cost = gas.storage_read_base
                                + gas.storage_read_key_byte * kl as u64
                                + gas.storage_read_value_byte * v.as_ref().map(|x| x.len()).unwrap_or(0) as u64
                                + trie
                                + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * kl as u64
                                + crate::WRITE_REGISTER_BASE_GAS
                                + crate::WRITE_REGISTER_BYTE_GAS * v.as_ref().map(|x| x.len()).unwrap_or(0) as u64;
                            drop(st);
                            charge_gas(&mut caller, cost)?;
                            v
                        };
                        if let Some(val) = val {
                            let mut st = s7.lock().unwrap();
                            write_reg_checked(&mut st, rid, val)
                                .map_err(|e| wasmtime::Error::msg(e))?;
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    htrace!(
                        "  → storage_read not found [{}]",
                        String::from_utf8_lossy(key)
                    );
                    // production charges the read base + trie walk even on
                    // miss; guest key was still read → read_memory too
                    let gas = &mock_cfg().gas;
                    let trie = trie_charge(&mut st, key);
                    let cost = gas.storage_read_base + gas.storage_read_key_byte * kl as u64
                        + trie + crate::READ_MEMORY_BASE_GAS
                        + crate::READ_MEMORY_BYTE_GAS * kl as u64;
                    drop(st);
                    charge_gas(&mut caller, cost)?;
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
    let storage_remove_fn = host_fn(
        "storage_remove",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
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
                        // PV155 composite: read_memory(key) + storage_remove +
                        // trie + write_register(removed value)
                        let gas = &mock_cfg().gas;
                        let cost = gas.storage_remove_base
                            + gas.storage_remove_key_byte * kl as u64
                            + trie
                            + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * kl as u64
                            + crate::WRITE_REGISTER_BASE_GAS
                            + crate::WRITE_REGISTER_BYTE_GAS * val.len() as u64;
                        // Storage staking: refund the removed bytes
                        apply_staking_delta(
                            &mut s8.lock().unwrap(),
                            &exec_ctx_or_default().contract,
                            -((kl + val.len()) as i64),
                        );
                        charge_gas(&mut caller, cost)?;
                        if rid != u64::MAX {
                            let mut st = s8.lock().unwrap();
                            write_reg_checked(&mut st, rid, val)
                                .map_err(|e| wasmtime::Error::msg(e))?;
                        }
                        crate::fork_tombstone(&rkey);
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
    let storage_has_key_fn = host_fn(
        "storage_has_key",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
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
                    let (mut has, trie) = {
                        let mut st = s9.lock().unwrap();
                        (st.storage.contains_key(&hkey), trie_charge(&mut st, &hkey))
                    };
                    if !has && crate::fork_enabled() {
                        let contract = exec_ctx_or_default().contract;
                        let raw = &hkey[contract.len() + 1..];
                        has = crate::fork_page_in(&s9, &contract, raw, &hkey);
                    }
                    // PV155 composite: read_memory(key) + storage_has_key + trie
                    let gas = &mock_cfg().gas;
                    let cost = gas.storage_has_key_base
                        + gas.storage_has_key_key_byte * kl as u64
                        + trie
                        + crate::READ_MEMORY_BASE_GAS
                        + crate::READ_MEMORY_BYTE_GAS * kl as u64;
                    charge_gas(&mut caller, cost)?;
                    results[0] = Val::I64(if has { 1 } else { 0 });
                    return Ok(());
                }
            }
            results[0] = Val::I64(0);
            Ok(())
        },
    );

    let panic_fn = host_fn(
        "panic_utf8",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
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

    let abort_fn = host_fn(
        "panic",
        &mut *store,
        FuncType::new(engine, vec![], vec![]),
        |_, _, _| Err(wasmtime::Error::msg("ABORT")),
    );

    let s_ca = state.clone();
    let current_account_fn = host_fn(
        "current_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let acct = exec_ctx_or_default().contract;
            let acct = if acct.is_empty() {
                "escrow.test.near".to_string()
            } else {
                acct
            };
            charge_gas(
                &mut caller,
                crate::WRITE_REGISTER_BASE_GAS
                    + crate::WRITE_REGISTER_BYTE_GAS * acct.len() as u64,
            )?;
            s_ca.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, acct.into_bytes());
            Ok(())
        },
    );

    let s_sa = state.clone();
    let signer_account_fn = host_fn(
        "signer_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            // NEAR_MOCK_SIGNER overrides the tx signer — liquidation tests
            // need caller ≠ account owner (default stays owner.test.near).
            let signer = {
                let ctx = exec_ctx_or_default();
                if EXEC_CTX.with(|c| c.borrow().is_some()) {
                    ctx.signer
                } else {
                    std::env::var("NEAR_MOCK_SIGNER").unwrap_or_else(|_| "owner.test.near".into())
                }
            };
            charge_gas(
                &mut caller,
                crate::WRITE_REGISTER_BASE_GAS
                    + crate::WRITE_REGISTER_BYTE_GAS * signer.len() as u64,
            )?;
            s_sa.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, signer.into_bytes());
            Ok(())
        },
    );

    let s_pa = state.clone();
    let predecessor_account_fn = host_fn(
        "predecessor_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let pred = {
                let ctx = exec_ctx_or_default();
                if EXEC_CTX.with(|c| c.borrow().is_some()) {
                    ctx.predecessor
                } else {
                    "owner.test.near".to_string()
                }
            };
            charge_gas(
                &mut caller,
                crate::WRITE_REGISTER_BASE_GAS
                    + crate::WRITE_REGISTER_BYTE_GAS * pred.len() as u64,
            )?;
            s_pa.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, pred.into_bytes());
            Ok(())
        },
    );

    let s_pk = state.clone();
    let signer_pk_fn = host_fn(
        "signer_account_pk",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            charge_gas(
                &mut caller,
                crate::WRITE_REGISTER_BASE_GAS + crate::WRITE_REGISTER_BYTE_GAS * 51,
            )?;
            s_pk.lock().unwrap().registers.insert(
                args[0].unwrap_i64() as u64,
                b"ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_vec(),
            );
            Ok(())
        },
    );

    let block_ts_fn = host_fn(
        "block_timestamp",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(
                // NEAR host returns NANoseconds — mock must match the real
                // scale (was millis: silent 1e6x unit divergence).
                // Precedence: NEAR_MOCK_BLOCK_TS (exact ns pin) > --now/--advance
                // (deterministic seconds base + travel) > real clock.
                std::env::var("NEAR_MOCK_BLOCK_TS")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or_else(mock_now_nanos),
            );
            Ok(())
        },
    );

    let _s_ab = state.clone();
    let account_balance_fn = host_fn(
        "account_balance",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
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
            // --staking: liquid balance excludes the storage-staked amount
            let amt = match STATE_ARC.with(|s| s.borrow().clone()) {
                Some(st) => {
                    let guard = st.lock().unwrap();
                    amt.saturating_sub(locked_balance_for(&guard, &contract))
                }
                None => amt,
            };
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
    let attached_deposit_fn = host_fn(
        "attached_deposit",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let ptr = args[0].unwrap_i64() as usize;
            // Real host shape: 16 LE bytes of THIS receipt's deposit.
            // CURRENT_DEPOSIT is set by execute_tx for the top-level entry
            // (from --attach/--deposit/NEAR_MOCK_ATTACH, ≥0.1.7) and by
            // sub_execute for promise children. The env var remains as the
            // single-wasm runner's fallback. (Was always 0 for cross/call
            // flag runs: the auction protocol reads it, and value-receiving
            // entries silently saw nothing. 2026-09-01; entry wiring 0.1.7.)
            let amt: u128 = CURRENT_DEPOSIT
                .with(|d| *d.borrow())
                .or_else(|| {
                    std::env::var("NEAR_MOCK_ATTACH")
                        .ok()
                        .and_then(|s| s.trim().parse().ok())
                })
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
    let used_gas_fn = host_fn(
        "used_gas",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        move |mut caller, _, results| {
            let remaining = caller
                .get_export(crate::REMAINING_GAS_EXPORT)
                .and_then(|e| e.into_global())
                .map(|g| match g.get(&mut caller) {
                    wasmtime::Val::I64(v) => u64::try_from(v).unwrap_or(0),
                    _ => 0,
                })
                .unwrap_or(0u64);
            results[0] =
                Val::I64(PREPAID_FUEL.with(|f| *f.borrow()).saturating_sub(remaining) as i64);
            Ok(())
        },
    );
    let prepaid_gas_fn = host_fn(
        "prepaid_gas",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        move |_, _, results| {
            results[0] = Val::I64(PREPAID_FUEL.with(|f| *f.borrow()) as i64);
            Ok(())
        },
    );

    // sha256(len, ptr, rid) — real digest to register (was noop)
    let sg1 = state.clone();
    let sha256_fn = host_fn(
        "sha256",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use sha2::{Digest, Sha256};
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Indicative legacy fees
            let cost = crate::SHA256_BASE_GAS
                + crate::SHA256_BYTE_GAS * len as u64
                + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * len as u64
                + crate::WRITE_REGISTER_BASE_GAS + crate::WRITE_REGISTER_BYTE_GAS * 32;
            charge_gas(&mut caller, cost)?;
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
    let keccak256_fn = host_fn(
        "keccak256",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use sha3::Keccak256;
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // PV155 (protocol-86)
            let cost = crate::KECCAK256_BASE_GAS
                + crate::KECCAK256_BYTE_GAS * len as u64
                + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * len as u64
                + crate::WRITE_REGISTER_BASE_GAS + crate::WRITE_REGISTER_BYTE_GAS * 32;
            charge_gas(&mut caller, cost)?;
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
    let write_register_fn = host_fn(
        "write_register",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            // Indicative legacy fees
            let cost = crate::KECCAK512_BASE_GAS
                + crate::KECCAK512_BYTE_GAS * len as u64
                + crate::READ_MEMORY_BASE_GAS + crate::READ_MEMORY_BYTE_GAS * len as u64
                + crate::WRITE_REGISTER_BASE_GAS + crate::WRITE_REGISTER_BYTE_GAS * 64;
            charge_gas(&mut caller, cost)?;
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
    // All digest hosts follow the (len, ptr, rid) register ABI.
    // keccak512: REAL Keccak-512 (tiny-keccak — the pre-standard variant,
    // matching nearcore) — the SHAKE128 stand-in is gone.
    let sg_keccak512 = state.clone();
    let keccak512_fn = host_fn(
        "keccak512",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            use tiny_keccak::Hasher;
            let (len, ptr, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if ptr + len <= md.len() {
                    let mut h = tiny_keccak::Keccak::v512();
                    h.update(&md[ptr..ptr + len]);
                    let mut digest = vec![0u8; 64];
                    h.finalize(&mut digest);
                    let mut st = sg_keccak512.lock().unwrap();
                    write_reg_checked(&mut st, rid, digest).map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    let sg_ripemd = state.clone();
    let ripemd160_fn = host_fn(
        "ripemd160",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
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
                    write_reg_checked(&mut st, rid, digest).map_err(|e| wasmtime::Error::msg(e))?;
                }
            }
            Ok(())
        },
    );
    // p256_verify(sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) -> u64
    // REAL (2026-09-08): RustCrypto p256, prehash verify — nearcore semantics
    // (wasmtime_runner/logic.rs p256_verify): sig/pk length violations are
    // host errors (trap); unparsable sig/key or bad verify → 0.
    let p256_fn = host_fn(
        "p256_verify",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 6], vec![ValType::I64]),
        move |mut caller, args, results| {
            let gas = &mock_cfg().gas;
            charge_gas(&mut caller, gas.p256_verify_base)?;
            let (sl, sp, ml, mp, kl, kp) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
                args[4].unwrap_i64() as usize,
                args[5].unwrap_i64() as usize,
            );
            let mut read = |len: usize, ptr: usize| -> Option<Vec<u8>> {
                let mem = caller.get_export("memory")?.into_memory()?;
                let md = mem.data(&caller);
                md.get(ptr..ptr.checked_add(len)?).map(|s| s.to_vec())
            };
            let ok = (|| -> Result<bool, String> {
                let sig = read(sl, sp).ok_or("MemoryAccessViolation: p256 sig")?;
                let msg = read(ml, mp).ok_or("MemoryAccessViolation: p256 msg")?;
                let pk = read(kl, kp).ok_or("MemoryAccessViolation: p256 pk")?;
                crypto_real::p256_verify(&sig, &msg, &pk)
            })();
            results[0] = match ok {
                Err(e) => return Err(wasmtime::Error::msg(e)),
                Ok(b) => Val::I64(b as i64),
            };
            Ok(())
        },
    );
    // ecrecover(n, ptr, m, ptr, s, ptr, register_id) -> u64 — REAL k256
    // recovery of the 65-byte uncompressed pubkey into the register.
    // nearcore convention: check_signature_values fail → 0 (no register
    // write); recover fail → 0; success → register + 1. Length/flag/malformed
    // input violations are host errors (trap), matching nearcore.
    let sg_ec = state.clone();
    let ecrecover_fn = host_fn(
        "ecrecover",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![ValType::I64]),
        move |mut caller, args, results| {
            let gas = &mock_cfg().gas;
            charge_gas(&mut caller, gas.ecrecover_base)?;
            let (hl, hp, sl, sp, ml, mp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
                args[4].unwrap_i64() as usize,
                args[5].unwrap_i64() as usize,
                args[6].unwrap_i64() as u64,
            );
            let mut read = |len: usize, ptr: usize| -> Option<Vec<u8>> {
                let mem = caller.get_export("memory")?.into_memory()?;
                let md = mem.data(&caller);
                md.get(ptr..ptr.checked_add(len)?).map(|s| s.to_vec())
            };
            let outcome = (|| -> Result<Option<Vec<u8>>, String> {
                let hash =
                    read(hl, hp).ok_or("ECRecoverError: MemoryAccessViolation reading hash")?;
                let sig = read(sl, sp)
                    .ok_or("ECRecoverError: MemoryAccessViolation reading signature")?;
                let v = ml as u64; // 7-arg ABI packs v in the msg-len slot
                let _ = mp;
                Ok(crypto_real::ecrecover(&hash, &sig, v, (mp & 1) as u64)?.map(|pk| pk.to_vec()))
            })();
            results[0] = match outcome {
                Err(e) => return Err(wasmtime::Error::msg(e)),
                Ok(None) => Val::I64(0),
                Ok(Some(pk65)) => {
                    let mut st = sg_ec.lock().unwrap();
                    write_reg_checked(&mut st, rid, pk65).map_err(|e| wasmtime::Error::msg(e))?;
                    Val::I64(1)
                }
            };
            Ok(())
        },
    );
    // alt_bn128 precompiles — REAL BN254 math (bn254.rs, zeropool-bn 0.5.11:
    // the exact crate the vendored near-vm-runner pins). ABI from nearcore
    // imports.rs: g1_sum/g1_multiexp are (value_len, value_ptr, register_id)
    // with NO return. Per-element costs use the PV155 fee table.
    let sg_sum = state.clone();
    let g1_sum_fn = host_fn(
        "alt_bn128_g1_sum",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let gas = &mock_cfg().gas;
            let (len, ptr, rid) = (
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                args[2].unwrap_i64() as u64,
            );
            let data = read_guest_bytes(&mut caller, len, ptr)
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: alt_bn128_g1_sum"))?;
            let n = (data.len() / bn254::G1_SUM_ELEMENT_SIZE) as u64;
            charge_gas(
                &mut caller,
                gas.alt_bn128_g1_sum_base + gas.alt_bn128_g1_sum_element * n,
            )?;
            let elems = bn254::split_elements::<{ bn254::G1_SUM_ELEMENT_SIZE }>(&data)
                .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            let out = bn254::g1_sum(elems).map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            let mut st = sg_sum.lock().unwrap();
            write_reg_checked(&mut st, rid, out.to_vec()).map_err(|e| wasmtime::Error::msg(e))?;
            Ok(())
        },
    );
    let sg_mx = state.clone();
    let g1_multiexp_fn = host_fn(
        "alt_bn128_g1_multiexp",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let gas = &mock_cfg().gas;
            let (len, ptr, rid) = (
                args[0].unwrap_i64(),
                args[1].unwrap_i64(),
                args[2].unwrap_i64() as u64,
            );
            let data = read_guest_bytes(&mut caller, len, ptr).ok_or_else(|| {
                wasmtime::Error::msg("MemoryAccessViolation: alt_bn128_g1_multiexp")
            })?;
            let n = (data.len() / bn254::G1_MULTIEXP_ELEMENT_SIZE) as u64;
            charge_gas(
                &mut caller,
                gas.alt_bn128_g1_multiexp_base + gas.alt_bn128_g1_multiexp_element * n,
            )?;
            let elems = bn254::split_elements::<{ bn254::G1_MULTIEXP_ELEMENT_SIZE }>(&data)
                .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            let out = bn254::g1_multiexp(elems).map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            let mut st = sg_mx.lock().unwrap();
            write_reg_checked(&mut st, rid, out.to_vec()).map_err(|e| wasmtime::Error::msg(e))?;
            Ok(())
        },
    );
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
        bls_fns.push(host_fn(
            _nm,
            &mut *store,
            FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
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
    let bls_pairing_fn = host_fn(
        "bls12381_pairing_check",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
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
        FuncType::new(engine, vec![ValType::I64], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop0r = Func::new(
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_2i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_3i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_3i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_2i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_4i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_6i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_7i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_7i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 7], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_8i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 8], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_9i = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![]),
        |_, _, _| Ok(()),
    );
    let _noop_4i_i32 = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![ValType::I32]),
        |_, _, r| {
            r[0] = Val::I32(0);
            Ok(())
        },
    );
    let _noop_4i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_8i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 8], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let _noop_9i_1o = Func::new(
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );

    // === Link ===
    // max = 2048 pages (128 MiB) — mainnet's memory ceiling (parity audit
    // 2026-09-10); unbounded before.
    let env_memory = wasmtime::Memory::new(
        &mut *store,
        wasmtime::MemoryType::new(1024, Some(crate::MAX_MEMORY_PAGES as u32)),
    )?;
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
    let block_index_fn = host_fn(
        "block_index",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
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
    let random_seed_fn = host_fn(
        "random_seed",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            charge_gas(
                &mut caller,
                crate::WRITE_REGISTER_BASE_GAS + crate::WRITE_REGISTER_BYTE_GAS * 32,
            )?;
            // Entropy source, in priority order:
            //   1. NEAR_MOCK_SEED pin (explicit reproducibility)
            //   2. --now / NEAR_MOCK_BLOCK_HEIGHT pin → seed = SplitMix64 of
            //      (height, ts): fully deterministic, yet differs per block —
            //      matches real NEAR's per-block random_seed semantics.
            //   3. Wall clock ^ pid (real-ish entropy for non-pinned runs).
            let height = std::env::var("NEAR_MOCK_BLOCK_HEIGHT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let pinned_ts = mock_cfg().base_ts.map(|t| t as u64);
            let mut z = match (pinned_ts, std::env::var("NEAR_MOCK_SEED").ok()) {
                (Some(ts), None) => {
                    let mut h = height;
                    splitmix64(&mut h)
                        ^ splitmix64(&mut {
                            let t = ts;
                            t
                        })
                }
                _ => {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0x5EED)
                        ^ ((std::process::id() as u64) << 32)
                }
            };
            let seed: Vec<u8> = (0..4)
                .flat_map(|_| splitmix64(&mut z).to_le_bytes())
                .collect();
            let hex: String = match std::env::var("NEAR_MOCK_SEED") {
                Ok(pin) => format!("{:0<64}", pin.trim()),
                _ => seed.iter().map(|b| format!("{b:02x}")).collect(),
            };
            let mut st = rs1.lock().unwrap();
            write_reg_checked(&mut st, rid, hex.into_bytes())
                .map_err(|e| wasmtime::Error::msg(e))?;
            Ok(())
        },
    );
    linker.define(&*store, "env", "random_seed", random_seed_fn)?;
    linker.define(&*store, "env", "sha256", sha256_fn)?;
    // ── finite-wasm instrumentation hooks (module "internal") ──
    // The instrumented code traps by calling one of these on exhaustion.
    // Gas-exhausted must error with mainnet's receipt-level message so the
    // differ's same-class comparison matches "Exceeded the prepaid gas".
    {
        let exhausted_ty = FuncType::new(engine, vec![], vec![]);
        let gas_fn = host_fn(
            "finite_wasm_gas_exhausted",
            &mut *store,
            exhausted_ty.clone(),
            |_, _, _| Err(wasmtime::Error::msg("Exceeded the prepaid gas")),
        );
        let stack_fn = host_fn(
            "finite_wasm_stack_exhausted",
            &mut *store,
            exhausted_ty,
            |_, _, _| Err(wasmtime::Error::msg("WasmTrap: StackOverflow")),
        );
        let report_ty = FuncType::new(engine, vec![ValType::I64], vec![]);
        let report_fn = host_fn("finite_wasm_gas", &mut *store, report_ty, |_, _, _| {
            Err(wasmtime::Error::msg("Exceeded the prepaid gas"))
        });
        linker.define(&*store, "internal", "finite_wasm_gas_exhausted", gas_fn)?;
        linker.define(&*store, "internal", "finite_wasm_stack_exhausted", stack_fn)?;
        linker.define(&*store, "internal", "finite_wasm_gas", report_fn)?;
    }
    // schnorr_verify_bip340(pk_ptr: i32, sig_ptr: i32, msg_ptr: i32, msg_len: i32) -> i32
    let schnorr_fn = host_fn(
        "schnorr_verify_bip340",
        &mut *store,
        FuncType::new(engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |caller, params, results| {
            let pk_ptr = params[0].unwrap_i32() as usize;
            let sig_ptr = params[1].unwrap_i32() as usize;
            let msg_ptr = params[2].unwrap_i32() as usize;
            let msg_len = params[3].unwrap_i32() as usize;

            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("missing memory export");
            let data = mem.data(&caller);

            if mock_cfg().debug {
                htrace!("[schnorr-dbg] entry pk_ptr={pk_ptr} sig_ptr={sig_ptr} msg_ptr={msg_ptr} msg_len={msg_len} mem_len={}", data.len());
            }
            if pk_ptr + 32 > data.len()
                || sig_ptr + 64 > data.len()
                || msg_ptr + msg_len > data.len()
            {
                if mock_cfg().debug {
                    htrace!("[schnorr-dbg] BOUNDS REJECT");
                }
                results[0] = Val::I32(0);
                return Ok(());
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let pk: [u8; 32] = data[pk_ptr..pk_ptr+32].try_into().unwrap();
                let sig: [u8; 64] = data[sig_ptr..sig_ptr+64].try_into().unwrap();
                let msg = &data[msg_ptr..msg_ptr+msg_len];
                if mock_cfg().debug {
                    htrace!("[schnorr-dbg] pk_ptr={pk_ptr} sig_ptr={sig_ptr} msg_ptr={msg_ptr} msg_len={msg_len}");
                }
                let r = schnorr_verify_impl(&pk, &sig, msg) as i32;
                if mock_cfg().debug {
                    htrace!("[schnorr-dbg] result={r}");
                }
                r
            }))
            .unwrap_or_else(|_| {
                if mock_cfg().debug {
                    htrace!("[schnorr-dbg] PANIC");
                }
                0
            });

            results[0] = Val::I32(result);
            Ok(())
        },
    );
    linker.define(&*store, "env", "schnorr_verify_bip340", schnorr_fn)?;
    // ed25519_verify — real host ABI: (sig_len: i64, sig_ptr: i64,
    // msg_len: i64, msg_ptr: i64, pk_len: i64, pk_ptr: i64) -> i64 (1/0).
    // Signature is 64 bytes (R||s), pk 32 bytes; pk_len/sig_len must match
    // or reject, mirroring VMLogic's length checks.
    let ed25519_fn = host_fn(
        "ed25519_verify",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |caller, params, results| {
            let sig_len = params[0].unwrap_i64() as usize;
            let sig_ptr = params[1].unwrap_i64() as usize;
            let msg_len = params[2].unwrap_i64() as usize;
            let msg_ptr = params[3].unwrap_i64() as usize;
            let pk_len = params[4].unwrap_i64() as usize;
            let pk_ptr = params[5].unwrap_i64() as usize;
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("missing memory export");
            let data = mem.data(&caller);
            if pk_len != 32
                || sig_len != 64
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
    let log_utf16_fn = host_fn(
        "log_utf16",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            let cost = mock_cfg().gas.log_base + mock_cfg().gas.log_byte * len as u64;
            charge_gas(&mut caller, cost)?;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let units: Vec<u16> = data[ptr..ptr + len]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let msg = String::from_utf16_lossy(&units);
                    handle_log_line(
                        &msg,
                        mock_cfg().debug,
                        &format!("  [debug len={len} ptr={ptr}] (utf16)"),
                    );
                } else {
                    println!(
                        "  LOG: <out-of-range> [debug len={} ptr={}] (utf16)",
                        len, ptr
                    );
                }
            }
            Ok(())
        },
    );
    linker.define(&*store, "env", "log_utf16", log_utf16_fn)?;
    linker.define(&*store, "env", "keccak256", keccak256_fn)?;
    // `log`/`log_s`/`validator_account_id` are REMOVED from the NEAR host
    // surface (never in near-vm-runner's import list at 0.37.x). Real runner:
    // unknown import = instantiating error. Bind loud Deprecated traps so a
    // hand-built wasm calling them fails here exactly like on-chain, instead
    // of succeeding silently (the weighted-promise lesson: silent stubs hide
    // drift for weeks).
    let deprecated_log = host_fn(
        "log",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        |_, _, _| {
            Err(wasmtime::Error::msg(
                "Deprecated { method_name: \"log\" } — removed from the NEAR host surface; use log_utf8",
            ))
        },
    );
    linker.define(&*store, "env", "log", deprecated_log.clone())?;
    linker.define(&*store, "env", "log_s", deprecated_log)?;
    let deprecated_vaid = host_fn(
        "validator_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        |_, _, _| {
            Err(wasmtime::Error::msg(
                "Deprecated { method_name: \"validator_account_id\" } — removed from the NEAR host surface",
            ))
        },
    );
    linker.define(&*store, "env", "validator_account_id", deprecated_vaid)?;
    // === Real validator hosts (nearcore semantics): stake lookups come from
    // the validator map in state storage under "\x00validators" — JSON map
    // {account_id: yocto_stake_string}, plus "\x00validators:total" for the
    // epoch total. Seeded ONCE at chain genesis (install_sandbox), not here:
    // a per-linker-build insert happens inside the tx window, so the key
    // survived trap rollbacks as a phantom (chain_api test caught it
    // 2026-09-08). Unseeded → account not a validator (u128 0) / total 0.
    let vs0 = state.clone();
    let vs_engine = engine.clone();
    let validator_stake_fn = host_fn(
        "validator_stake",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let gas = &mock_cfg().gas;
            charge_gas(&mut caller, gas.validator_stake_base)?;
            let (a_len, a_ptr, stake_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let acct =
                super::mem_read_str(&mut caller, a_len as i64, a_ptr as i64).ok_or_else(|| {
                    wasmtime::Error::msg("MemoryAccessViolation: validator_stake account")
                })?;
            let map: HashMap<String, String> = {
                let st = vs0.lock().unwrap();
                st.storage
                    .get(b"\x00validators".as_slice())
                    .and_then(|v| serde_json::from_slice(v).ok())
                    .unwrap_or_default()
            };
            let stake_yocto: u128 = map.get(&acct).and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data_mut(&mut caller);
                if stake_ptr + 16 <= md.len() {
                    md[stake_ptr..stake_ptr + 16].copy_from_slice(&stake_yocto.to_le_bytes());
                } else {
                    return Err(wasmtime::Error::msg(
                        "MemoryAccessViolation: validator_stake stake_ptr",
                    ));
                }
            }
            htrace!("  → validator_stake({}) = {} yocto", acct, stake_yocto);
            Ok(())
        },
    );
    let _ = vs_engine;
    linker.define(&*store, "env", "validator_stake", validator_stake_fn)?;
    let vts0 = state.clone();
    let validator_total_stake_fn = host_fn(
        "validator_total_stake",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let gas = &mock_cfg().gas;
            charge_gas(&mut caller, gas.validator_total_stake_base)?;
            let ptr = args[0].unwrap_i64() as usize;
            let total: u128 = {
                let st = vts0.lock().unwrap();
                st.storage
                    .get(b"\x00validators:total".as_slice())
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
                        // default when unseeded: sum the map if present
                        let map: HashMap<String, String> = st
                            .storage
                            .get(b"\x00validators".as_slice())
                            .and_then(|v| serde_json::from_slice(v).ok())
                            .unwrap_or_default();
                        map.values()
                            .filter_map(|s| s.parse::<u128>().ok())
                            .sum::<u128>()
                    })
            };
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data_mut(&mut caller);
                if ptr + 16 <= md.len() {
                    md[ptr..ptr + 16].copy_from_slice(&total.to_le_bytes());
                } else {
                    return Err(wasmtime::Error::msg(
                        "MemoryAccessViolation: validator_total_stake ptr",
                    ));
                }
            }
            htrace!("  → validator_total_stake() = {} yocto", total);
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "validator_total_stake",
        validator_total_stake_fn,
    )?;
    linker.define(&*store, "env", "alt_bn128_g1_multiexp", g1_multiexp_fn)?;
    linker.define(&*store, "env", "alt_bn128_g1_sum", g1_sum_fn)?;
    // alt_bn128_pairing_check(value_len, value_ptr) -> u64 — REAL pairing;
    // 1 = the product of pairings equals the GT identity (nearcore semantic),
    // malformed input is a host error (trap).
    let pairing_fn = host_fn(
        "alt_bn128_pairing_check",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let gas = &mock_cfg().gas;
            let (len, ptr) = (args[0].unwrap_i64(), args[1].unwrap_i64());
            let data = read_guest_bytes(&mut caller, len, ptr).ok_or_else(|| {
                wasmtime::Error::msg("MemoryAccessViolation: alt_bn128_pairing_check")
            })?;
            let n = (data.len() / bn254::PAIRING_CHECK_ELEMENT_SIZE) as u64;
            charge_gas(
                &mut caller,
                gas.alt_bn128_pairing_check_base + gas.alt_bn128_pairing_check_element * n,
            )?;
            let elems = bn254::split_elements::<{ bn254::PAIRING_CHECK_ELEMENT_SIZE }>(&data)
                .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            let holds =
                bn254::pairing_check(elems).map_err(|e| wasmtime::Error::msg(e.to_string()))?;
            results[0] = Val::I64(holds as i64);
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

    // epoch_height() -> u64: real chain-level counter — NEAR_MOCK_EPOCH pin
    // wins, else derived from the mock clock (epochs ≈ 43_200 s so --now/
    // --advance scenarios advance epochs consistently with block_timestamp).
    // Was a silent 0: epoch-conditioned logic (Burrow distribution windows,
    // staking APR) could never be exercised.
    let epoch_height_fn = host_fn(
        "epoch_height",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(mock_epoch_height() as i64);
            Ok(())
        },
    );
    linker.define(&*store, "env", "epoch_height", epoch_height_fn)?;
    // storage_usage() -> u64: bytes used by THIS contract's namespace
    // (was a silent 0 — flashpool-style checks saw free storage forever).
    let su0 = state.clone();
    let storage_usage_fn = host_fn(
        "storage_usage",
        &mut *store,
        FuncType::new(engine, vec![], vec![ValType::I64]),
        move |_, _, r| {
            let contract = exec_ctx_or_default().contract;
            let bytes: u64 = su0
                .lock()
                .unwrap()
                .storage
                .iter()
                .filter(|(k, _)| k.starts_with(&prefixed_key(&contract, b"")))
                .map(|(k, v)| (k.len() + v.len()) as u64)
                .sum();
            r[0] = Val::I64(bytes as i64);
            Ok(())
        },
    );
    linker.define(&*store, "env", "storage_usage", storage_usage_fn)?;
    // current_contract_code(register_id) — protocol 69 code introspection:
    // near-contract-standard binaries import it unconditionally (link-time
    // requirement even for views). The mock returns the CURRENT module's
    // bytes from MODULES via EXEC_CTX.contract when available (faithful for
    // hash checks), else an empty write — never a silent success: callers
    // can read register 0 length to detect the mock's answer.
    let ccc_st = state.clone();
    let current_contract_code_fn = host_fn(
        "current_contract_code",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![ValType::I64]),
        move |_, args, results| {
            let rid = args[0].unwrap_i64() as u64;
            let acct = exec_ctx_or_default().contract;
            // MODULES holds Rc<Module>; as_binary() gives the compiled form,
            // not original bytes — emitting an empty register is the honest
            // mock answer for "the code bytes" (hash users see len=0).
            let _ = acct;
            let mut st = ccc_st.lock().unwrap();
            write_reg_checked(&mut st, rid, Vec::new()).map_err(|e| wasmtime::Error::msg(e))?;
            htrace!("  → current_contract_code(reg={rid}) → empty (mock: code bytes not modeled)");
            results[0] = wasmtime::Val::I64(0);
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "current_contract_code",
        current_contract_code_fn,
    )?;
    // chain_id(register_id) — protocol 69 chain identification: writes
    // "testnet"/"mainnet" (or the genesis hash on other chains). The mock
    // picks testnet by default — matches where these rehearsal snapshots
    // come from — overridable via NEAR_MOCK_CHAIN_ID for mainnet fixtures.
    let cid_st = state.clone();
    let chain_id_fn = host_fn(
        "chain_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            let chain = std::env::var("NEAR_MOCK_CHAIN_ID").unwrap_or_else(|_| "testnet".into());
            let mut st = cid_st.lock().unwrap();
            write_reg_checked(&mut st, rid, chain.clone().into_bytes())
                .map_err(|e| wasmtime::Error::msg(e))?;
            htrace!("  → chain_id(reg={rid}) → \"{chain}\" (NEAR_MOCK_CHAIN_ID to override)");
            Ok(())
        },
    );
    linker.define(&*store, "env", "chain_id", chain_id_fn)?;
    // (log_s / validator_account_id are bound to Deprecated traps at ~1373 —
    // no second bind here, wasmtime rejects duplicate import definitions.)
    linker.define(&*store, "env", "promise_results", noop1.clone())?;
    // (yield hosts defined below — cross engine or noop, never twice)
    // account_locked_balance(balance_ptr): 16-byte u128 write of the
    // storage-staked amount (was a silent 0).
    let alb0 = state.clone();
    let account_locked_balance_fn = host_fn(
        "account_locked_balance",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64], vec![]),
        move |mut caller, args, _| {
            let contract = exec_ctx_or_default().contract;
            let amt = match alb0.try_lock() {
                Ok(g) => locked_balance_for(&g, &contract),
                Err(_) => 0u128,
            };
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
    linker.define(
        &*store,
        "env",
        "account_locked_balance",
        account_locked_balance_fn,
    )?;
    // storage_iter_* — REMOVED from the NEAR host surface (deprecated at the
    // protocol level since iterator removal; near-vm-runner 0.37.3 registers
    // them ONLY to return HostError::Deprecated — vendored
    // wasmtime_runner/logic.rs storage_iter_prefix). The mock mirrors that:
    // any call is a loud trap, never silent iteration.
    let iter_prefix_fn = host_fn(
        "storage_iter_prefix",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 2], vec![ValType::I64]),
        |_, _, _| {
            Err(wasmtime::Error::msg(
                "Attempted to call deprecated host function storage_iter_prefix",
            ))
        },
    );
    let iter_range_fn = host_fn(
        "storage_iter_range",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![ValType::I64]),
        |_, _, _| {
            Err(wasmtime::Error::msg(
                "Attempted to call deprecated host function storage_iter_range",
            ))
        },
    );
    let iter_next_fn = host_fn(
        "storage_iter_next",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![ValType::I64]),
        |_, _, _| {
            Err(wasmtime::Error::msg(
                "Attempted to call deprecated host function storage_iter_next",
            ))
        },
    );
    linker.define(&*store, "env", "storage_iter_prefix", iter_prefix_fn)?;
    linker.define(&*store, "env", "storage_iter_range", iter_range_fn)?;
    linker.define(&*store, "env", "storage_iter_next", iter_next_fn)?;
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

    // 43b promise_batch_action_function_call_weight(idx, m_len, m_ptr,
    // a_len, a_ptr, dep_ptr, gas, weight_ptr) — same action as the plain
    // function_call, plus a GasWeight (u64 at weight_ptr). 2026-09-07:
    // near-sdk's PromiseAnd / attached-gas paths emit the weighted import;
    // the old noop stub silently dropped EVERY action (empty receipt
    // batches — burrow's pyth query + ref swap never executed). Record the
    // action identically: gas is prepaid and the weight only splits unused
    // gas on-chain, which the mock ignores.
    let pafcw = host_fn(
        "promise_batch_action_function_call_weight",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 8], vec![]),
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
            htrace!(
                "  → action_fn_call_weight(idx={}, {} args={} dep={})",
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

    // Shadow the noop stub with the real weighted action host.
    linker.define(
        &*store,
        "env",
        "promise_batch_action_function_call_weight",
        pafcw,
    )?;

    // Batch key/account actions — REAL semantics (2026-09-08), replacing
    // silent noops. Arities from nearcore imports.rs:255-288. Like FnCall/
    // Transfer, each host RECORDS its action on the DAG node; effects apply
    // at drain time (execute_promise) so failed receipts revert atomically.
    // Key model: ED25519 only (32B raw pk), matching the mock's signer model
    // — other curves trap loudly, never silently succeed.
    let stake_fn = host_fn(
        "promise_batch_action_stake",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![]),
        move |mut caller, args, _| {
            let (idx, amt_ptr, pk_len, pk_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: stake"))?;
            let md = mem.data(&caller);
            if amt_ptr + 16 > md.len() || pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg("MemoryAccessViolation: stake"));
            }
            let amt = u128::from_le_bytes(md[amt_ptr..amt_ptr + 16].try_into().unwrap());
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            if pk[0] != 0xED {
                return Err(wasmtime::Error::msg(
                    "StakeInvalidKey: only ED25519 (0xED-prefixed) validator keys supported",
                ));
            }
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::Stake { amount: amt });
                } else {
                    panic!("InvalidPromiseIndex: stake");
                }
            });
            htrace!("  ↗ stake {amt} yocto (pk ed25519 33B)");
            Ok(())
        },
    );
    let add_key_full_fn = host_fn(
        "promise_batch_action_add_key_with_full_access",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![]),
        move |mut caller, args, _| {
            let (idx, pk_len, pk_ptr, _nonce) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64(),
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: add_key"))?;
            let md = mem.data(&caller);
            if pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg("MemoryAccessViolation: add_key"));
            }
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            if pk[0] != 0xED {
                return Err(wasmtime::Error::msg(
                    "AddKeyInvalidKey: only ED25519 (0xED-prefixed) keys supported",
                ));
            }
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::AddKey { pk });
                } else {
                    panic!("InvalidPromiseIndex: add_key");
                }
            });
            htrace!("  🔑 add_key(full-access, ed25519 33B)");
            Ok(())
        },
    );
    let add_key_fc_fn = host_fn(
        "promise_batch_action_add_key_with_function_call",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![]),
        move |mut caller, args, _| {
            let (idx, pk_len, pk_ptr, _nonce, _allow_ptr, _rcv_len, _rcv_ptr, _mns_len, _mns_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64(),
                args[4].unwrap_i64() as usize,
                args[5].unwrap_i64() as usize,
                args[6].unwrap_i64() as usize,
                args[7].unwrap_i64() as usize,
                args[8].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: add_key_fc"))?;
            let md = mem.data(&caller);
            if pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg("MemoryAccessViolation: add_key_fc"));
            }
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            if pk[0] != 0xED {
                return Err(wasmtime::Error::msg(
                    "AddKeyInvalidKey: only ED25519 (0xED-prefixed) keys supported",
                ));
            }
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::AddKey { pk });
                } else {
                    panic!("InvalidPromiseIndex: add_key_fc");
                }
            });
            htrace!("  🔑 add_key(function-call, ed25519 33B; ACL recorded as full)");
            Ok(())
        },
    );
    let delete_key_fn = host_fn(
        "promise_batch_action_delete_key",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (idx, pk_len, pk_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: delete_key"))?;
            let md = mem.data(&caller);
            if pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg("MemoryAccessViolation: delete_key"));
            }
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::DeleteKey { pk });
                } else {
                    panic!("InvalidPromiseIndex: delete_key");
                }
            });
            htrace!("  🗑 delete_key(ed25519 33B)");
            Ok(())
        },
    );
    let delete_account_fn = host_fn(
        "promise_batch_action_delete_account",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (idx, ben_len, ben_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: delete_account"))?;
            let md = mem.data(&caller);
            if ben_ptr + ben_len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: delete_account",
                ));
            }
            let beneficiary = String::from_utf8_lossy(&md[ben_ptr..ben_ptr + ben_len]).to_string();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::DeleteAccount {
                        beneficiary: beneficiary.clone(),
                    });
                } else {
                    panic!("InvalidPromiseIndex: delete_account");
                }
            });
            htrace!("  💥 delete_account → beneficiary {beneficiary}");
            Ok(())
        },
    );
    linker.define(&*store, "env", "promise_batch_action_stake", stake_fn)?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_key_with_full_access",
        add_key_full_fn,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_key_with_function_call",
        add_key_fc_fn,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_delete_key",
        delete_key_fn,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_delete_account",
        delete_account_fn,
    )?;

    // ── Protocol 69/72-era hosts (near-sdk 5 binaries import these at link
    // time even when never called — wasmtime requires every import bound, so
    // views on such contracts CRASHED at instantiation before these existed;
    // dogfooded against susuplus.susumi.testnet, 2026-09-08). They record
    // into the promise DAG like the other batch actions; drain-time behavior
    // is documented on the PAction variants.
    let t2gk = host_fn(
        "promise_batch_action_transfer_to_gas_key",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![]),
        move |mut caller, args, _| {
            let (idx, key_len, key_ptr, amt_ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| {
                    wasmtime::Error::msg("MemoryAccessViolation: transfer_to_gas_key")
                })?;
            let md = mem.data(&caller);
            if key_ptr + key_len > md.len() || amt_ptr + 16 > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: transfer_to_gas_key",
                ));
            }
            let _key = &md[key_ptr..key_ptr + key_len]; // implicit account derivation not modeled
            let amt = u128::from_le_bytes(md[amt_ptr..amt_ptr + 16].try_into().unwrap());
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::TransferToGasKey { amount: amt });
                } else {
                    panic!("InvalidPromiseIndex: transfer_to_gas_key");
                }
            });
            htrace!("  ↗ transfer_to_gas_key {amt} yocto");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_transfer_to_gas_key",
        t2gk,
    )?;

    // add_gas_key_*: same shape as add_key_* (ED25519 33B pk), recorded as
    // AddGasKey — the mock applies no gas-key rules.
    let agk_full = host_fn(
        "promise_batch_action_add_gas_key_with_full_access",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 4], vec![]),
        move |mut caller, args, _| {
            let (idx, pk_len, pk_ptr, _nonce) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64(),
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: add_gas_key"))?;
            let md = mem.data(&caller);
            if pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg("MemoryAccessViolation: add_gas_key"));
            }
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            if pk[0] != 0xED {
                return Err(wasmtime::Error::msg(
                    "AddKeyInvalidKey: only ED25519 (0xED-prefixed) keys supported",
                ));
            }
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::AddGasKey { pk });
                } else {
                    panic!("InvalidPromiseIndex: add_gas_key");
                }
            });
            htrace!("  🔑 add_gas_key(full-access) — recorded, no enforcement");
            Ok(())
        },
    );
    let agk_fc = host_fn(
        "promise_batch_action_add_gas_key_with_function_call",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 9], vec![]),
        move |mut caller, args, _| {
            let idx = args[0].unwrap_i64() as usize;
            let pk_len = args[1].unwrap_i64() as usize;
            let pk_ptr = args[2].unwrap_i64() as usize;
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: add_gas_key_fc"))?;
            let md = mem.data(&caller);
            if pk_len != 33 || pk_ptr + pk_len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: add_gas_key_fc",
                ));
            }
            let pk = md[pk_ptr..pk_ptr + pk_len].to_vec();
            if pk[0] != 0xED {
                return Err(wasmtime::Error::msg(
                    "AddKeyInvalidKey: only ED25519 (0xED-prefixed) keys supported",
                ));
            }
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::AddGasKey { pk });
                } else {
                    panic!("InvalidPromiseIndex: add_gas_key_fc");
                }
            });
            htrace!("  🔑 add_gas_key(function-call) — recorded, no enforcement");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_gas_key_with_full_access",
        agk_full,
    )?;
    linker.define(
        &*store,
        "env",
        "promise_batch_action_add_gas_key_with_function_call",
        agk_fc,
    )?;

    // global contracts (protocol 69): deploy/use record onto the DAG; no
    // global-contract cache exists in the mock — drain-time loud no-ops.
    let dgc = host_fn(
        "promise_batch_action_deploy_global_contract",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (idx, len, ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: global_contract"))?;
            let md = mem.data(&caller);
            if ptr + len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: global_contract",
                ));
            }
            let code = md[ptr..ptr + len].to_vec();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::DeployGlobalContract { code });
                } else {
                    panic!("InvalidPromiseIndex: deploy_global_contract");
                }
            });
            htrace!("  ⚠ deploy_global_contract — recorded, no cache in mock");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_deploy_global_contract",
        dgc,
    )?;
    let dgc_by = host_fn(
        "promise_batch_action_deploy_global_contract_by_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        |caller, args, _| {
            let (idx, len, ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| wasmtime::Error::msg("MemoryAccessViolation: global_contract"))?;
            let md = mem.data(&caller);
            if ptr + len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: global_contract",
                ));
            }
            let code = md[ptr..ptr + len].to_vec();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::DeployGlobalContract { code });
                } else {
                    panic!("InvalidPromiseIndex: deploy_global_contract");
                }
            });
            htrace!("  ⚠ deploy_global_contract_by_account_id — recorded, no cache");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_deploy_global_contract_by_account_id",
        dgc_by,
    )?;
    let ugc = host_fn(
        "promise_batch_action_use_global_contract",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        move |mut caller, args, _| {
            let (idx, len, ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| {
                    wasmtime::Error::msg("MemoryAccessViolation: use_global_contract")
                })?;
            let md = mem.data(&caller);
            if ptr + len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: use_global_contract",
                ));
            }
            let account_id = String::from_utf8_lossy(&md[ptr..ptr + len]).to_string();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::UseGlobalContract {
                        account_id: account_id.clone(),
                    });
                } else {
                    panic!("InvalidPromiseIndex: use_global_contract");
                }
            });
            htrace!("  ⚠ use_global_contract({account_id}) — no-op in mock");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_use_global_contract",
        ugc,
    )?;
    let ugc_by = host_fn(
        "promise_batch_action_use_global_contract_by_account_id",
        &mut *store,
        FuncType::new(engine, vec![ValType::I64; 3], vec![]),
        |caller, args, _| {
            let (idx, len, ptr) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
            );
            let mem = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| {
                    wasmtime::Error::msg("MemoryAccessViolation: use_global_contract")
                })?;
            let md = mem.data(&caller);
            if ptr + len > md.len() {
                return Err(wasmtime::Error::msg(
                    "MemoryAccessViolation: use_global_contract",
                ));
            }
            let account_id = String::from_utf8_lossy(&md[ptr..ptr + len]).to_string();
            PROMISE_DAG.with(|d| {
                if let Some(b) = d.borrow_mut().get_mut(idx) {
                    b.actions.push(PAction::UseGlobalContract {
                        account_id: account_id.clone(),
                    });
                } else {
                    panic!("InvalidPromiseIndex: use_global_contract");
                }
            });
            htrace!("  ⚠ use_global_contract_by_account_id({account_id}) — no-op");
            Ok(())
        },
    );
    linker.define(
        &*store,
        "env",
        "promise_batch_action_use_global_contract_by_account_id",
        ugc_by,
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
        // Defensive mode (STATE_ARC unset): promise hosts LOUDLY TRAP instead
        // of silently returning 0 (the week-lost weighted-promise bug class:
        // 22de34b). A contract calling promise_create outside a driver run
        // must fail visibly, never fake success.
        let mut trap0 = |name: &'static str, n: usize, ret: bool| {
            host_fn(
                name,
                &mut *store,
                FuncType::new(
                    engine,
                    vec![ValType::I64; n],
                    if ret { vec![ValType::I64] } else { vec![] },
                ),
                move |_, _, _| {
                    Err(wasmtime::Error::msg(format!(
                        "CannotClaimPromise: {name} called with no execution state — run via a near-mock driver (cross/run/state), never bare"
                    )))
                },
            )
        };
        // Build all trap funcs through the closure first — its &mut *store
        // borrow must end before linker.define borrows store immutably.
        let (f_pc, f_pt, f_pa, f_pbc, f_pbt, f_prc, f_pr, f_pret, f_pafc, f_pyc, f_pyr, f_pbat) = (
            trap0("promise_create", 8, true),
            trap0("promise_then", 9, true),
            trap0("promise_and", 2, true),
            trap0("promise_batch_create", 2, true),
            trap0("promise_batch_then", 3, true),
            trap0("promise_results_count", 0, true),
            trap0("promise_result", 2, true),
            trap0("promise_return", 1, false),
            trap0("promise_batch_action_function_call", 7, false),
            trap0("promise_yield_create", 7, true),
            trap0("promise_yield_resume", 4, true),
            trap0("promise_batch_action_transfer", 2, false),
        );
        linker.define(&*store, "env", "promise_create", f_pc)?;
        linker.define(&*store, "env", "promise_then", f_pt)?;
        linker.define(&*store, "env", "promise_and", f_pa)?;
        linker.define(&*store, "env", "promise_batch_create", f_pbc)?;
        linker.define(&*store, "env", "promise_batch_then", f_pbt)?;
        linker.define(&*store, "env", "promise_results_count", f_prc)?;
        linker.define(&*store, "env", "promise_result", f_pr)?;
        linker.define(&*store, "env", "promise_return", f_pret)?;
        linker.define(&*store, "env", "promise_batch_action_function_call", f_pafc)?;
        linker.define(&*store, "env", "promise_yield_create", f_pyc)?;
        linker.define(&*store, "env", "promise_yield_resume", f_pyr)?;
        linker.define(&*store, "env", "promise_batch_action_transfer", f_pbat)?;
    }

    Ok(linker)
}
