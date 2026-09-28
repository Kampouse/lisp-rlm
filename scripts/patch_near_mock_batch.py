#!/usr/bin/env python3
"""Atomic multi-edit for near_mock.rs (2026-09-05 improvement batch).
Exact-match replacements with assertion counts — fails loudly on drift."""

PATH = "/Users/asil/dev/lisp-rlm/src/bin/near_mock.rs"

R = []  # (old, new)

# R1: storage_write — schedule + staking
R.append((
"""                    // Indicative legacy fees: base + key/value bytes
                    let cost = 64_000_000u64
                        + 90_563u64 * kl as u64
                        + 3_548_576u64 * vl as u64;
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    let mut st = s6.lock().unwrap();
                    let trie = trie_charge_write(&mut st, &key);
                    let old = st.storage.insert(key, val);
""",
"""                    // Fee schedule (legacy indicative defaults, --gas-schedule to override)
                    let gas = &mock_cfg().gas;
                    let cost = gas.storage_write_base
                        + gas.storage_write_key_byte * kl as u64
                        + gas.storage_write_value_byte * vl as u64;
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(cost))?;
                    let mut st = s6.lock().unwrap();
                    let trie = trie_charge_write(&mut st, &key);
                    // Storage staking: lock for net new bytes (refund replaced).
                    // Computed pre-insert because `key`/`val` move into the map.
                    let old_raw_len = old
                        .as_ref()
                        .map(|o| key.len() - acct.len() - 1 + o.len())
                        .unwrap_or(0);
                    apply_staking_delta(
                        &mut st,
                        &acct,
                        (kl + vl) as i64 - old_raw_len as i64,
                    );
                    drop(st);
                    caller.set_fuel(caller.get_fuel()?.saturating_sub(trie))?;
                    let mut st = s6.lock().unwrap();
                    let old = st.storage.insert(key, val);
"""))

# R2a: storage_read found branch — schedule
R.append((
"""                    eprintln!("  → storage_read found {}b", val.len());
                    // Indicative flat fees + production trie-node access
                    let trie = trie_charge(&mut st, key);
                    let cost = 56_356_995u64
                        + 81_569u64 * kl as u64
                        + 3_574_166u64 * val.len() as u64
                        + trie;
""",
"""                    eprintln!("  → storage_read found {}b", val.len());
                    // Fee schedule + production trie-node access
                    let gas = &mock_cfg().gas;
                    let trie = trie_charge(&mut st, key);
                    let cost = gas.storage_read_base
                        + gas.storage_read_key_byte * kl as u64
                        + gas.storage_read_value_byte * val.len() as u64
                        + trie;
"""))

# R2b: storage_read miss branch — schedule
R.append((
"""                    // production charges the read base + trie walk even on miss
                    let trie = trie_charge(&mut st, key);
                    let cost = 56_356_995u64 + 81_569u64 * kl as u64 + trie;
""",
"""                    // production charges the read base + trie walk even on miss
                    let gas = &mock_cfg().gas;
                    let trie = trie_charge(&mut st, key);
                    let cost = gas.storage_read_base + gas.storage_read_key_byte * kl as u64 + trie;
"""))

# R3: storage_remove — schedule + staking refund
R.append((
"""                    if let Some(val) = val {
                        // Indicative legacy fees: base + key bytes + trie access
                        let cost = 64_000_000u64 + 90_563u64 * kl as u64 + trie;
""",
"""                    if let Some(val) = val {
                        // Fee schedule: base + key bytes + trie access
                        let gas = &mock_cfg().gas;
                        let cost =
                            gas.storage_remove_base + gas.storage_remove_key_byte * kl as u64 + trie;
                        // Storage staking: refund the removed bytes
                        apply_staking_delta(
                            &mut s8.lock().unwrap(),
                            &exec_ctx_or_default().contract,
                            -((kl + val.len()) as i64),
                        );
"""))

