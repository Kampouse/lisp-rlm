//! Cryptographic hash builtins — thin wrappers over the `sha2` and `sha3` crates.

use crate::helpers::as_str;
use crate::types::LispVal;
use sha2::{Sha256, Digest};
use sha3::Keccak256;

/// SHA-256 hash — returns hex-encoded digest.
pub fn builtin_sha256(args: &[LispVal]) -> Result<LispVal, String> {
    let data = as_str(&args[0])?;
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    Ok(LispVal::Str(hex_encode(&result)))
}

/// Keccak-256 hash — returns hex-encoded digest.
pub fn builtin_keccak256(args: &[LispVal]) -> Result<LispVal, String> {
    let data = as_str(&args[0])?;
    let mut hasher = Keccak256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    Ok(LispVal::Str(hex_encode(&result)))
}

// ---------------------------------------------------------------------------
// Hex utilities
// ---------------------------------------------------------------------------

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}
