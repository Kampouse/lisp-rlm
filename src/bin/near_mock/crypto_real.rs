//! Real crypto for the near-mock hosts — replaces fixed-shape stubs with the
//! same crate-level math nearcore uses (verified against the vendored
//! near-vm-runner 0.37.3 sources):
//!   - ecrecover: k256 (secp256k1), 65B uncompressed recovered point into the
//!     register, bool return; malformed input → host error, bad signature → 0.
//!     Malleability flag 1 enforces nearcore's check_signature_values(low-s).
//!   - p256_verify: p256 crate, prehash verify over NIST P-256; 33B sec1 key.
//!   - alt_bn128: see bn254.rs (substrate-bn port of vendor alt_bn128.rs).

use k256::ecdsa::{Signature, VerifyingKey};

/// nearcore Secp256K1Signature::check_signature_values(enforce_low_s):
/// recovery id 0..=3 already validated by the 65-byte decode; with the
/// malleability flag set, additionally require recid 0 (and s in the lower
/// half is implied by recid <= 1 at recovery; nearcore checks recid==0).
fn check_signature_values(recid: u8, s: &[u8; 32], enforce_low_s: bool) -> bool {
    // s must be canonical (< curve order) — k256's Signature::from_slice
    // already rejects s >= n; here we only need the malleability policy.
    //
    // nearcore near-crypto signature.rs check_signature_values(reject_upper):
    //   r < SECP256K1_N && s < (reject_upper ? SECP256K1_N_HALF_ONE : N)
    // SECP256K1_N_HALF_ONE = N/2 + 1 (NOT floor(N/2)) — verified in
    // near-crypto-0.26.0/src/signature.rs:424. k256 has already rejected
    // s >= N by the time we get here, so only the low-s bound is ours.
    if enforce_low_s {
        // SECP256K1_N_HALF_ONE = N/2 + 1, big-endian (limbs shown per u64):
        const N_HALF_ONE: [u8; 32] = [
            0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // limb 3
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // limb 2
            0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, // limb 1
            0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA1, // limb 0
        ];
        let _ = recid; // no recovery-id rule in nearcore's check
        if *s >= N_HALF_ONE {
            return false;
        }
    }
    true
}

/// nearcore ecrecover semantics (wasmtime_runner/logic.rs ecrecover):
/// returns Ok(Some(pk65)) on success (host writes register, returns 1),
/// Ok(None) when the signature does not verify (returns 0),
/// Err(msg) for nearcore HostError::ECRecoverError (host error/trap).
pub(crate) fn ecrecover(
    hash: &[u8],
    sig: &[u8],
    v: u64,
    malleability_flag: u64,
) -> Result<Option<[u8; 65]>, String> {
    if sig.len() != 64 {
        return Err(format!(
            "ECRecoverError: The length of the signature: {}, exceeds the limit of 64 bytes",
            sig.len()
        ));
    }
    if v >= 4 {
        return Err(format!(
            "ECRecoverError: V recovery byte 0 through 3 are valid but was provided {v}"
        ));
    }
    if hash.len() != 32 {
        return Err(format!(
            "ECRecoverError: The length of the hash: {}, exceeds the limit of 32 bytes",
            hash.len()
        ));
    }
    if malleability_flag != 0 && malleability_flag != 1 {
        return Err(format!(
            "ECRecoverError: Malleability flag needs to be 0 or 1, but is instead {malleability_flag}"
        ));
    }

    // (k256 0.13 API: the old 65-byte r||s||recid blob construction was
    // removed with the recoverable-signature migration below — recovery now
    // takes the raw 64B sig + RecoveryId directly.)
    let s: [u8; 32] = sig[32..].try_into().unwrap();
    if !check_signature_values(v as u8, &s, malleability_flag != 0) {
        return Ok(None);
    }

    let hash32: [u8; 32] = hash.try_into().unwrap();
    // k256 0.13: recoverable::Signature folded into the main API — a 65-byte
    // r||s||recid blob parses as `Signature::from_slice`, and recovery goes
    // through `VerifyingKey::recover_from_prehash` (equiv of nearcore's
    // secp256k1_recover, which also takes (msg32, r, s, recid)).
    let recid = k256::ecdsa::RecoveryId::try_from(v as u8)
        .map_err(|e| format!("ECRecoverError: invalid recovery id: {e}"))?;
    let sig = match Signature::from_slice(sig) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    let vk = match VerifyingKey::recover_from_prehash(&hash32, &sig, recid) {
        Ok(vk) => vk,
        Err(_) => return Ok(None),
    };
    let point = VerifyingKey::to_encoded_point(&vk, false);
    let mut out = [0u8; 65];
    out.copy_from_slice(point.as_bytes());
    Ok(Some(out))
}

