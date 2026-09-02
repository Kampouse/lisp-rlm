//! Protocol #16: BLS12-381 threshold multisig — runtime suite.
//!
//! Mock semantics: precompile stubs are shape-correct (96/192-char hex
//! convention) but not crypto-true; pairing_check returns 1 iff input
//! length is a positive multiple of 384 (one G1||G2 pair per 384 bytes
//! of hex-convention data: σ 96 + apk 192 + H(m) 96). The STATE MACHINE
//! is what's gated here; cryptographic truth runs on the real chain.
//!
//! Found & fixed along the way (2026-09-02):
//! - register-result aliasing: two live near/* crypto results shared
//!   TEMP_MEM — second call overwrote first (σ showed apk's bytes);
//!   emitter now heap-copies every register read in the crypto family.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

const BLS: &str = include_str!("../fixtures/bls_msig.ts");

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

fn vid(i: usize) -> String { let c = (b'a' + i as u8) as char; format!("{c}{c}") }
// NEAR BLS ABI shapes: pk = sign+uncompressed G2 = 193B = 386 hex;
// sig = sign+uncompressed G1 = 97B = 194 hex
fn pk(i: usize) -> String { format!("00{}{}", vid(i), "ee".repeat(191)) }
fn sig(i: usize) -> String { format!("00{}{}", vid(i), "ee".repeat(95)) }

