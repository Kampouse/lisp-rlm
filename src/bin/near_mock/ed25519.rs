//! ed25519 host helper for the NEAR mock.
//!
//! Single source of truth for mock-side ed25519 verification — mirrors
//! `builtin_schnorr`'s role. Thin wrapper over `ed25519-dalek` (already in
//! the dependency tree for the standalone near-vm-run host): signature
//! layout `R || s` (64 B), public key 32 B, returns 1/0 like the real
//! `env.ed25519_verify` host, treating any malformed input as invalid
//! (the real host rejects, never panics the guest).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// Verify an ed25519 signature. `sig` = 64 bytes (R || s), `pk` = 32 bytes.
/// Returns 1 for a valid signature, 0 for invalid/malformed input —
/// never panics (host convention: verification failure is data, not a trap).
pub fn ed25519_verify_impl(pk: &[u8; 32], sig: &[u8; 64], msg: &[u8]) -> i32 {
    let Ok(vk) = VerifyingKey::from_bytes(pk) else {
        return 0;
    };
    let Ok(signature) = Signature::from_slice(sig) else {
        return 0;
    };
    i32::from(vk.verify(msg, &signature).is_ok())
}

#[cfg(test)]
mod tests {
    use super::ed25519_verify_impl;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn rfc8032_empty_message() {
        let pk: [u8; 32] = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .try_into()
            .unwrap();
        let sig: [u8; 64] = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
            .try_into()
            .unwrap();
        assert_eq!(ed25519_verify_impl(&pk, &sig, b""), 1);
    }

    #[test]
    fn tampered_sig_rejected() {
        let pk: [u8; 32] = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .try_into()
            .unwrap();
        let mut sig: [u8; 64] = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
            .try_into()
            .unwrap();
        sig[0] ^= 0xFF;
        assert_eq!(ed25519_verify_impl(&pk, &sig, b""), 0);
    }
}