# R4: storage_has_key — schedule
R.append((
"""                    // Indicative legacy fees + trie-node access
                    let cost = 56_356_995u64 + 81_569u64 * kl as u64 + trie;
""",
"""                    // Fee schedule + trie-node access
                    let gas = &mock_cfg().gas;
                    let cost = gas.storage_has_key_base + gas.storage_has_key_key_byte * kl as u64 + trie;
"""))

# R5: value_return — schedule
R.append((
"""            // Indicative legacy fees: read_memory base + per byte
            let cost = 4_141_250u64 + 3_574_166u64 * len as u64;
""",
"""            // Fee schedule: read_memory base + per byte
            let cost =
                mock_cfg().gas.value_return_base + mock_cfg().gas.value_return_byte * len as u64;
"""))

# R6: read_register — schedule
R.append((
"""                    // Indicative legacy fees: base + per byte
                    let cost = 24_108_449u64 + 3_574_166u64 * data.len() as u64;
""",
"""                    // Fee schedule: base + per byte
                    let cost = mock_cfg().gas.read_register_base
                        + mock_cfg().gas.read_register_byte * data.len() as u64;
"""))

# R7: random_seed — real entropy + NEAR_MOCK_SEED pin
R.append((
"""            let rid = args[0].unwrap_i64() as u64;
            // Deterministic 32B seed → 64-char lowercase hex (real NEAR
            // returns raw bytes; the compiler's read_to_register path keeps
            // bytes, but the TS surface stringifies as hex — parity with
            // the ctx battery's `seed.length == 64` probe).
            let seed: Vec<u8> = (0u32..8)
                .flat_map(|i| (0x5EED_0000u32.wrapping_add(i)).to_le_bytes())
                .collect();
            let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
""",
"""            let rid = args[0].unwrap_i64() as u64;
            // Real entropy (time ^ pid, SplitMix64-spread) → 64-char lowercase
            // hex (real NEAR returns raw bytes; the compiler's
            // read_to_register path keeps bytes, but the TS surface stringifies
            // as hex — parity with the ctx battery's `seed.length == 64`
            // probe). Pin with NEAR_MOCK_SEED for reproducible runs.
            let mut z = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5EED)
                ^ ((std::process::id() as u64) << 32);
            let seed: Vec<u8> = (0..4).flat_map(|_| splitmix64(&mut z).to_le_bytes()).collect();
            let hex: String = match std::env::var("NEAR_MOCK_SEED") {
                Ok(pin) => format!("{:0<64}", pin.trim()),
                _ => seed.iter().map(|b| format!("{b:02x}")).collect(),
            };
"""))

# R8a: schnorr dbg — gate entry line
R.append((
"""            eprintln!("[schnorr-dbg] entry pk_ptr={} sig_ptr={} msg_ptr={} msg_len={} mem_len={}", pk_ptr, sig_ptr, msg_ptr, msg_len, data.len());
""",
"""            if mock_cfg().debug {
                eprintln!("[schnorr-dbg] entry pk_ptr={pk_ptr} sig_ptr={sig_ptr} msg_ptr={msg_ptr} msg_len={msg_len} mem_len={}", data.len());
            }
"""))

# R8b: schnorr dbg — gate BOUNDS REJECT
R.append((
"""                eprintln!("[schnorr-dbg] BOUNDS REJECT");
""",
"""                if mock_cfg().debug {
                    eprintln!("[schnorr-dbg] BOUNDS REJECT");
                }
"""))

# R8c: schnorr dbg — gate in-closure lines
R.append((
"""                eprintln!("[schnorr-dbg] pk_ptr={} sig_ptr={} msg_ptr={} msg_len={}", pk_ptr, sig_ptr, msg_ptr, msg_len);
                eprintln!("[schnorr-dbg] pk[0..8]={:02x?} sig[0..8]={:02x?} msg[0..8]={:02x?}", &pk[0..8], &sig[0..8], &msg[msg.len().min(8)..msg.len().min(16).max(8)]);
                let r = schnorr_verify_impl(&pk, &sig, msg) as i32;
                eprintln!("[schnorr-dbg] result={}", r);
""",
"""                if mock_cfg().debug {
                    eprintln!("[schnorr-dbg] pk_ptr={pk_ptr} sig_ptr={sig_ptr} msg_ptr={msg_ptr} msg_len={msg_len}");
                }
                let r = schnorr_verify_impl(&pk, &sig, msg) as i32;
                if mock_cfg().debug {
                    eprintln!("[schnorr-dbg] result={r}");
                }
"""))