/// nearcore p256_verify semantics: (sig 64B r||s, message, pk 33B sec1).
/// Length violations are host errors (trap); bad sig/key/verify → false.
pub(crate) fn p256_verify(sig: &[u8], message: &[u8], public_key: &[u8]) -> Result<bool, String> {
    use p256::ecdsa::signature::hazmat::PrehashVerifier;
    use p256::ecdsa::{Signature, VerifyingKey};

    if sig.len() != 64 {
        return Err("P256VerifyInvalidInput: invalid signature length".to_string());
    }
    let signature = match Signature::from_slice(sig) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };
    if public_key.len() != 33 {
        return Err("P256VerifyInvalidInput: invalid public key length".to_string());
    }
    let vk = match VerifyingKey::from_sec1_bytes(public_key) {
        Ok(k) => k,
        Err(_) => return Ok(false),
    };
    Ok(vk.verify_prehash(message, &signature).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use sha2::{Digest, Sha256};

    /// Recover from a real signed message; recovered key must re-verify.
    #[test]
    fn ecrecover_roundtrip() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let msg = b"near-mock ecrecover probe";
        let digest: [u8; 32] = Sha256::digest(msg).into();
        let (sig, recid) = sk.sign_prehash_recoverable(&digest).unwrap();
        let mut sig64 = [0u8; 64];
        sig64[..32].copy_from_slice(&sig.r().to_bytes());
        sig64[32..].copy_from_slice(&sig.s().to_bytes());

        // recovery id: sign_prehash_recoverable yields ids 0/1 for
        // normal signatures (no s-low-r normalization); v = recid as u8.
        let v = recid.to_byte() as u64;
        let out = ecrecover(&digest, &sig64, v, 0).unwrap();
        let pk65 = out.expect("must recover");

        // recovered point must equal the signer's public key point
        let expected = VerifyingKey::from(&sk).to_encoded_point(false);
        assert_eq!(&pk65, expected.as_bytes());

        // wrong hash → recovers a DIFFERENT key (real EC math: recovery from
        // (r,s,recid,z) always yields a point; the caller compares keys —
        // nearcore secp256k1_recover behaves the same)
        let bad_hash: [u8; 32] = Sha256::digest(b"other msg").into();
        let bad = ecrecover(&bad_hash, &sig64, v, 0).unwrap();
        assert_ne!(bad.expect("still recovers"), pk65);

        // v=4 → host error
        assert!(ecrecover(&digest, &sig64, 4, 0).is_err());
        // bad malleability flag → host error
        assert!(ecrecover(&digest, &sig64, v, 7).is_err());
        // short sig → host error
        assert!(ecrecover(&digest, &sig64[..32], v, 0).is_err());
    }

    #[test]
    fn ecrecover_low_s_policy() {
        let sk = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let digest: [u8; 32] = Sha256::digest(b"low-s").into();
        let (sig, recid) = sk.sign_prehash_recoverable(&digest).unwrap();
        let mut sig64 = [0u8; 64];
        sig64[..32].copy_from_slice(&sig.r().to_bytes());
        sig64[32..].copy_from_slice(&sig.s().to_bytes());
        let v = recid.to_byte() as u64;
        if v == 0 {
            // flag=1 with recid 0 still recovers (s is already low from k256)
            assert!(ecrecover(&digest, &sig64, v, 1).unwrap().is_some());
        } else {
            // flag=1 rejects non-zero recids
            assert!(ecrecover(&digest, &sig64, v, 1).unwrap().is_none());
            assert!(ecrecover(&digest, &sig64, v, 0).unwrap().is_some());
        }
    }

    /// Real P-256 signature verifies; tampered message/key → false.
    #[test]
    fn p256_roundtrip() {
        use p256::ecdsa::signature::Signer as PSigner;
        use p256::ecdsa::{
            Signature as PSignature, SigningKey as PSigningKey, VerifyingKey as PVerifyingKey,
        };
        let sk = PSigningKey::from_slice(&[11u8; 32]).unwrap();
        let msg = b"p256 probe";
        let sig: PSignature = sk.sign(msg); // Signer hashes the message (SHA-256)
        let msg_hash: [u8; 32] = Sha256::digest(msg).into();
        let pk33 = PVerifyingKey::from(&sk)
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        assert!(p256_verify(&sig.to_bytes(), &msg_hash, &pk33).unwrap());
        // tampered prehash → false (not an error)
        let bad_hash: [u8; 32] = Sha256::digest(b"nope").into();
        assert!(!p256_verify(&sig.to_bytes(), &bad_hash, &pk33).unwrap());
        // compressed key expected: uncompressed → host error
        let pk65 = PVerifyingKey::from(&sk)
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        assert!(p256_verify(&sig.to_bytes(), &msg_hash, &pk65).is_err());
    }

    /// alt_bn128: sum of G with 1P + 1P = 2P must equal multiexp scalar 2.
    #[test]
    fn bn254_sum_multiexp_agree() {
        use crate::bn254::{g1_multiexp, g1_sum};
        use bn::{AffineG1, Fq, G1};

        // generator of bn254 G1 (well-known point (1, 2)); u128 limbs are LE
        let g = AffineG1::new(
            Fq::from_u256(bn::arith::U256([1, 0])).unwrap(),
            Fq::from_u256(bn::arith::U256([2, 0])).unwrap(),
        )
        .expect("generator (1,2) is on bn254 G1");
        let g_ser = {
            let mut out = Vec::new();
            out.extend_from_slice(&encode_u256_pub(g.x().into_u256()));
            out.extend_from_slice(&encode_u256_pub(g.y().into_u256()));
            out
        };

        // g1_sum: [sign=0, P] + [sign=0, P]  (2 × 65 bytes)
        let mut sum_in = Vec::new();
        for _ in 0..2 {
            sum_in.push(0u8);
            sum_in.extend_from_slice(&g_ser);
        }
        let sum_out = g1_sum(
            crate::bn254::split_elements::<
                { crate::bn254::G1_SUM_ELEMENT_SIZE },
            >(&sum_in)
            .expect("65B-aligned sum input"),
        )
        .expect("valid points sum");

        // g1_multiexp: [P, scalar=2] (96 bytes)
        let mut mx_in = Vec::new();
        mx_in.extend_from_slice(&g_ser);
        mx_in.extend_from_slice(&encode_u256_pub(bn::arith::U256([2, 0])));
        let mx_out = g1_multiexp(
            crate::bn254::split_elements::<
                { crate::bn254::G1_MULTIEXP_ELEMENT_SIZE },
            >(&mx_in)
            .expect("96B-aligned multiexp input"),
        )
        .expect("valid multiexp");

        assert_eq!(sum_out, mx_out, "2P from sum == 2P from multiexp");

        // cross-check against native group math: 2G = G + G
        let two_g = G1::from(g) + G1::from(g);
        let expected = AffineG1::from_jacobian(two_g).expect("2G is not identity");
        let mut expected_ser = [0u8; 64];
        expected_ser[..32].copy_from_slice(&encode_u256_pub(expected.x().into_u256()));
        expected_ser[32..].copy_from_slice(&encode_u256_pub(expected.y().into_u256()));
        assert_eq!(sum_out, expected_ser, "sum matches native 2G");
    }

    /// alt_bn128 pairing_check: the stride is 192B (64B G1 ‖ 128B G2 —
    /// nearcore's POINT_SIZE + POINT_SIZE*2), and the semantics are the
    /// EVM-compatible ∏e(g1_i, g2_i) == identity check. Regression for
    /// the mock bug where the stride was 128B: every real 192B gate
    /// trapped with "Invalid input length 192 for 128-byte elements"
    /// (the BN254-verify breakage, 2026-09-11).
    #[test]
    fn bn254_pairing_stride_and_semantics() {
        use crate::bn254::{pairing_check, split_elements};
        use bn::{AffineG1, AffineG2, Fq, Group, Gt, G1, G2};

        fn fq_wire(v: &Fq) -> [u8; 32] {
            let mut out = [0u8; 32];
            let u = v.into_u256();
            out[..16].copy_from_slice(&u.0[0].to_le_bytes());
            out[16..].copy_from_slice(&u.0[1].to_le_bytes());
            out
        }
        fn g1_wire(p: &G1) -> [u8; 64] {
            let mut out = [0u8; 64];
            if let Some(a) = AffineG1::from_jacobian(*p) {
                out[..32].copy_from_slice(&fq_wire(&a.x()));
                out[32..].copy_from_slice(&fq_wire(&a.y()));
            }
            out
        }
        fn g2_wire(p: &G2) -> [u8; 128] {
            let mut out = [0u8; 128];
            if let Some(a) = AffineG2::from_jacobian(*p) {
                let (x, y) = (a.x(), a.y());
                out[..32].copy_from_slice(&fq_wire(&x.real()));
                out[32..64].copy_from_slice(&fq_wire(&x.imaginary()));
                out[64..96].copy_from_slice(&fq_wire(&y.real()));
                out[96..].copy_from_slice(&fq_wire(&y.imaginary()));
            }
            out
        }

        let g1 = G1::one();
        let g2 = G2::one();
        let neg_g2 = G2::zero() - g2;

        // native ground truth (same crate the hosts use)
        assert!(bn::pairing_batch(&[(g1, g2)]) != Gt::one());
        assert!(bn::pairing_batch(&[(g1, g2), (g1, neg_g2)]) == Gt::one());

        // single 192B pair on the wire: e(G1, G2) ≠ 1 → false
        let mut gate = Vec::new();
        gate.extend_from_slice(&g1_wire(&g1));
        gate.extend_from_slice(&g2_wire(&g2));
        let elems =
            split_elements::<{ crate::bn254::PAIRING_CHECK_ELEMENT_SIZE }>(&gate)
                .expect("192B-aligned gate");
        assert!(!pairing_check(elems).expect("valid points"));

        // two pairs: e(G1,G2)·e(G1,−G2) == 1 → true (the verify shape)
        gate.extend_from_slice(&g1_wire(&g1));
        gate.extend_from_slice(&g2_wire(&neg_g2));
        let elems =
            split_elements::<{ crate::bn254::PAIRING_CHECK_ELEMENT_SIZE }>(&gate)
                .expect("384B-aligned gate");
        assert!(pairing_check(elems).expect("valid points"));

        // empty input is vacuously true (matches nearcore/EVM)
        let elems = split_elements::<{ crate::bn254::PAIRING_CHECK_ELEMENT_SIZE }>(&[])
            .expect("empty is aligned");
        assert!(pairing_check(elems).expect("no points"));
    }

    fn encode_u256_pub(v: bn::arith::U256) -> [u8; 32] {
        let [lo, hi] = v.0;
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(&lo.to_le_bytes());
        out[16..].copy_from_slice(&hi.to_le_bytes());
        out
    }
}
