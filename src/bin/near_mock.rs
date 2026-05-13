//! NEAR contract mock runner with state persistence.
//! Warms up wee_alloc by calling a cheap init method first.
//!
//! Usage:
//!   cargo run --bin near-mock -- <wasm> <method> [args-json]
//!   cargo run --bin near-mock -- <wasm> exports|imports|reset

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

const STATE_FILE: &str = "/tmp/near-mock-state.bin";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: near-mock <wasm> <method> [args-json]");
        eprintln!("       near-mock <wasm> exports|imports|reset");
        std::process::exit(1);
    }

    let wasm_path = &args[1];
    let method = &args[2];
    let args_json = args.get(3).cloned().unwrap_or_else(|| "{}".to_string());

    if method == "reset" {
        let _ = std::fs::remove_file(STATE_FILE);
        println!("🗑️  State cleared");
        return Ok(());
    }

    let wasm_bytes = std::fs::read(wasm_path)?;
    println!("📦 {} ({} bytes)", wasm_path, wasm_bytes.len());

    let engine = Engine::default();
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
    let loaded_storage: HashMap<Vec<u8>, Vec<u8>> = std::fs::read(STATE_FILE)
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
        registers: HashMap::new(),
        return_data: None,
    }));

    let mut store = Store::new(&engine, ());
    // 1024 pages = 64MB initial memory. Enough that wee_alloc never needs memory_grow.
    let memory = Memory::new(&mut store, MemoryType::new(1024, None))?;

    // === Host functions (all created before linking) ===

    let s1 = state.clone();
    let log_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    let msg = String::from_utf8_lossy(&data[ptr..ptr + len]).to_string();
                    println!("  LOG: {}", msg);
                }
            }
            Ok(())
        },
    );

    let s2 = state.clone();
    let value_return_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                if ptr + len <= data.len() {
                    s2.lock().unwrap().return_data = Some(data[ptr..ptr + len].to_vec());
                }
            }
            Ok(())
        },
    );

    let s3 = state.clone();
    let read_register_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        move |mut caller, args, _| {
            let (rid, ptr) = (args[0].unwrap_i64() as u64, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                if let Some(data) = s3.lock().unwrap().registers.get(&rid).cloned() {
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
                    eprintln!("  ⚠ read_register({}): not found", rid);
                }
            }
            Ok(())
        },
    );

    let s4 = state.clone();
    let register_len_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![ValType::I64]),
        move |_, args, results| {
            let rid = args[0].unwrap_i64() as u64;
            let len = s4
                .lock()
                .unwrap()
                .registers
                .get(&rid)
                .map(|d| d.len() as i64)
                .unwrap_or(0);
            eprintln!("  → register_len({}) = {}", rid, len);
            results[0] = Val::I64(len);
            Ok(())
        },
    );

    let input_src = args_json.clone();
    let s5 = state.clone();
    let input_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            eprintln!("  → input(reg={})", rid);
            let mut st = s5.lock().unwrap();
            if !st.registers.contains_key(&rid) {
                st.registers.insert(rid, input_src.as_bytes().to_vec());
            }
            Ok(())
        },
    );

    let s6 = state.clone();
    let storage_write_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 5], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp, vl, vp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize,
                args[3].unwrap_i64() as usize,
                args[4].unwrap_i64() as u64,
            );
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() && vp + vl <= md.len() {
                    let key = md[kp..kp + kl].to_vec();
                    let val = md[vp..vp + vl].to_vec();
                    eprintln!(
                        "  → storage_write(\"{}\") = {}b",
                        String::from_utf8_lossy(&key),
                        vl
                    );
                    let old = s6.lock().unwrap().storage.insert(key, val);
                    if rid != u64::MAX {
                        if let Some(old) = old {
                            s6.lock().unwrap().registers.insert(rid, old);
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
        &mut store,
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

            // Step 2: search HashMap (no caller borrow)
            let found = if let Some(key) = &key_from_mem {
                let mut st = s7.lock().unwrap();
                if let Some(val) = st.storage.get(key).cloned() {
                    st.registers.insert(rid, val);
                    eprintln!("  → storage_read found {}b", val.len());
                    true
                } else {
                    eprintln!("  → storage_read not found");
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
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp, rid) = (
                args[0].unwrap_i64() as usize,
                args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as u64,
            );
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() {
                    if let Some(val) = s8.lock().unwrap().storage.remove(&md[kp..kp + kl].to_vec())
                    {
                        if rid != u64::MAX {
                            s8.lock().unwrap().registers.insert(rid, val);
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
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        move |mut caller, args, results| {
            let (kl, kp) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp + kl <= md.len() {
                    results[0] = Val::I64(
                        if s9.lock().unwrap().storage.contains_key(&md[kp..kp + kl]) {
                            1
                        } else {
                            0
                        },
                    );
                    return Ok(());
                }
            }
            results[0] = Val::I64(0);
            Ok(())
        },
    );

    let panic_fn = Func::new(
        &mut store,
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
        &mut store,
        FuncType::new(&engine, vec![], vec![]),
        |_, _, _| Err(wasmtime::Error::msg("ABORT")),
    );

    let s_ca = state.clone();
    let current_account_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_ca.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, b"escrow.test.near".to_vec());
            Ok(())
        },
    );

    let s_sa = state.clone();
    let signer_account_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_sa.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, b"owner.test.near".to_vec());
            Ok(())
        },
    );

    let s_pa = state.clone();
    let predecessor_account_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_pa.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, b"owner.test.near".to_vec());
            Ok(())
        },
    );

    let s_pk = state.clone();
    let signer_pk_fn = Func::new(
        &mut store,
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
        &mut store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64,
            );
            Ok(())
        },
    );

    let s_ab = state.clone();
    let account_balance_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_ab.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, vec![0u8; 16]);
            Ok(())
        },
    );

    let s_ad = state.clone();
    let attached_deposit_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        move |_, args, _| {
            s_ad.lock()
                .unwrap()
                .registers
                .insert(args[0].unwrap_i64() as u64, vec![0u8; 16]);
            Ok(())
        },
    );

    // Noop stubs with correct arities
    let noop1 = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        |_, _, _| Ok(()),
    );
    let noop0r = Func::new(
        &mut store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_2i_1o = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_3i_1o = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_3i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_2i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_4i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 4], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_6i_1o = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 6], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_7i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 7], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_7i_1o = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 7], vec![ValType::I64]),
        |_, _, r| {
            r[0] = Val::I64(0);
            Ok(())
        },
    );
    let noop_8i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 8], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_9i = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 9], vec![]),
        |_, _, _| Ok(()),
    );
    let noop_4i_i32 = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I64; 4], vec![ValType::I32]),
        |_, _, r| {
            r[0] = Val::I32(0);
            Ok(())
        },
    );

    // === Link ===
    let mut linker = Linker::new(&engine);
    linker.define(&store, "env", "memory", memory)?;
    linker.define(&store, "env", "log_utf8", log_fn)?;
    linker.define(&store, "env", "value_return", value_return_fn)?;
    linker.define(&store, "env", "read_register", read_register_fn)?;
    linker.define(&store, "env", "register_len", register_len_fn)?;
    linker.define(&store, "env", "input", input_fn)?;
    linker.define(&store, "env", "storage_write", storage_write_fn)?;
    linker.define(&store, "env", "storage_read", storage_read_fn)?;
    linker.define(&store, "env", "storage_remove", storage_remove_fn)?;
    linker.define(&store, "env", "storage_has_key", storage_has_key_fn)?;
    linker.define(&store, "env", "panic_utf8", panic_fn)?;
    linker.define(&store, "env", "panic", abort_fn.clone())?;
    linker.define(&store, "env", "abort", abort_fn)?;
    linker.define(&store, "env", "current_account_id", current_account_fn)?;
    linker.define(&store, "env", "signer_account_id", signer_account_fn)?;
    linker.define(&store, "env", "signer_account_pk", signer_pk_fn)?;
    linker.define(
        &store,
        "env",
        "predecessor_account_id",
        predecessor_account_fn,
    )?;
    linker.define(&store, "env", "block_index", noop0r.clone())?;
    linker.define(&store, "env", "block_timestamp", block_ts_fn)?;
    linker.define(&store, "env", "account_balance", account_balance_fn)?;
    linker.define(&store, "env", "attached_deposit", attached_deposit_fn)?;
    linker.define(&store, "env", "used_gas", noop0r.clone())?;
    linker.define(&store, "env", "prepaid_gas", noop0r.clone())?;
    linker.define(&store, "env", "random_seed", noop1.clone())?;
    linker.define(&store, "env", "sha256", noop1.clone())?;
    linker.define(&store, "env", "keccak256", noop1.clone())?;
    linker.define(&store, "env", "log", noop1.clone())?;
    linker.define(&store, "env", "validator_stake", noop_2i_1o.clone())?;
    linker.define(&store, "env", "validator_total_stake", noop0r.clone())?;
    linker.define(&store, "env", "alt_bn128_g1_multiexp", noop1.clone())?;
    linker.define(&store, "env", "alt_bn128_g1_sum", noop1.clone())?;
    linker.define(&store, "env", "alt_bn128_pairing_check", noop1.clone())?;
    linker.define(&store, "env", "ed25519_verify", noop_6i_1o)?;
    linker.define(&store, "env", "ecrecover", noop_2i_1o.clone())?;
    linker.define(&store, "env", "epoch_height", noop0r.clone())?;
    linker.define(&store, "env", "storage_usage", noop0r.clone())?;
    linker.define(&store, "env", "log_s", noop1.clone())?;
    linker.define(&store, "env", "validator_account_id", noop1.clone())?;
    linker.define(&store, "env", "promise_create", noop_3i_1o.clone())?;
    linker.define(&store, "env", "promise_then", noop_3i_1o.clone())?;
    linker.define(&store, "env", "promise_and", noop_2i_1o.clone())?;
    linker.define(&store, "env", "promise_batch_create", noop_2i_1o.clone())?;
    linker.define(&store, "env", "promise_batch_then", noop_3i_1o)?;
    linker.define(&store, "env", "promise_results", noop1.clone())?;
    linker.define(&store, "env", "promise_results_count", noop0r.clone())?;
    linker.define(&store, "env", "promise_result", noop_2i_1o.clone())?;
    linker.define(&store, "env", "promise_return", noop1.clone())?;
    linker.define(&store, "env", "promise_yield_create", noop_7i_1o)?;
    linker.define(&store, "env", "promise_yield_resume", noop_4i_i32)?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_create_account",
        noop1.clone(),
    )?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_deploy_contract",
        noop_3i.clone(),
    )?;
    linker.define(&store, "env", "promise_batch_action_function_call", noop_7i)?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_function_call_weight",
        noop_8i,
    )?;
    linker.define(&store, "env", "promise_batch_action_transfer", noop_2i)?;
    linker.define(&store, "env", "promise_batch_action_stake", noop_4i.clone())?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_add_key_with_full_access",
        noop_4i,
    )?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_add_key_with_function_call",
        noop_9i,
    )?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_delete_key",
        noop_3i.clone(),
    )?;
    linker.define(
        &store,
        "env",
        "promise_batch_action_delete_account",
        noop_3i,
    )?;

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
    println!(
        "▶ {}({})",
        method,
        if args_json == "{}" { "" } else { &args_json }
    );
    let result = func.call(&mut store, &[], &mut []);

    // Check memory before call
    if let Some(real_mem) = instance.get_memory(&mut store, "memory") {
        eprintln!(
            "  WASM memory before: {} pages",
            real_mem.data(&store).len() / 65536
        );
    }

    // Use a thread with timeout
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
                let s = String::from_utf8_lossy(data);
                if !s.is_empty() {
                    println!("📄 {}", s);
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
        }
        Err(e) => println!("❌ {}", e),
    }

    // Persist storage
    {
        let st = state.lock().unwrap();
        let encoded = bincode::serialize(&st.storage)?;
        std::fs::write(STATE_FILE, encoded)?;
        println!("💾 Saved {} keys", st.storage.len());
    }

    Ok(())
}

struct MockState {
    storage: HashMap<Vec<u8>, Vec<u8>>,
    registers: HashMap<u64, Vec<u8>>,
    return_data: Option<Vec<u8>>,
}