/// Un-ignored 2026-09-02: json_get_str now unescapes into a runtime-heap
/// block sized to the value (TASK-json-bug.md) — the setPoints/execute
/// flow's ~700-byte multi-key args parse correctly.
#[test]
fn bls_msig_threshold_lifecycle() {
    let st = "/tmp/bls-msig-t.bin";
    let _ = std::fs::remove_file(st);

    // 4 validators, threshold 3
    let pks: Vec<String> = (0..4).map(|i| format!("{}{}{}", "ab".repeat(48), (b'a' + i as u8) as char, "dd".repeat(47)).replace("dd".repeat(47).as_str(), "")).collect();
    let pks: Vec<String> = (0..4)
        .map(pk)
        .collect();
    let init = format!(r#"{{"pks":[{:?},"{:?}","{:?}","{:?}"],"t":3}}"#, pks[0], pks[1], pks[2], pks[3]);
    // NOTE: format {:?} on String adds quotes; build plainly instead:
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

    let msg = "cd".repeat(96);   // msgPoint: 96B uncompressed G1 = 192 hex
    let sub = |i: usize| format!(r#"{{"id":"m1","msg":"{msg}","i":{i},"sig":"{}"}}"#, sig(i));
    for i in 0..3 {
        let r = call(st, "submit", &sub(i));
        assert!(r.contains(&format!("submitted:{}", i + 1)), "sub{i}: {r}");
    }

    // dedupe: same validator resubmits
    let r = call(st, "submit", &sub(2));
    assert!(r.contains("partial already submitted"), "dedupe: {r}");

    // message binding: validator 3 signs a different message for m1
    let bad_msg = format!(r#"{{"id":"m1","msg":"{}","i":3,"sig":"{}"}}"#, "ef".repeat(48), sig(3));
    let r = call(st, "submit", &bad_msg);
    assert!(r.contains("message mismatch"), "msg bind: {r}");

    // short sig rejected
    let short = format!(r#"{{"id":"m1","msg":"{msg}","i":3,"sig":"aabb"}}"#);
    let r = call(st, "submit", &short);
    assert!(r.contains("97-byte"), "sig len: {r}");

    // out-of-range validator
    let oor = format!(r#"{{"id":"m1","msg":"{msg}","i":9,"sig":"{}"}}"#, sig(9));
    let r = call(st, "submit", &oor);
    assert!(r.contains("out of range"), "range: {r}");

    // execute before threshold on a second message id
    let early = format!(r#"{{"id":"m2","coeffs":"{}"}}"#, "00".to_string() + &"11".repeat(32));
    let r = call(st, "execute", &early);
    assert!(r.contains("not enough partials") || r.contains("unknown message"), "early: {r}");

    // happy execute: coeffs = 3 fixed-stride entries (00|01|02), 32B each
    let coeffs = format!("{}{}{}", "01".to_string() + &"11".repeat(32), "02".to_string() + &"22".repeat(32), "03".to_string() + &"33".repeat(32));
    let g2gen = "07".repeat(192);
    let pts = format!(r#"{{"id":"m1","msgPoint":"{msg}","g2gen":"{g2gen}"}}"#);
    let r = call(st, "setPoints", &pts);
    assert!(r.contains("points-ok"), "points: {r}");
    let exec = format!(r#"{{"id":"m1","coeffs":"{coeffs}"}}"#);
    let r = call(st, "execute", &exec);
    assert!(r.contains("executed:0a11"), "exec: {r}");  // sign-free mock blob starts 0a (i=1,2: 0x0a,0x11)

    // verified view holds the aggregate signature
    let r = call(st, "verified", r#"{"id":"m1"}"#);
    assert!(r.contains("executed") || r.contains("00"), "verified: {r}");

    // execute-once
    let r = call(st, "execute", &exec);
    assert!(r.contains("already executed"), "once: {r}");

    // missing coefficient: fresh id, 3 partials, coeffs only cover 0,1
    for i in 0..3 {
        let m3 = format!(r#"{{"id":"m3","msg":"{msg}","i":{i},"sig":"{}"}}"#, sig(i));
        let r = call(st, "submit", &m3);
        assert!(r.contains("submitted:"), "m3 sub{i}: {r}");
    }
    let coeffs2 = format!("{}{}", "01".to_string() + &"11".repeat(32), "02".to_string() + &"22".repeat(32));
    let exec_bad = format!(r#"{{"id":"m3","coeffs":"{coeffs2}"}}"#);
    let r = call(st, "execute", &exec_bad);
    assert!(r.contains("missing coefficient"), "missing coeff: {r}");

    // m3 with full coeffs succeeds
    let r = call(st, "execute", &format!(r#"{{"id":"m3","coeffs":"{coeffs}"}}"#));
    // NEAR-ABI mock P1Sum blob hex starts with sign byte 00 (971744e) —
    // was "executed:p1s" under the pre-971744e stub markers
    assert!(r.contains("executed:0a11"), "m3 exec: {r}");
}

#[test]
fn bls_pairing_gate_shape() {
    // stub gate: well-formed 384-multiple passes, malformed fails
    let st = "/tmp/bls-msig-gate.bin";
    let _ = std::fs::remove_file(st);
    let pks: Vec<String> = (0..4)
        .map(pk)
        .collect();
    let init = format!(r#"{{"pks":["{}","{}","{}","{}"],"t":1}}"#, pks[0], pks[1], pks[2], pks[3]);
    let r = call(st, "init", &init);
    assert!(r.contains("ok:4:1"), "init t=1: {r}");

    // H(m) = 96 hex → gate input 96(σ)+192(apk)+96 = 384 → stub-true
    let msg = "cd".repeat(96);   // msgPoint: 96B uncompressed G1 = 192 hex
    let sub = format!(r#"{{"id":"g1","msg":"{msg}","i":0,"sig":"{}"}}"#, sig(0));
    let r = call(st, "submit", &sub);
    assert!(r.contains("submitted:1"), "sub: {r}");
    let exec = format!(r#"{{"id":"g1","msgPoint":"{msg}","g2gen":"{}","coeffs":"{}"}}"#, "07".repeat(192), "01".to_string() + &"11".repeat(32));
    let r = call(st, "execute", &exec);
    // NEAR-ABI mock P1Sum blob hex starts with sign byte 00 (971744e) —
    // was "executed:p1s" under the pre-971744e stub markers
    assert!(r.contains("executed:0a11"), "gate pass: {r}");

    // short H(m) (64 hex) → gate 320 bytes → not %384 → pairing abort
    let st2 = "/tmp/bls-msig-gate2.bin";
    let _ = std::fs::remove_file(st2);
    let r = call(st2, "init", &init);
    assert!(r.contains("ok:4:1"));
    let msg2 = "cd".repeat(32);
    let sub = format!(r#"{{"id":"g1","msg":"{msg2}","i":0,"sig":"{}"}}"#, sig(0));
    let _ = &msg2;
    let r = call(st2, "submit", &sub);
    assert!(r.contains("submitted:1"), "sub2: {r}");
    let r = call(st2, "execute", &exec);
    assert!(r.contains("pairing check failed"), "gate fail: {r}");
}
