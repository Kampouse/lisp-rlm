//! MockState storage model: key/value map + registers, partition
//! snapshot/restore for failed-receipt revert, register limits.

use std::collections::HashMap;
use wasmtime::*;

// State file: /tmp/near-mock-state.bin by default, overridable via
// NEAR_MOCK_STATE override supported
// so parallel sessions / concurrent test runners never stomp each other.
pub(crate) fn state_file() -> String {
    std::env::var("NEAR_MOCK_STATE").unwrap_or_else(|_| "/tmp/near-mock-state.bin".to_string())
}

pub(crate) fn prefixed_key(acct: &str, key: &[u8]) -> Vec<u8> {
    let mut k = acct.as_bytes().to_vec();
    k.push(0x01);
    k.extend_from_slice(key);
    k
}

/// Snapshot + revert one account's storage partition (failed receipts).
pub(crate) fn snapshot_partition(st: &MockState, acct: &str) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let pre = prefixed_key(acct, b"");
    st.storage
        .iter()
        .filter(|(k, _)| k.len() > pre.len() && k.starts_with(&pre))
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect()
}

pub(crate) fn restore_partition(
    st: &mut MockState,
    snap: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    acct: &str,
) {
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

pub(crate) struct MockState {
    pub(crate) storage: HashMap<Vec<u8>, Vec<u8>>,
    pub(crate) registers: HashMap<u64, Vec<u8>>,
    pub(crate) return_data: Option<Vec<u8>>,
    pub(crate) view: bool,
    /// keys already trie-touched this invocation (cached thereafter)
    pub(crate) touched: std::collections::HashSet<Vec<u8>>,
}

pub(crate) fn write_reg_checked(st: &mut MockState, rid: u64, data: Vec<u8>) -> Result<(), String> {
    // Mainnet VM limits (protocol-86 parameters snapshot, parity audit
    // 2026-09-10): max_register_size = 100 MiB, registers_memory_limit =
    // 1 GiB across all registers, max_number_registers = 100 — including the
    // nearcore quirk that at exactly 100 registers even REPLACING an existing
    // one fails. The old 1 MiB cap rejected mainnet-legal inputs (aurora's
    // multi-MiB submit args land in a register via input()).
    const MAX_REGS: usize = 100;
    const MAX_REG_SIZE: usize = 104_857_600;
    const REGISTERS_MEMORY_LIMIT: usize = 1_073_741_824;
    if data.len() > MAX_REG_SIZE {
        return Err(format!(
            "MemoryAccessViolation: register {} value {}b exceeds max {}b",
            rid,
            data.len(),
            MAX_REG_SIZE
        ));
    }
    if rid != u64::MAX && !st.registers.contains_key(&rid) && st.registers.len() >= MAX_REGS {
        return Err(format!(
            "MemoryAccessViolation: register limit {} exceeded",
            MAX_REGS
        ));
    }
    // total memory across registers (a replacement frees the old entry first)
    let freed = st.registers.get(&rid).map_or(0, |v| v.len());
    let new_total = st
        .registers
        .values()
        .map(|v| v.len())
        .sum::<usize>()
        .saturating_sub(freed)
        .saturating_add(data.len());
    if new_total > REGISTERS_MEMORY_LIMIT {
        return Err(format!(
            "MemoryAccessViolation: registers memory limit {} exceeded",
            REGISTERS_MEMORY_LIMIT
        ));
    }
    st.registers.insert(rid, data);
    Ok(())
}