# R8d: schnorr dbg — gate PANIC
R.append((
"""            })).unwrap_or_else(|_| { eprintln!("[schnorr-dbg] PANIC"); 0 });
""",
"""            }))
            .unwrap_or_else(|_| {
                if mock_cfg().debug {
                    eprintln!("[schnorr-dbg] PANIC");
                }
                0
            });
"""))

# R9a: log_utf16 — cost schedule
R.append((
"""            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            let cost = 13_181_732u64 + 19_335_348u64 * len as u64;
""",
"""            let (len, ptr) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            let cost = mock_cfg().gas.log_base + mock_cfg().gas.log_byte * len as u64;
"""))

# R9b: log_utf16 — route through handle_log_line
R.append((
"""                    let msg = String::from_utf16_lossy(&units);
                    println!("  LOG: {}  [debug len={} ptr={}] (utf16)", msg, len, ptr);
""",
"""                    let msg = String::from_utf16_lossy(&units);
                    handle_log_line(
                        &msg,
                        mock_cfg().debug,
                        &format!("  [debug len={len} ptr={ptr}] (utf16)"),
                    );
"""))

# R10: account_balance — subtract locked (staking)
R.append((
"""                .and_then(|s| s.parse().ok())
                })
                .unwrap_or(0);
            let ptr = args[0].unwrap_i64() as usize;
""",
"""                .and_then(|s| s.parse().ok())
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
"""))

# R11a: storage_usage — real implementation (was noop 0)
R.append((
"""    linker.define(&*store, "env", "storage_usage", noop0r.clone())?;
""",
"""    // storage_usage() -> u64: bytes used by THIS contract's namespace
    // (was a silent 0 — flashpool-style checks saw free storage forever).
    let su0 = state.clone();
    let storage_usage_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![], vec![ValType::I64]),
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
"""))

# R11b: account_locked_balance — real implementation (was noop 0)
R.append((
"""    linker.define(&*store, "env", "account_locked_balance", noop1.clone())?;
""",
"""    // account_locked_balance(balance_ptr): 16-byte u128 write of the
    // storage-staked amount (was a silent 0).
    let alb0 = state.clone();
    let account_locked_balance_fn = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
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
    linker.define(&*store, "env", "account_locked_balance", account_locked_balance_fn)?;
"""))

# R12a: validator_stake — named stub warning
R.append((
"""    linker.define(&*store, "env", "validator_stake", noop_3i.clone())?;
""",
"""    let vs0 = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64; 3], vec![]),
        |_, _, _| {
            stub_warn("validator_stake");
            Ok(())
        },
    );
    linker.define(&*store, "env", "validator_stake", vs0)?;
"""))

# R12b: validator_total_stake — named stub warning
R.append((
"""    linker.define(&*store, "env", "validator_total_stake", noop1.clone())?;
""",
"""    let vts0 = Func::new(
        &mut *store,
        FuncType::new(&engine, vec![ValType::I64], vec![]),
        |_, _, _| {
            stub_warn("validator_total_stake");
            Ok(())
        },
    );
    linker.define(&*store, "env", "validator_total_stake", vts0)?;
"""))

with open(PATH) as f:
    content = f.read()

failed = []
for i, (old, new) in enumerate(R, 1):
    n = content.count(old)
    if n != 1:
        failed.append((i, n))
        print(f"R{i}: MATCH COUNT {n} (expected 1) — SKIPPED")
    else:
        content = content.replace(old, new, 1)
        print(f"R{i}: applied")

if failed:
    print(f"\n{len(failed)} replacements failed — file NOT written")
    raise SystemExit(1)

with open(PATH, "w") as f:
    f.write(content)
print("\nAll replacements applied, file written.")
