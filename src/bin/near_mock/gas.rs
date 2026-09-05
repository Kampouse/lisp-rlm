//! Gas fee schedule (loadable via --gas-schedule), storage-staking
//! accounting, trie charging, stub warnings.

use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use wasmtime::*;
use lisp_rlm_wasm::bls_validate;
use lisp_rlm_wasm::builtin_ed25519::ed25519_verify_impl;
use lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl;

// ============ Run configuration (CLI flags + env, 2026-09-05) ============
/// Per-host gas schedule. Defaults = the legacy indicative constants that
/// were previously hardcoded at each call site. Override per-run with
/// `--gas-schedule file.json` (missing fields fall back to these defaults)
/// after calibrating against a real sandbox / near-vm-run oracle.
#[derive(Clone, Debug)]
pub(crate) struct GasSchedule {
    pub(crate) log_base: u64,
    pub(crate) log_byte: u64,
    pub(crate) value_return_base: u64,
    pub(crate) value_return_byte: u64,
    pub(crate) read_register_base: u64,
    pub(crate) read_register_byte: u64,
    pub(crate) storage_write_base: u64,
    pub(crate) storage_write_key_byte: u64,
    pub(crate) storage_write_value_byte: u64,
    pub(crate) storage_read_base: u64,
    pub(crate) storage_read_key_byte: u64,
    pub(crate) storage_read_value_byte: u64,
    pub(crate) storage_remove_base: u64,
    pub(crate) storage_remove_key_byte: u64,
    pub(crate) storage_has_key_base: u64,
    pub(crate) storage_has_key_key_byte: u64,
    pub(crate) trie_node: u64,
    pub(crate) trie_walk_nodes: u64,
}

impl Default for GasSchedule {
    fn default() -> Self {
        GasSchedule {
            log_base: 13_181_732,
            log_byte: 19_335_348,
            value_return_base: 4_141_250,
            value_return_byte: 3_574_166,
            read_register_base: 24_108_449,
            read_register_byte: 3_574_166,
            storage_write_base: 64_000_000,
            storage_write_key_byte: 90_563,
            storage_write_value_byte: 3_548_576,
            storage_read_base: 56_356_995,
            storage_read_key_byte: 81_569,
            storage_read_value_byte: 3_574_166,
            storage_remove_base: 64_000_000,
            storage_remove_key_byte: 90_563,
            storage_has_key_base: 56_356_995,
            storage_has_key_key_byte: 81_569,
            trie_node: 2_280_000_000,
            trie_walk_nodes: 16,
        }
    }
}

impl GasSchedule {
    pub(crate) fn from_json_file(path: &str) -> Result<GasSchedule, String> {
        let raw = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let v: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|e| format!("bad JSON in {path}: {e}"))?;
        let d = GasSchedule::default();
        // Strict: an explicitly present field with the wrong type/value is an
        // error, not a silent fall-back to the default (a typo'd schedule must
        // never masquerade as a calibrated one). Missing fields still default.
        let g = |k: &str, def: u64| -> Result<u64, String> {
            match v.get(k) {
                None | Some(serde_json::Value::Null) => Ok(def),
                Some(x) => match x.as_u64() {
                    Some(n) if n > 0 => Ok(n),
                    Some(0) => Err(format!("gas schedule field '{k}' must be > 0")),
                    _ => Err(format!(
                        "gas schedule field '{k}' must be a positive integer, got {x}"
                    )),
                },
            }
        };
        Ok(GasSchedule {
            log_base: g("log_base", d.log_base)?,
            log_byte: g("log_byte", d.log_byte)?,
            value_return_base: g("value_return_base", d.value_return_base)?,
            value_return_byte: g("value_return_byte", d.value_return_byte)?,
            read_register_base: g("read_register_base", d.read_register_base)?,
            read_register_byte: g("read_register_byte", d.read_register_byte)?,
            storage_write_base: g("storage_write_base", d.storage_write_base)?,
            storage_write_key_byte: g("storage_write_key_byte", d.storage_write_key_byte)?,
            storage_write_value_byte: g("storage_write_value_byte", d.storage_write_value_byte)?,
            storage_read_base: g("storage_read_base", d.storage_read_base)?,
            storage_read_key_byte: g("storage_read_key_byte", d.storage_read_key_byte)?,
            storage_read_value_byte: g("storage_read_value_byte", d.storage_read_value_byte)?,
            storage_remove_base: g("storage_remove_base", d.storage_remove_base)?,
            storage_remove_key_byte: g("storage_remove_key_byte", d.storage_remove_key_byte)?,
            storage_has_key_base: g("storage_has_key_base", d.storage_has_key_base)?,
            storage_has_key_key_byte: g("storage_has_key_key_byte", d.storage_has_key_key_byte)?,
            trie_node: g("trie_node", d.trie_node)?,
            trie_walk_nodes: g("trie_walk_nodes", d.trie_walk_nodes)?,
        })
    }

    pub(crate) fn to_json(&self) -> String {
        let j = serde_json::json!({
            "log_base": self.log_base, "log_byte": self.log_byte,
            "value_return_base": self.value_return_base, "value_return_byte": self.value_return_byte,
            "read_register_base": self.read_register_base, "read_register_byte": self.read_register_byte,
            "storage_write_base": self.storage_write_base,
            "storage_write_key_byte": self.storage_write_key_byte,
            "storage_write_value_byte": self.storage_write_value_byte,
            "storage_read_base": self.storage_read_base,
            "storage_read_key_byte": self.storage_read_key_byte,
            "storage_read_value_byte": self.storage_read_value_byte,
            "storage_remove_base": self.storage_remove_base,
            "storage_remove_key_byte": self.storage_remove_key_byte,
            "storage_has_key_base": self.storage_has_key_base,
            "storage_has_key_key_byte": self.storage_has_key_key_byte,
            "trie_node": self.trie_node, "trie_walk_nodes": self.trie_walk_nodes,
        });
        serde_json::to_string_pretty(&j).unwrap_or_default()
    }
}

