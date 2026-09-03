//! ed25519 host helper for the NEAR mock.
//!
//! Single source of truth for mock-side ed25519 verification — mirrors
//! `builtin_schnorr`'s role. Thin wrapper over `ed25519-dalek` (already in
//! the dependency tree for the standalone near-vm-run host): signature
//! layout `R || s` (64 B), public key 32 B, returns 1/0 like the real
//! `env.ed25519_verify` host, treating any malformed input as invalid
//! (the real host rejects, never panics the guest).

use crate::builtin_schnorr::lisp_val_to_bytes;
use crate::types::LispVal;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// Interpreter builtin `(ed25519-verify sig msg pk)` — byte-list spelling,
/// LispVal::Bool result. Mirrors the schnorr-verify builtin; the crypto core
/// is the shared `ed25519_verify_impl` (single source of truth with the
/// near-mock host and the wasm emitter's hex op).
pub fn builtin_ed25519_verify(args: &[LispVal]) -> Result<LispVal, String> {
    let sig = lisp_val_to_bytes(args.get(0).ok_or("ed25519-verify: expected (sig msg pk)")?)?;
    let msg = lisp_val_to_bytes(args.get(1).ok_or("ed25519-verify: expected (sig msg pk)")?)?;
    let pk = lisp_val_to_bytes(args.get(2).ok_or("ed25519-verify: expected (sig msg pk)")?)?;
    if pk.len() != 32 {
        return Err("ed25519-verify: pk must be 32 bytes".into());
    }
    if sig.len() != 64 {
        return Err("ed25519-verify: sig must be 64 bytes".into());
    }
    let pk_arr: [u8; 32] = pk.try_into().map_err(|_| "ed25519-verify: pk must be 32 bytes")?;
    let sig_arr: [u8; 64] = sig.try_into().map_err(|_| "ed25519-verify: sig must be 64 bytes")?;
    Ok(LispVal::Bool(ed25519_verify_impl(&pk_arr, &sig_arr, &msg) == 1))
}

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
    use super::builtin_ed25519_verify;
    use super::ed25519_verify_impl;
    use crate::builtin_schnorr::bytes_to_lisp_list;
    use crate::types::LispVal;

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
    fn rfc8032_msg_0x72() {
        let pk: [u8; 32] = hex("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
            .try_into()
            .unwrap();
        let sig: [u8; 64] = hex("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00")
            .try_into()
            .unwrap();
        assert_eq!(ed25519_verify_impl(&pk, &sig, &[0x72]), 1);
    }

    #[test]
    fn rejects_tampered_sig() {
        let pk: [u8; 32] = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .try_into()
            .unwrap();
        let mut sig: [u8; 64] = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
            .try_into()
            .unwrap();
        sig[63] ^= 0x01;
        assert_eq!(ed25519_verify_impl(&pk, &sig, b""), 0);
    }

    #[test]
    fn rejects_malformed_pk() {
        let pk = [0u8; 32]; // not a valid curve point
        let sig = [1u8; 64];
        assert_eq!(ed25519_verify_impl(&pk, &sig, b""), 0);
    }

    #[test]
    fn builtin_accepts_rfc8032_test1_and_rejects_tampered() {
        let pk: Vec<u8> =
            hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a").to_vec();
        let sig: Vec<u8> = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b").to_vec();
        let args = |sig: &[u8]| {
            vec![
                bytes_to_lisp_list(sig),
                bytes_to_lisp_list(&[]), // empty msg
                bytes_to_lisp_list(&pk),
            ]
        };
        assert!(matches!(
            builtin_ed25519_verify(&args(&sig)),
            Ok(LispVal::Bool(true))
        ));
        let mut bad = sig.clone();
        bad[0] ^= 0x01;
        assert!(matches!(
            builtin_ed25519_verify(&args(&bad)),
            Ok(LispVal::Bool(false))
        ));
    }
}
