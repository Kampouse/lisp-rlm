//! Protocol #16: BLS12-381 threshold multisig — runtime suite.
//!
//! Mock semantics (2026-09-02, mock-truth milestone): the BLS host stubs
//! now run a VERBATIM port of nearcore's bls12381.rs — real blst curve,
//! subgroup, canonical-encoding and sign-byte validation; sign-free
//! 96/192B outputs; nearcore ret codes (0 ok / 1 malformed / 2 pairing≠1)
//! and nearcore's trap-vs-ret split (bad total length = BLS12381InvalidInput
//! host error → the mock traps, like testnet).
//!
//! Test vectors are CRYPTOGRAPHICALLY REAL, not shape blobs: every point
//! and signature below was derived with blst and verified against the
//! ported validator (examples/gen_bls_consts.rs). sk=1 idiom: σi = H(m)
//! for every validator, coefficients all 1 → apk = 3·Q, σ = 3·H(m), and
//! e(σ,G2gen)·e(−H(m),apk) = 1 exactly.
//!
//! The STATE MACHINE remains gated here; wire-encoding bugs that testnet
//! rejects now fail the mock too (the 2026-09-02 testnet execute failure —
//! corrupted point bytes sailing through the old shape-only stubs — is a
//! regression case at the bottom).

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

const BLS: &str = include_str!("../fixtures/bls_msig.ts");

