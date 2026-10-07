//! Interp surface for the wasm schnorr sign/pubkey ops (BIP-340).
//!
//! Covers: official BIP-340 vector 0 sign (exact bytes), pubkey/pubkey33
//! ground truth (probed from the stitched lib), sign-pk agreement, failure
//! convention (sk=0 → all zeros), verify rejection, and — the point of the
//! port — BYTE-FOR-BYTE PARITY between the interp (builtin_schnorr.rs) and
//! the canonical stitched lib (src/wasm_emit/schnorr.wasm).

use lisp_rlm_wasm::builtin_schnorr::{
    schnorr_pubkey33_impl, schnorr_pubkey_impl, schnorr_sign_impl, schnorr_sign_pk_impl,
};
use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::program::run_program;

fn eval(code: &str) -> lisp_rlm_wasm::types::LispVal {
    let forms = parse_all(code).unwrap_or_else(|e| panic!("parse error: {}", e));
    let mut env = lisp_rlm_wasm::types::Env::new();
    let mut state = lisp_rlm_wasm::types::EvalState::new();
    run_program(&forms, &mut env, &mut state).unwrap_or_else(|e| panic!("eval error: {}", e))
}

fn bytes_list(b: &[u8]) -> String {
    let items: Vec<String> = b.iter().map(|x| x.to_string()).collect();
    format!("(list {})", items.join(" "))
}

