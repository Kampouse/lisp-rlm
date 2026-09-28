//! BN254 (alt_bn128) precompiles end-to-end — the "zk verify" surface.
//!
//! Regression for the 2026-09-11 fix: the wasm emitter passed the raw hex
//! STRING to the alt_bn128_* hosts instead of decoded binary (the bls12381
//! arms already had the hex⇄binary bridge; the alt_bn128 arms didn't). A
//! 384B pairing gate arrived as 768 ASCII bytes — which is %192==0, so it
//! even passed the length check and decoded as garbage points. Every
//! zk-Groth16-style verify built on pairing_check was broken (silent
//! wrong result or "AltBn128 invalid input" trap).
//!
//! All vectors are computed at test time with the same zeropool-bn the
//! hosts use — nothing hardcoded. Wire format is the nearcore quirk:
//! each 32-byte field element = [lo u128 LE ‖ hi u128 LE].

use bn::Group;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const BN: &str = include_str!("../fixtures/bn254.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn call(state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(BN).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("bn254_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("bn254.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(state)
        .arg(&manifest)
        .arg("bn254.t.near")
        .arg(method)
        .arg(args)
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default()
}

// ── nearcore wire encoding (LE-halves quirk), bn-crate powered ───────

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fq → 32B = [lo u128 LE ‖ hi u128 LE] (nearcore encode_u256).
fn fq_wire(v: &bn::Fq) -> [u8; 32] {
    let u = v.into_u256();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&u.0[0].to_le_bytes());
    out[16..].copy_from_slice(&u.0[1].to_le_bytes());
    out
}

/// u128 pair → 32B scalar wire (same LE-halves layout).
fn scalar_wire(lo: u128, hi: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&lo.to_le_bytes());
    out[16..].copy_from_slice(&hi.to_le_bytes());
    out
}

/// G1 jacobian → 64B affine wire; infinity (0,0), like nearcore encode_g1.
fn g1_wire(p: &bn::G1) -> [u8; 64] {
    let mut out = [0u8; 64];
    if let Some(a) = bn::AffineG1::from_jacobian(*p) {
        out[..32].copy_from_slice(&fq_wire(&a.x()));
        out[32..].copy_from_slice(&fq_wire(&a.y()));
    }
    out
}

/// G2 jacobian → 128B affine wire: x.c0 ‖ x.c1 ‖ y.c0 ‖ y.c1.
fn g2_wire(p: &bn::G2) -> [u8; 128] {
    let mut out = [0u8; 128];
    if let Some(a) = bn::AffineG2::from_jacobian(*p) {
        let (x, y) = (a.x(), a.y());
        out[..32].copy_from_slice(&fq_wire(&x.real()));
        out[32..64].copy_from_slice(&fq_wire(&x.imaginary()));
        out[64..96].copy_from_slice(&fq_wire(&y.real()));
        out[96..].copy_from_slice(&fq_wire(&y.imaginary()));
    }
    out
}

fn args_buf(buf: &str) -> String {
    format!(r#"{{"buf":"{buf}"}}"#)
}

#[test]
fn bn254_g1_sum_and_multiexp_e2e() {
    let st = "/tmp/bn254-e2e.bin";
    let _ = std::fs::remove_file(st);

    let g1 = bn::G1::one();
    let g1_hex = hex(&g1_wire(&g1));

    // single positive element: sum = the point itself (round-trip)
    let r = call(st, "g1_sum", &args_buf(&format!("00{g1_hex}")));
    assert!(r.contains(&format!("LOG: g1sum:{g1_hex}")), "identity: {r}");

    // P + (−P) = infinity → nearcore encodes it as all-zero 64B
    let r = call(st, "g1_sum", &args_buf(&format!("00{g1_hex}01{g1_hex}")));
    assert!(
        r.contains(&format!("LOG: g1sum:{}", "0".repeat(128))),
        "cancel: {r}"
    );

    // multiexp [P, 2] == 2P == sum [P, +P]
    let two_g1_hex = hex(&g1_wire(&(g1 + g1)));
    let r = call(
        st,
        "g1_multiexp",
        &args_buf(&format!("{g1_hex}{}", hex(&scalar_wire(2, 0)))),
    );
    assert!(r.contains(&format!("LOG: g1mx:{two_g1_hex}")), "mx 2P: {r}");

    let r = call(st, "g1_sum", &args_buf(&format!("00{g1_hex}00{g1_hex}")));
    assert!(
        r.contains(&format!("LOG: g1sum:{two_g1_hex}")),
        "sum 2P: {r}"
    );

    // multiexp with a SECOND-term check: [P,1]+[P,1] via one 192B buffer
    let r = call(
        st,
        "g1_multiexp",
        &args_buf(&format!(
            "{g1_hex}{}{g1_hex}{}",
            hex(&scalar_wire(1, 0)),
            hex(&scalar_wire(1, 0))
        )),
    );
    assert!(
        r.contains(&format!("LOG: g1mx:{two_g1_hex}")),
        "mx 1+1: {r}"
    );

    // ragged length (66B % 65 ≠ 0) → HOST ERROR trap, like the chain
    let r = call(st, "g1_sum", &args_buf(&"0".repeat(132)));
    assert!(r.contains("❌"), "ragged must trap: {r}");
    assert!(r.contains("Invalid input length"), "trap flavor: {r}");
}

#[test]
fn bn254_pairing_check_e2e() {
    let st = "/tmp/bn254-pc.bin";
    let _ = std::fs::remove_file(st);

    let g1 = bn::G1::one();
    let g2 = bn::G2::one();
    let g1_hex = hex(&g1_wire(&g1));
    let g2_hex = hex(&g2_wire(&g2));

    // sanity in the test itself: e(G1,G2) ≠ 1 …
    assert!(bn::pairing_batch(&[(g1, g2)]) != bn::Gt::one());
    // … and e(G1,G2)·e(G1,−G2) == 1
    let neg_g2 = bn::G2::zero() - g2;
    assert!(bn::pairing_batch(&[(g1, g2), (g1, neg_g2)]) == bn::Gt::one());

    // empty input: vacuous true → 1
    let r = call(st, "pairing_check", &args_buf(""));
    assert!(r.contains("LOG: pairing:1"), "vacuous: {r}");

    // single pair (G1, G2): product ≠ identity → 0
    let r = call(st, "pairing_check", &args_buf(&format!("{g1_hex}{g2_hex}")));
    assert!(r.contains("LOG: pairing:0"), "e(g1,g2)≠1: {r}");

    // (G1, G2), (G1, −G2): product == identity → 1 — THE verify shape
    let neg_g2_hex = hex(&g2_wire(&neg_g2));
    let r = call(
        st,
        "pairing_check",
        &args_buf(&format!("{g1_hex}{g2_hex}{g1_hex}{neg_g2_hex}")),
    );
    assert!(r.contains("LOG: pairing:1"), "verify shape: {r}");

    // not-on-curve G1 (1,3) → AltBn128InvalidInput host error → trap
    let mut bad = [0u8; 64];
    bad[..32].copy_from_slice(&scalar_wire(1, 0)); // x = 1
    bad[32..].copy_from_slice(&scalar_wire(3, 0)); // y = 3 — not on curve
    let mut gate = bad.to_vec();
    gate.extend_from_slice(&g2_wire(&g2));
    let r = call(st, "pairing_check", &args_buf(&hex(&gate)));
    assert!(r.contains("❌"), "bad point must trap: {r}");
    assert!(r.contains("invalid g1"), "trap flavor: {r}");
}
