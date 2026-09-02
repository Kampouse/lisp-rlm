//! One-off generator for the real BLS12-381 constants used by
//! tests/test_ts_bls_msig.rs + fixtures/bls_msig.ts. Run:
//!   cargo run --release --example gen_bls_consts
//!
//! Trick: every scalar in the test vector is 1, so all curve points are
//! fixed and the aggregate signature verifies with pairing identity
//! e(σ,G2gen)·e(−H(m),apk) = 1:
//!   Q   = map_fp2_to_g2(fp2 = 0 + 1u)      (a fixed G2 subgroup point)
//!   σ   = P1Sum("00" || G1ser(1))          = +G1ser(1)
//!   apk = G2Multiexp(Q || fr=1)            = Q   (== G2gen by construction)
//!   msg = "00" || (−G1ser(1))              (client pre-negated H(m))
//! Testnet-truth gate: pairing_check(σ‖G2gen‖msg‖apk) MUST return 0.

use lisp_rlm_wasm::bls_validate::{self, kind};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    // G2 subgroup point from a canonical Fp2 element (c1 = 0, c0 = 1).
    // Wire order per nearcore: Fp2 = [c1 (48B BE)][c0 (48B BE)].
    let mut fp2_one = [0u8; 96];
    fp2_one[95] = 1;
    let g2gen = bls_validate::eval(kind::MAP_FP2_TO_G2, &fp2_one)
        .unwrap()
        .expect("canonical fp2 maps into G2");
    assert_eq!(g2gen.len(), 192);

    // Uncompressed G1 point 1·G1 from canonical Fp element (48B BE).
    let mut fp1 = [0u8; 48];
    fp1[47] = 1;
    let g1_1 = bls_validate::eval(kind::MAP_FP_TO_G1, &fp1)
        .unwrap()
        .expect("canonical fp maps into G1");
    assert_eq!(g1_1.len(), 96);

    // σ = +G1ser(1) (sign byte 0); negG1 = −G1ser(1) (sign byte 1).
    let sigma_in = {
        let mut v = vec![0u8];
        v.extend_from_slice(&g1_1);
        v
    };
    let neg_in = {
        let mut v = vec![1u8];
        v.extend_from_slice(&g1_1);
        v
    };
    let sigma = bls_validate::eval(kind::P1_SUM, &sigma_in)
        .unwrap()
        .expect("sigma sums");
    let neg_g1 = bls_validate::eval(kind::P1_SUM, &neg_in)
        .unwrap()
        .expect("negation sums");

    // apk = G2Multiexp(Q || fr=1 LE) = 1·Q = Q = g2gen (gate needs apk == g2gen).
    let apk_in = {
        let mut v = g2gen.clone();
        let mut fr = [0u8; 32];
        fr[0] = 1; // little-endian 1
        v.extend_from_slice(&fr);
        v
    };
    let apk = bls_validate::eval(kind::G2_MULTIEXP, &apk_in)
        .unwrap()
        .expect("apk multiexps");
    assert_eq!(
        apk, g2gen,
        "1·Q must be Q — apk must equal g2gen for the identity pairing"
    );

    // Client pre-negated H(m): sign-free 96B wire bytes = −G1ser(1).
    let msg = neg_g1.clone();

    // The validator pk (init arg): sign byte + 192B G2.
    let pk = {
        let mut v = vec![0u8];
        v.extend_from_slice(&g2gen);
        v
    };

    // THE GATE — this exact tuple must verify (ret 0) like on testnet.
    let gate = {
        let mut v = sigma.clone();
        v.extend_from_slice(&g2gen);
        v.extend_from_slice(&msg);
        v.extend_from_slice(&apk);
        v
    };
    assert_eq!(
        bls_validate::pairing_check(&gate).unwrap(),
        0,
        "gate must PASS"
    );

    // Negative checks — the port must reject what testnet rejects.
    let mut bad_sig_gate = gate.clone();
    bad_sig_gate[10] ^= 0x01; // corrupt σ's point bytes → malformed → ret 1
    assert_eq!(bls_validate::pairing_check(&bad_sig_gate).unwrap(), 1);
    // corrupt σ's input point → p1_sum must ret-1 (malformed), not trap
    let mut bad_sigma_in = sigma_in.clone();
    bad_sigma_in[50] ^= 0x01;
    assert_eq!(
        bls_validate::eval(kind::P1_SUM, &bad_sigma_in).unwrap(),
        None
    );
    // short gate (old 320-byte shape) → host error, like BLS12381InvalidInput
    assert!(bls_validate::pairing_check(&gate[..320]).is_err());

    // t=3 lifecycle tuple: three partials (all σi = H(m), sk=1 idiom),
    // coeffs c1=c2=c3=1 → apk = 3·Q, σ = 3·H(m). Gate:
    // e(σ,Q)·e(−H(m),apk) = e(H(m),Q)^3 · e(H(m),Q)^{−3} = 1.
    let sigma3_in = {
        let mut v = sigma_in.clone();
        v.extend_from_slice(&sigma_in);
        v.extend_from_slice(&sigma_in);
        v
    };
    let sigma3 = bls_validate::eval(kind::P1_SUM, &sigma3_in)
        .unwrap()
        .expect("sigma3 sums");
    let apk3_in = {
        let mut fr = [0u8; 32];
        fr[0] = 1;
        let mut w = Vec::new();
        for _ in 0..3 {
            w.extend_from_slice(&g2gen);
            w.extend_from_slice(&fr);
        }
        w
    };
    let apk3 = bls_validate::eval(kind::G2_MULTIEXP, &apk3_in)
        .unwrap()
        .expect("apk3 multiexps");
    let gate3 = {
        let mut v = sigma3.clone();
        v.extend_from_slice(&g2gen);
        v.extend_from_slice(&msg);
        v.extend_from_slice(&apk3);
        v
    };
    assert_eq!(
        bls_validate::pairing_check(&gate3).unwrap(),
        0,
        "t=3 gate must PASS (all coeffs 1)"
    );

    println!("// ── real BLS12-381 constants (gate verified: pairing ret 0) ──");
    println!("pub const PK_0: &str = \"{}\";", hex(&pk));
    println!("pub const SIG_0: &str = \"{}\";", hex(&sigma_in));
    println!("pub const MSG_POINT: &str = \"{}\";", hex(&msg));
    println!("pub const G2GEN: &str = \"{}\";", hex(&g2gen));
    println!("pub const SIGMA_HEX: &str = \"{}\";", hex(&sigma));
    println!("pub const APK_HEX: &str = \"{}\";", hex(&apk));
    println!("pub const SIGMA3_HEX: &str = \"{}\";", hex(&sigma3));
    println!("pub const APK3_HEX: &str = \"{}\";", hex(&apk3));
}
