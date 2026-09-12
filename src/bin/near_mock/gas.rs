//! Gas fee schedule (loadable via --gas-schedule), storage-staking
//! accounting, trie charging, stub warnings.

use super::*;
use wasmtime::*;

// ============ Run configuration (CLI flags + env, 2026-09-05) ============
/// Per-host gas schedule. Defaults = MAINNET PV155 values (nearcore
/// core/parameters snapshot, protocol 86, pulled 2026-09-10 during the
/// wasmtime parity audit — the old defaults were "legacy indicative"
/// fictions, e.g. sha256_base was ~100× low, log_base ~240× high, which made
/// any used_gas()-branching contract diverge). Override per-run with
/// `--gas-schedule file.json` (missing fields fall back to these defaults).
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
    // Crypto precompiles + validator hosts (PV155 protocol constants,
    // near-parameters 0.37.3 res/runtime_configs/parameters.yaml).
    pub(crate) ecrecover_base: u64,
    pub(crate) p256_verify_base: u64,
    pub(crate) alt_bn128_g1_multiexp_base: u64,
    pub(crate) alt_bn128_g1_multiexp_element: u64,
    pub(crate) alt_bn128_pairing_check_base: u64,
    pub(crate) alt_bn128_pairing_check_element: u64,
    pub(crate) alt_bn128_g1_sum_base: u64,
    pub(crate) alt_bn128_g1_sum_element: u64,
    pub(crate) validator_stake_base: u64,
    pub(crate) validator_total_stake_base: u64,
}

impl Default for GasSchedule {
    fn default() -> Self {
        GasSchedule {
            // ── mainnet ext_costs (PV155 / protocol-86 snapshot) ──
            log_base: 3_543_313_050,
            log_byte: 13_198_791,
            // value_return charges read_memory for the payload (PV155)
            value_return_base: 2_609_863_200,
            value_return_byte: 3_801_333,
            read_register_base: 2_517_165_186,
            read_register_byte: 98_562,
            storage_write_base: 64_196_736_000,
            storage_write_key_byte: 70_482_867,
            storage_write_value_byte: 31_018_539,
            storage_read_base: 56_356_845_749,
            storage_read_key_byte: 30_952_533,
            storage_read_value_byte: 5_611_004,
            storage_remove_base: 53_473_030_500,
            storage_remove_key_byte: 38_220_384,
            storage_has_key_base: 54_039_896_625,
            storage_has_key_key_byte: 30_790_845,
            trie_node: 2_280_000_000,
            // mock-trie calibration (NOT protocol: the mock walks a flat map,
            // 16 nodes ≈ a 32-byte key trie depth; keep for relative accuracy)
            trie_walk_nodes: 16,
            // ── crypto/validator precompiles: unchanged, were already PV155 ──
            ecrecover_base: 278_821_988_457,
            p256_verify_base: 1_300_000_000_000,
            alt_bn128_g1_multiexp_base: 713_000_000_000,
            alt_bn128_g1_multiexp_element: 320_000_000_000,
            alt_bn128_pairing_check_base: 9_686_000_000_000,
            alt_bn128_pairing_check_element: 5_102_000_000_000,
            alt_bn128_g1_sum_base: 3_000_000_000,
            alt_bn128_g1_sum_element: 5_000_000_000,
            validator_stake_base: 911_834_726_400,
            validator_total_stake_base: 911_834_726_400,
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
            ecrecover_base: g("ecrecover_base", d.ecrecover_base)?,
            p256_verify_base: g("p256_verify_base", d.p256_verify_base)?,
            alt_bn128_g1_multiexp_base: g(
                "alt_bn128_g1_multiexp_base",
                d.alt_bn128_g1_multiexp_base,
            )?,
            alt_bn128_g1_multiexp_element: g(
                "alt_bn128_g1_multiexp_element",
                d.alt_bn128_g1_multiexp_element,
            )?,
            alt_bn128_pairing_check_base: g(
                "alt_bn128_pairing_check_base",
                d.alt_bn128_pairing_check_base,
            )?,
            alt_bn128_pairing_check_element: g(
                "alt_bn128_pairing_check_element",
                d.alt_bn128_pairing_check_element,
            )?,
            alt_bn128_g1_sum_base: g("alt_bn128_g1_sum_base", d.alt_bn128_g1_sum_base)?,
            alt_bn128_g1_sum_element: g("alt_bn128_g1_sum_element", d.alt_bn128_g1_sum_element)?,
            validator_stake_base: g("validator_stake_base", d.validator_stake_base)?,
            validator_total_stake_base: g(
                "validator_total_stake_base",
                d.validator_total_stake_base,
            )?,
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
            // PV155 crypto/validator pins (2026-09-08 stub-kill batch)
            "ecrecover_base": self.ecrecover_base,
            "p256_verify_base": self.p256_verify_base,
            "alt_bn128_g1_sum_base": self.alt_bn128_g1_sum_base,
            "alt_bn128_g1_sum_element": self.alt_bn128_g1_sum_element,
            "alt_bn128_g1_multiexp_base": self.alt_bn128_g1_multiexp_base,
            "alt_bn128_g1_multiexp_element": self.alt_bn128_g1_multiexp_element,
            "alt_bn128_pairing_check_base": self.alt_bn128_pairing_check_base,
            "alt_bn128_pairing_check_element": self.alt_bn128_pairing_check_element,
            "validator_stake_base": self.validator_stake_base,
            "validator_total_stake_base": self.validator_total_stake_base,
        });
        serde_json::to_string_pretty(&j).unwrap_or_default()
    }
}

/// Fork-mode config: lazily page contract code + storage from an archival
/// RPC at a pinned block ("anvil --fork-url" for NEAR).
#[derive(Clone, Debug)]
pub(crate) struct ForkCfg {
    pub(crate) rpc: String,
    /// Pinned block for all fetches. `None` = `finality: "final"` on every
    /// request (always-fresh, but state may drift mid-session — opt-in).
    pub(crate) block: Option<u64>,
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
    /// --now <unix-seconds> | NEAR_MOCK_NOW: fixed base timestamp.
    pub(crate) base_ts: Option<i64>,
    /// --advance <seconds>: added to the base timestamp (time travel).
    pub(crate) advance_secs: i64,
    /// --trace | NEAR_MOCK_TRACE=1: record every host call (name, gas, seq)
    /// into HOST_TRACE and print a per-host summary after the run.
    pub(crate) trace: bool,
    /// Fork-mode: lazy state/code paging from archival RPC.
    pub(crate) fork: Option<ForkCfg>,
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
    let new_bal = if bytes_delta > 0 {
        bal.saturating_sub(locked_delta)
    } else {
        bal + locked_delta
    };
    st.storage.insert(bk, new_bal.to_string().into_bytes());
    if bytes_delta > 0 {
        eprintln!("  🔒 staking: locked {locked_delta} yocto (+{bytes_delta} bytes)");
    } else {
        eprintln!(
            "  🔓 staking: released {locked_delta} yocto (-{} bytes)",
            -bytes_delta
        );
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