// ── Real BLS12-381 constants (gate verified: pairing ret 0) ──────────
// G2gen = map_fp2_to_g2(Fp2(0 + 1u)); G1(1) = map_fp_to_g1(Fp(1)).
const PK_0: &str = "0000e12b55d801607d9760f8637ac80a4fececd3eb74045b342ee3c7dddd2037e72dedccc27e9a89491d4e57bde555fead1770d4f641225e1a1c0f7d05857299763e98e47ec6355b81dd6cdaf6db6825052f71d35ede3af8b70f046474c48d712e143ef77ba72f284b5b4f5c5ea227d269d98a8cf74a5c048a07852874d50632806cf66bc25db089319df2ee3f0212fc1c05695a740eaae8452a882e7647f22bc17782b00afa7b6be2d974824a2a7cba7eece26c60671d41145266582912235323";
const SIG_0: &str = "001073311196f8ef19477219ccee3a48035ff432295aa9419eed45d186027d88b90832e14c4f0e2aa4d15f54d1c3ed0f93034d6e3755a2073039d609db4cf3aef548283b5cc92f1021cbdb276414bcd8072b112d80a2b0a7dbf22bdaf17e006d45";
const MSG_POINT: &str = "1073311196f8ef19477219ccee3a48035ff432295aa9419eed45d186027d88b90832e14c4f0e2aa4d15f54d1c3ed0f9316b3a3b2e3dddf6a11459ddaf657fde21c4f10282a56029d9b55ab3ce1f41e1cf39ad27e0ea35823c7d3250e81ff3d66";
const G2GEN: &str = "00e12b55d801607d9760f8637ac80a4fececd3eb74045b342ee3c7dddd2037e72dedccc27e9a89491d4e57bde555fead1770d4f641225e1a1c0f7d05857299763e98e47ec6355b81dd6cdaf6db6825052f71d35ede3af8b70f046474c48d712e143ef77ba72f284b5b4f5c5ea227d269d98a8cf74a5c048a07852874d50632806cf66bc25db089319df2ee3f0212fc1c05695a740eaae8452a882e7647f22bc17782b00afa7b6be2d974824a2a7cba7eece26c60671d41145266582912235323";
const SIGMA_HEX: &str = "1073311196f8ef19477219ccee3a48035ff432295aa9419eed45d186027d88b90832e14c4f0e2aa4d15f54d1c3ed0f93034d6e3755a2073039d609db4cf3aef548283b5cc92f1021cbdb276414bcd8072b112d80a2b0a7dbf22bdaf17e006d45";
const APK_HEX: &str = "00e12b55d801607d9760f8637ac80a4fececd3eb74045b342ee3c7dddd2037e72dedccc27e9a89491d4e57bde555fead1770d4f641225e1a1c0f7d05857299763e98e47ec6355b81dd6cdaf6db6825052f71d35ede3af8b70f046474c48d712e143ef77ba72f284b5b4f5c5ea227d269d98a8cf74a5c048a07852874d50632806cf66bc25db089319df2ee3f0212fc1c05695a740eaae8452a882e7647f22bc17782b00afa7b6be2d974824a2a7cba7eece26c60671d41145266582912235323";
const SIGMA3_HEX: &str = "08692fe0860bba1b8106c0c309e852f9dea92730d3866f5d66e2ab7ccbf410834999d32fdfe478d967823e2c4f236a50009bf9263153412b71ab3b62bb02c80e407541774da7d3f19a388309f08c713ddbfe25f6a1ddff9498ff7a76889944a7";
const APK3_HEX: &str = "16159b696ba14508f75e67d8542b05c8966f809a8739e0aa894cb8c85bd31809d4f02349cdbc95fe4f2e68111185c79310e32a6ba2f90244297c55fbb4c18dde05060e597ff0b2e4370a6ee5ae3662336a903b1f5a5cbfd680de4cecb0084dcc1914349479c0c975bcc6bf2a4a75d3b5b195794c68b64e8b35fce02d02760ca7300f930caeef82cd461e2b7609af86a9083f1acda186afd8fd3469590d8b92910f1cd7b011d881ff18b4930bc814a2dc734a1f94264aa0c374923d1aea097da1";

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn call(state: &str, method: &str, args: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(BLS).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("bls_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("bls.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross").arg(state).arg(&manifest)
        .arg("bls.t.near").arg(method).arg(args)
        .output()
        .map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
        .unwrap_or_default()
}

// sk=1 idiom: every validator has the same view, so pk(0)==pk(1)==… and
// σi are identical; the aggregate (all-1 coefficients) is 3·Q / 3·H(m).
fn pk(_i: usize) -> String { PK_0.to_string() }
fn sig(_i: usize) -> String { SIG_0.to_string() }

/// Coefficient blob for validators 0..n: 66-char entries (2-hex 1-based
/// idx + 64-hex LE scalar 1).
fn coeffs_ones(n: usize) -> String {
    (0..n)
        .map(|i| format!("{:02x}{}{}", i + 1, "01", "00".repeat(31)))
        .collect()
}

/// Direct check of the ported validator semantics the mock now inherits.
#[test]
fn bls_validator_semantics() {
    use lisp_rlm_wasm::bls_validate::{self, kind};

    // wire hex → bytes helper
    let unhex = |h: &str| {
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
    };

    // σ = P1Sum("00" ‖ G1(1)) reproduces the testnet-signature shape
    let sigma_in = {
        let mut v = vec![0u8];
        v.extend_from_slice(&unhex(&SIGMA_HEX));
        v
    };
    assert_eq!(
        bls_validate::eval(kind::P1_SUM, &sigma_in).unwrap().unwrap(),
        unhex(&SIGMA_HEX),
        "sum of a single positive point must be the point itself"
    );

    // apk = G2Multiexp(G2gen ‖ fr=1) == G2gen (1·Q)
    let apk_in = {
        let mut v = unhex(&G2GEN);
        v.extend_from_slice(&[1u8]);
        v.extend_from_slice(&[0u8; 31]);
        v
    };
    assert_eq!(
        bls_validate::eval(kind::G2_MULTIEXP, &apk_in).unwrap().unwrap(),
        unhex(&G2GEN)
    );

    // empty sum = identity, serialized exactly as nearcore does it
    // (p_affine_serialize → uncompressed-infinity: 0x40 + 95 zero bytes)
    let empty = bls_validate::eval(kind::P1_SUM, &[]).unwrap().unwrap();
    assert_eq!(empty.len(), 96);
    assert_eq!(empty[0], 0x40, "identity: uncompressed-infinity flag");
    assert!(empty[1..].iter().all(|&b| b == 0));

    // nearcore ret-code split:
    //   malformed POINT (canonical, on-curve, subgroup, sign ∈ {0,1}) → ret 1
    let mut bad = sigma_in.clone();
    bad[50] ^= 0x01;
    assert_eq!(bls_validate::eval(kind::P1_SUM, &bad).unwrap(), None);
    //   sign byte 2 → ret 1
    let mut bad_sign = sigma_in.clone();
    bad_sign[0] = 2;
    assert_eq!(bls_validate::eval(kind::P1_SUM, &bad_sign).unwrap(), None);
    //   non-canonical Fp (≥ modulus) → map_fp_to_g1 ret 1
    assert_eq!(bls_validate::eval(kind::MAP_FP_TO_G1, &[0xFF; 48]).unwrap(), None);
    //   bad TOTAL length → HOST ERROR (the mock traps, like BLS12381InvalidInput)
    assert!(bls_validate::eval(kind::P1_SUM, &[0u8; 96]).is_err());
    assert!(bls_validate::pairing_check(&[0u8; 320]).is_err());

    // pairing gate: the real tuple verifies…
    let gate = {
        let mut v = unhex(&SIGMA_HEX);
        v.extend_from_slice(&unhex(&G2GEN));
        v.extend_from_slice(&unhex(&MSG_POINT));
        v.extend_from_slice(&unhex(&APK_HEX));
        v
    };
    assert_eq!(bls_validate::pairing_check(&gate).unwrap(), 0);
    // …and a corrupted tuple fails structurally (ret 1), not accidentally
    let mut corrupt = gate.clone();
    corrupt[10] ^= 0x01;
    assert_eq!(bls_validate::pairing_check(&corrupt).unwrap(), 1);
}

#[test]
fn bls_msig_threshold_lifecycle() {
    let st = "/tmp/bls-msig-t.bin";
    let _ = std::fs::remove_file(st);

    // 4 validators, threshold 3
    let pks: Vec<String> = (0..4).map(pk).collect();
    let init = format!(
        r#"{{"pks":["{}","{}","{}","{}"],"t":3}}"#,
        pks[0], pks[1], pks[2], pks[3]
    );
    let r = call(st, "init", &init);
    assert!(r.contains("ok:4:3"), "init: {r}");

    // re-init rejected
    let r = call(st, "init", &init);
    assert!(r.contains("already initialized"), "re-init: {r}");

    // threshold > n rejected (fresh state)
    let st2 = "/tmp/bls-msig-t2.bin";
    let _ = std::fs::remove_file(st2);
    let bad = format!(r#"{{"pks":["{}"],"t":2}}"#, pks[0]);
    let r = call(st2, "init", &bad);
    assert!(r.contains("threshold exceeds"), "t>n: {r}");

    let sub = |i: usize| {
        format!(
            r#"{{"id":"m1","msg":"{}","i":{i},"sig":"{}"}}"#,
            MSG_POINT,
            sig(i)
        )
    };
    for i in 0..3 {
        let r = call(st, "submit", &sub(i));
        assert!(r.contains(&format!("submitted:{}", i + 1)), "sub{i}: {r}");
    }

    // dedupe: same validator resubmits
    let r = call(st, "submit", &sub(2));
    assert!(r.contains("partial already submitted"), "dedupe: {r}");

    // message binding: validator 3 signs a different message for m1
    let bad_msg = format!(r#"{{"id":"m1","msg":"{}","i":3,"sig":"{}"}}"#, SIG_0, sig(3));
    let r = call(st, "submit", &bad_msg);
    assert!(r.contains("message mismatch"), "msg bind: {r}");

    // short sig rejected
    let short = format!(r#"{{"id":"m1","msg":"{}","i":3,"sig":"aabb"}}"#, MSG_POINT);
    let r = call(st, "submit", &short);
    assert!(r.contains("97-byte"), "sig len: {r}");

    // out-of-range validator
    let oor = format!(r#"{{"id":"m1","msg":"{}","i":9,"sig":"{}"}}"#, MSG_POINT, sig(9));
    let r = call(st, "submit", &oor);
    assert!(r.contains("out of range"), "range: {r}");

    // execute before threshold on a second message id
    let early = format!(r#"{{"id":"m2","coeffs":"{}"}}"#, coeffs_ones(1));
    let r = call(st, "execute", &early);
    assert!(r.contains("not enough partials"), "early: {r}");

    // happy execute: real points through the json path (setPoints) + gate
    let pts = format!(r#"{{"id":"m1","msgPoint":"{}","g2gen":"{}"}}"#, MSG_POINT, G2GEN);
    let r = call(st, "setPoints", &pts);
    assert!(r.contains("points-ok"), "points: {r}");
    let exec = format!(r#"{{"id":"m1","coeffs":"{}"}}"#, coeffs_ones(3));
    let r = call(st, "execute", &exec);
    // cryptographic truth: σ = 3·H(m) with all-1 coefficients
    assert!(
        r.contains(&format!("executed:{}", SIGMA3_HEX)),
        "exec: {r}"
    );

    // verified view holds the aggregate signature
    let r = call(st, "verified", r#"{"id":"m1"}"#);
    assert!(r.contains(&SIGMA3_HEX[..32]), "verified: {r}");

    // execute-once
    let r = call(st, "execute", &exec);
    assert!(r.contains("already executed"), "once: {r}");

    // missing coefficient: fresh id, 3 partials, coeffs only cover 0,1
    for i in 0..3 {
        let m3 = format!(r#"{{"id":"m3","msg":"{}","i":{i},"sig":"{}"}}"#, MSG_POINT, sig(i));
        let r = call(st, "submit", &m3);
        assert!(r.contains("submitted:"), "m3 sub{i}: {r}");
    }
    let coeffs2 = coeffs_ones(2);
    let exec_bad = format!(r#"{{"id":"m3","coeffs":"{coeffs2}"}}"#);
    let r = call(st, "execute", &exec_bad);
    assert!(r.contains("missing coefficient"), "missing coeff: {r}");

    // m3 with full coeffs succeeds (same sk=1 algebra as m1)
    let r = call(
        st,
        "execute",
        &format!(r#"{{"id":"m3","coeffs":"{}"}}"#, coeffs_ones(3)),
    );
    assert!(r.contains(&format!("executed:{}", SIGMA3_HEX)), "m3 exec: {r}");
}

#[test]
fn bls_pairing_gate_shape() {
    // pairing gate end-to-end: real valid signature → executes;
    // structurally valid but wrong points → pairing ret 2 → abort;
    // short H(m) → 320B gate → host error TRAP (BLS12381InvalidInput).
    let st = "/tmp/bls-msig-gate.bin";
    let _ = std::fs::remove_file(st);
    let pks: Vec<String> = (0..4).map(pk).collect();
    let init = format!(r#"{{"pks":["{}","{}","{}","{}"],"t":1}}"#, pks[0], pks[1], pks[2], pks[3]);
    let r = call(st, "init", &init);
    assert!(r.contains("ok:4:1"), "init t=1: {r}");

    // H(m) resolves from the submitted message blob (client pre-negated
    // H(m) = −G1(1)); g2gen from the args. Gate = (σ ‖ G2gen ‖ H(m) ‖ apk).
    let msg = MSG_POINT;
    let sub = format!(r#"{{"id":"g1","msg":"{msg}","i":0,"sig":"{}"}}"#, sig(0));
    let r = call(st, "submit", &sub);
    assert!(r.contains("submitted:1"), "sub: {r}");

    let exec = format!(
        r#"{{"id":"g1","msgPoint":"{msg}","g2gen":"{}","coeffs":"{}"}}"#,
        G2GEN,
        coeffs_ones(1)
    );
    let r = call(st, "execute", &exec);
    assert!(r.contains(&format!("executed:{}", SIGMA_HEX)), "gate pass: {r}");

    // wrong H(m) (valid point, wrong sign — breaks the pairing identity)
    // → well-formed gate, pairing ≠ 1 → ret 2 → abort (NOT a trap)
    let st_w = "/tmp/bls-msig-gate-wrong.bin";
    let _ = std::fs::remove_file(st_w);
    let r = call(st_w, "init", &init);
    assert!(r.contains("ok:4:1"));
    let r = call(st_w, "submit", &sub);
    assert!(r.contains("submitted:1"));
    // wrong H(m) (valid point, wrong sign — breaks the pairing identity),
    // delivered through the fixture's real data path: setPoints writes
    // bls:mp:<id>, which execute resolves BEFORE the submit-bound blob.
    // → well-formed 576B gate, pairing ≠ 1 → ret 2 → abort (NOT a trap)
    let st_w = "/tmp/bls-msig-gate-wrong.bin";
    let _ = std::fs::remove_file(st_w);
    let r = call(st_w, "init", &init);
    assert!(r.contains("ok:4:1"));
    let r = call(st_w, "submit", &sub);
    assert!(r.contains("submitted:1"));
    let pts_wrong = format!(
        r#"{{"id":"g1","msgPoint":"{}","g2gen":"{}"}}"#,
        SIGMA_HEX, // +H(m) instead of −H(m)
        G2GEN
    );
    let r = call(st_w, "setPoints", &pts_wrong);
    assert!(r.contains("points-ok"), "setPoints wrong: {r}");
    let r = call(st_w, "execute", &exec);
    assert!(r.contains("pairing check failed"), "wrong H(m): {r}");

    // short H(m) (32B) → gate 320B binary → not %288 → HOST ERROR → trap
    let st2 = "/tmp/bls-msig-gate2.bin";
    let _ = std::fs::remove_file(st2);
    let r = call(st2, "init", &init);
    assert!(r.contains("ok:4:1"));
    let short_msg = format!("{}{}", &MSG_POINT[..96], "00".repeat(32));
    let sub2 = format!(r#"{{"id":"g1","msg":"{short_msg}","i":0,"sig":"{}"}}"#, sig(0));
    let r = call(st2, "submit", &sub2);
    assert!(r.contains("submitted:1"), "sub2: {r}");
    let r = call(st2, "execute", &exec);
    assert!(r.contains("❌"), "short gate must trap: {r}");
    assert!(r.contains("BLS12381InvalidInput"), "trap flavor: {r}");
}

/// Regression for the 2026-09-02 testnet execute failure: corrupted point
/// bytes must now FAIL locally (they sailed through the old shape stubs).
#[test]
fn bls_wire_corruption_rejected_end_to_end() {
    let st = "/tmp/bls-msig-corrupt.bin";
    let _ = std::fs::remove_file(st);
    let pks: Vec<String> = (0..2).map(pk).collect();
    let init = format!(r#"{{"pks":["{}","{}"],"t":1}}"#, pks[0], pks[1]);
    let r = call(st, "init", &init);
    assert!(r.contains("ok:2:1"), "init: {r}");

    // (a) corrupted partial sig → p1_sum ret 1 → σ empty → gate built from
    // empty pieces is 576B of structured-but-wrong points → pairing ret 1
    // → abort. Deterministic, not an accidental pass.
    let mut bad_sig = sig(0);
    let flip = 40; // inside the G1 x-coordinate
    let byte = if bad_sig.as_bytes()[flip] == b'0' { b'1' } else { b'0' };
    bad_sig.replace_range(flip..flip + 1, &(byte as char).to_string());
    let sub = format!(r#"{{"id":"c1","msg":"{}","i":0,"sig":"{bad_sig}"}}"#, MSG_POINT);
    let r = call(st, "submit", &sub);
    assert!(r.contains("submitted:1"), "sub: {r}");
    let exec = format!(
        r#"{{"id":"c1","msgPoint":"{}","g2gen":"{}","coeffs":"{}"}}"#,
        MSG_POINT,
        G2GEN,
        coeffs_ones(1)
    );
    let r = call(st, "execute", &exec);
    // corrupt σ → p1_sum ret 1 → σ="" → gate = 0+384+192+384 hex = 480B
    // → pairing_check length error → TRAP (same class as today's testnet
    // failure: BLS12381InvalidInput kills the tx, no abort path).
    assert!(
        r.contains("BLS12381InvalidInput") && r.contains("480 is not divisible by 288"),
        "corrupt σ must trap at the gate: {r}"
    );

    // (b) the g2gen-clobber shape from TASK-json-bug.md: corrupted G2gen
    // bytes reach the pairing gate → malformed → ret 1 → abort (mock now
    // catches locally what only testnet caught before).
    let st2 = "/tmp/bls-msig-corrupt2.bin";
    let _ = std::fs::remove_file(st2);
    let r = call(st2, "init", &init);
    assert!(r.contains("ok:2:1"));
    let sub = format!(r#"{{"id":"c2","msg":"{}","i":0,"sig":"{}"}}"#, MSG_POINT, sig(0));
    let r = call(st2, "submit", &sub);
    assert!(r.contains("submitted:1"), "sub2: {r}");
    let mut bad_gen = G2GEN.to_string();
    let flip = 200;
    let byte = if bad_gen.as_bytes()[flip] == b'0' { b'1' } else { b'0' };
    bad_gen.replace_range(flip..flip + 1, &(byte as char).to_string());
    let exec_badgen = format!(
        r#"{{"id":"c2","msgPoint":"{}","g2gen":"{bad_gen}","coeffs":"{}"}}"#,
        MSG_POINT,
        coeffs_ones(1)
    );
    let r = call(st2, "execute", &exec_badgen);
    // corrupt g2gen flows RAW into the gate (still 192B — apk aggregates
    // the uncorrupted pks). The gate stays 576B well-formed; pairing's own
    // parse hits the off-curve G2gen → ret 1 → abort. (Old mock: always
    // ret 1 gate → executed:garbage. Now the corruption is caught.)
    assert!(r.contains("pairing check failed"), "corrupt g2gen: {r}");
}
