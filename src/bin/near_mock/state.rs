//! MockState storage model: key/value map + registers, partition
//! snapshot/restore for failed-receipt revert, register limits.

use super::*;
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
pub(crate) fn state_file() -> String {
    // single source of truth lives in the library (tests use it too)
    lisp_rlm_wasm::near_mock_state_file()
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

pub(crate) fn restore_partition(st: &mut MockState, snap: Vec<(Vec<u8>, Option<Vec<u8>>)>, acct: &str) {
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