fn eval_bytes(code: &str) -> Vec<u8> {
    match eval(code) {
        lisp_rlm_wasm::types::LispVal::List(items) => items
            .into_iter()
            .map(|v| match v {
                lisp_rlm_wasm::types::LispVal::Num(n) => n as u8,
                other => panic!("non-byte in list: {other}"),
            })
            .collect(),
        other => panic!("expected byte list, got {}", other),
    }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

fn unhex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

const ZEROS32: &[u8] = &[0u8; 32];

// ── official BIP-340 vector 0: sk=3, msg=0^32, aux=0^32 ──
fn v0_sk() -> Vec<u8> {
    let mut b = vec![0u8; 32];
    b[31] = 3;
    b
}
const V0_PK: &str = "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9";
const V0_SIG: &str = "E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0";

#[test]
fn sign_matches_official_vector0() {
    let sig = eval_bytes(&format!(
        "(schnorr-sign {} {} {})",
        bytes_list(&v0_sk()),
        bytes_list(ZEROS32),
        bytes_list(ZEROS32)
    ));
    assert_eq!(unhex(&sig), V0_SIG);
}

#[test]
fn verify_accepts_signed_vector0() {
    let code = format!(
        "(schnorr-verify {} {} {})",
        bytes_list(&hex(V0_PK)),
        bytes_list(&hex(V0_SIG)),
        bytes_list(ZEROS32)
    );
    assert_eq!(format!("{}", eval(&code)), "true");
}

#[test]
fn pubkey_ground_truth() {
    // x(1·G) — probed from the stitched lib (pubkey33 prefix 0x02: y even)
    let sk1 = {
        let mut b = vec![0u8; 32];
        b[31] = 1;
        b
    };
    let pk = eval_bytes(&format!("(schnorr-pubkey {})", bytes_list(&sk1)));
    assert_eq!(
        unhex(&pk),
        "79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798"
    );
    let pk33 = eval_bytes(&format!("(schnorr-pubkey33 {})", bytes_list(&sk1)));
    assert_eq!(
        unhex(&pk33),
        "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798"
    );
}

#[test]
fn sign_pk_agrees_with_sign_and_verifies() {
    let sk = v0_sk();
    let pk33 = schnorr_pubkey33_impl(sk[..32].try_into().unwrap());
    let sig_pk = eval_bytes(&format!(
        "(schnorr-sign-pk {} {} {} {})",
        bytes_list(&sk),
        bytes_list(&pk33),
        bytes_list(ZEROS32),
        bytes_list(ZEROS32)
    ));
    // Same aux/msg + consistent prefix ⇒ identical signature
    assert_eq!(unhex(&sig_pk), V0_SIG);
    // Verifies against the x-only pk sliced from pk33
    let xpk = &pk33[1..];
    let code = format!(
        "(schnorr-verify {} {} {})",
        bytes_list(xpk),
        bytes_list(&sig_pk),
        bytes_list(ZEROS32)
    );
    assert_eq!(format!("{}", eval(&code)), "true");
}

#[test]
fn failure_convention_all_zero_output() {
    // sk = 0 → all-zero outputs (interp twin of the wasm ret=0)
    for op in ["schnorr-pubkey", "schnorr-pubkey33"] {
        let out = eval_bytes(&format!("({op} {})", bytes_list(ZEROS32)));
        assert!(out.iter().all(|&b| b == 0), "{op} must zero-fill on sk=0");
    }
    let sig = eval_bytes(&format!(
        "(schnorr-sign {} {} {})",
        bytes_list(ZEROS32),
        bytes_list(ZEROS32),
        bytes_list(ZEROS32)
    ));
    assert!(sig.iter().all(|&b| b == 0));
}

#[test]
fn verify_rejects_tampered_sig() {
    let mut sig = hex(V0_SIG);
    let last = sig.len() - 1;
    sig[last] ^= 1;
    let code = format!(
        "(schnorr-verify {} {} {})",
        bytes_list(&hex(V0_PK)),
        bytes_list(&sig),
        bytes_list(ZEROS32)
    );
    assert_eq!(format!("{}", eval(&code)), "false");
}

// ── cross-runtime parity: interp Rust port vs canonical schnorr.wasm ──

fn wasm_sign(sk: &[u8; 32], msg: &[u8; 32], aux: &[u8; 32]) -> ([u8; 64], u32) {
    use wasmtime::*;
    let lib: &[u8] = include_bytes!("../src/wasm_emit/schnorr.wasm");
    let engine = Engine::default();
    let module = Module::new(&engine, lib).expect("schnorr.wasm module");
    let mut store = Store::new(&engine, ());
    let linker = Linker::new(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("instantiate");
    let mem = instance
        .get_export(&mut store, "memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    // Buffers at a LOW address (64K): statics (comb table, SHA K) live at the
    // bottom, and sign's shadow-stack frame descends from the very top of
    // memory — a HIGH out-buffer overlaps that frame and gets stomped mid-call
    // (uninitialized-stack reads made results unstable across processes).
    // 64K is clear of statics (< ~60KB) and far below any call frame.
    let base = 65_536usize;
    let (sk_a, msg_a, aux_a, out_a) = (base, base + 128, base + 192, base + 256);
    mem.write(&mut store, sk_a, sk).unwrap();
    mem.write(&mut store, msg_a, msg).unwrap();
    mem.write(&mut store, aux_a, aux).unwrap();
    let f = instance
        .get_func(&mut store, "schnorr_sign_bip340")
        .expect("export schnorr_sign_bip340");
    let mut ret = [Val::I32(0)];
    f.call(
        &mut store,
        &[
            Val::I32(sk_a as i32),
            Val::I32(msg_a as i32),
            Val::I32(aux_a as i32),
            Val::I32(out_a as i32),
        ],
        &mut ret,
    )
    .expect("call sign");
    let mut out = [0u8; 64];
    mem.read(&store, out_a, &mut out).unwrap();
    (out, ret[0].i32().unwrap() as u32)
}

#[test]
fn interp_sign_parity_with_wasm_lib() {
    // Vector 0 + a large-sk (near-n, exercises d' parity branch) + a msg
    // with high bits set. Interp must match the canonical lib byte-for-byte.
    let mut hi_sk = [0u8; 32];
    hi_sk[..30].copy_from_slice(&[0xAAu8; 30]);
    let mut hi_msg = [0u8; 32];
    for (i, b) in hi_msg.iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    let cases: Vec<([u8; 32], [u8; 32], [u8; 32])> = vec![
        (
            v0_sk()[..].try_into().unwrap(),
            ZEROS32.try_into().unwrap(),
            ZEROS32.try_into().unwrap(),
        ),
        (hi_sk, hi_msg, ZEROS32.try_into().unwrap()),
        (hi_sk, ZEROS32.try_into().unwrap(), hi_msg),
    ];
    for (i, (sk, msg, aux)) in cases.into_iter().enumerate() {
        let (wasm_sig, ret) = wasm_sign(&sk, &msg, &aux);
        assert_eq!(ret, 1, "case {i}: wasm lib must sign successfully");
        let interp_sig = schnorr_sign_impl(&sk, &msg, &aux);
        assert_eq!(
            interp_sig,
            wasm_sig,
            "case {i}: interp/wasm sign divergence\n  interp={:02X?}\n  wasm  ={}",
            interp_sig,
            unhex(&wasm_sig)
        );
        // And both must verify through the interp verify path
        let pk = schnorr_pubkey_impl(&sk);
        assert!(lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl(
            &pk,
            &interp_sig,
            &msg
        ));
    }
}

#[test]
fn impls_agree_on_pubkey_forms() {
    let sk: [u8; 32] = v0_sk().try_into().unwrap();
    let pk = schnorr_pubkey_impl(&sk);
    let pk33 = schnorr_pubkey33_impl(&sk);
    assert_eq!(unhex(&pk), V0_PK); // sanity vs const
    assert_eq!(&pk33[1..], &pk[..]);
    assert!(pk33[0] == 0x02 || pk33[0] == 0x03);
    // sign_pk with the true prefix == sign
    let sig1 = schnorr_sign_impl(
        &sk,
        ZEROS32.try_into().unwrap(),
        ZEROS32.try_into().unwrap(),
    );
    let sig2 = schnorr_sign_pk_impl(
        &sk,
        &pk33,
        ZEROS32.try_into().unwrap(),
        ZEROS32.try_into().unwrap(),
    );
    assert_eq!(sig1, sig2);
    // Wrong-prefix pk33 still produces a SIG (fail-closed at verify, not here)
    let mut flipped = pk33;
    flipped[0] = if flipped[0] == 0x02 { 0x03 } else { 0x02 };
    let sig3 = schnorr_sign_pk_impl(
        &sk,
        &flipped,
        ZEROS32.try_into().unwrap(),
        ZEROS32.try_into().unwrap(),
    );
    assert_ne!(sig1, sig3);
    assert!(!lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl(
        &pk, &sig3, ZEROS32
    ));
}