/// Real NEAR storage staking: 1e20 yoctoNEAR (0.1 NEAR) locked per byte.
pub(crate) const STAKING_COST_PER_BYTE: u128 = 100_000_000_000_000_000_000;

#[derive(Clone)]
pub(crate) struct RunCfg {
    pub(crate) gas: GasSchedule,
    /// --staking: charge storage staking (account_balance shrinks, locked
    /// balance grows, remove refunds). Default off (legacy behavior).
    pub(crate) staking: bool,
    /// --dry-run: execute + report, but do NOT persist state.
    pub(crate) dry_run: bool,
    /// NEAR_MOCK_DEBUG=1 or --debug: verbose host traces ([schnorr-dbg] etc).
    pub(crate) debug: bool,
    /// NEAR_MOCK_WARN_STUBS=1: eprintln on every unimplemented host stub call.
    pub(crate) warn_stubs: bool,
    /// --now <unix-seconds> | NEAR_MOCK_NOW: fixed base timestamp.
    pub(crate) base_ts: Option<i64>,
    /// --advance <seconds>: added to the base timestamp (time travel).
    pub(crate) advance_secs: i64,
}

/// Warn once per host name when a stub backed by zeros/empties is called.
pub(crate) fn stub_warn(name: &str) {
    if mock_cfg().warn_stubs {
        eprintln!("  ⚠ STUB {name}: not implemented — returns 0/empty (NEAR_MOCK_WARN_STUBS)");
    }
}

/// SplitMix64 — cheap mixing for the per-call random_seed entropy.
pub(crate) fn splitmix64(z: &mut u64) -> u64 {
    *z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = *z;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Storage currently locked (staked) for an account = bytes × 1e20 yocto.
/// The balance entry itself is excluded.
pub(crate) fn locked_balance_for(st: &MockState, acct: &str) -> u128 {
    if !mock_cfg().staking {
        return 0;
    }
    let prefix = prefixed_key(acct, b"");
    st.storage
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix) && !k.ends_with(b"\x00near-bal"))
        .map(|(k, v)| (k.len() + v.len()) as u128)
        .sum::<u128>()
        .saturating_mul(STAKING_COST_PER_BYTE)
}

/// Credit/debit the account's storage-staking locked amount when raw bytes
/// are added/removed under its namespace. No-op unless --staking.
pub(crate) fn apply_staking_delta(st: &mut MockState, acct: &str, bytes_delta: i64) {
    if !mock_cfg().staking || bytes_delta == 0 {
        return;
    }
    let bk = prefixed_key(acct, b"\x00near-bal");
    let bal: u128 = st
        .storage
        .get(&bk)
        .and_then(|v| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let locked_delta = (bytes_delta.unsigned_abs() as u128).saturating_mul(STAKING_COST_PER_BYTE);
    let new_bal = if bytes_delta > 0 { bal.saturating_sub(locked_delta) } else { bal + locked_delta };
    st.storage.insert(bk, new_bal.to_string().into_bytes());
    if bytes_delta > 0 {
        eprintln!("  🔒 staking: locked {locked_delta} yocto (+{bytes_delta} bytes)");
    } else {
        eprintln!("  🔓 staking: released {locked_delta} yocto (-{} bytes)", -bytes_delta);
    }
}

/// Production trie-access charging (testnet PV85, EXPERIMENTAL_protocol_config
/// at block 266,843,869, fetched 2026-09-02):
///   touching_trie_node    = 2_280_000_000 gas / node
///   read_cached_trie_node = 2_280_000_000 gas / node (no read discount at PV85)
/// First touch of a key walks ~16 trie nodes (32-byte key depth in the mock
/// trie); repeats charge at the cached-read rate. Calibrated against the
/// near-vm-run oracle: view reads land within ~10% of production.
pub(crate) fn trie_charge(st: &mut MockState, key: &[u8]) -> u64 {
    let g = mock_cfg().gas;
    if st.touched.insert(key.to_vec()) {
        g.trie_walk_nodes * g.trie_node
    } else {
        g.trie_node
    }
}

/// Writes re-walk the trie unconditionally (locate node + persist mutation) —
/// the read cache never subsidizes a write.
pub(crate) fn trie_charge_write(st: &mut MockState, key: &[u8]) -> u64 {
    let g = mock_cfg().gas;
    st.touched.insert(key.to_vec());
    g.trie_walk_nodes * g.trie_node
}
