//! HTLC escrow — hashlock + timelock state machine (2026-09-01).
//!
//! Found and fixed bug #12 on the way in: sha256Hash used to return
//! RAW 32-byte digests aliasing a fixed scratch — binary bytes inside
//! json-set records derailed __json_set's scanner (silent fresh-object
//! fallback) and two live digests overwrote each other. Digests are
//! now hex strings (64 chars, heap-allocated, scanner-safe).

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const SRC: &str = include_str!("../fixtures/escrow.ts");

fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let m = LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn run(wasm_path: &str, method: &str, input: &str, signer: Option<&str>, ts: Option<i64>) -> String {
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg(wasm_path).arg(method).arg(input);
    if let Some(s) = signer { cmd.env("NEAR_MOCK_SIGNER", s); }
    if let Some(t) = ts { cmd.env("NEAR_MOCK_BLOCK_TS", t.to_string()); }
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn compile(src: &str, tag: &str) -> String {
    let ir = ts_to_lisp_source(src).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).expect("compile");
    let tmp = std::env::temp_dir().join(format!("nm_htlc_{}_{}.wasm", tag, std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    tmp.to_str().unwrap().to_string()
}

#[test]
fn htlc_full_lifecycle() {
    let _lock = state_lock();
    let _ = std::fs::remove_file(lisp_rlm_wasm::near_mock_state_file());
    let w = compile(SRC, "main");
    let u: u128 = 10u128.pow(18);
    let (mint, esc1, esc2) = (2_000_000*u, 500_000*u, 100_000*u);
    let ts1: i64 = 1_800_000_000_000_000_000;
    let ts2: i64 = 1_800_000_600_000_000_001; // 1ns past escrow#1 timeout
    let ts3: i64 = 1_800_000_123_456_789_123; // NON-ROUND: f64-unrepresentable

    assert!(run(&w, "ftMint", &format!(r#"{{"to":"owner.test.near","amount":"{mint}"}}"#), Some("owner.test.near"), Some(ts1)).contains(&format!("supply:{mint}")));
    assert!(run(&w, "escrowNew", &format!(r#"{{"recipient":"alice.test.near","secret":"s3cr3t-demo","amount":"{esc1}","timeoutSec":"600"}}"#), Some("owner.test.near"), Some(ts1)).contains("escrow:1"));
    assert!(run(&w, "escrowClaim", r#"{"id":"1","secret":"wrong"}"#, Some("alice.test.near"), Some(ts1)).contains("wrong secret"));
    assert!(run(&w, "escrowClaim", r#"{"id":"1","secret":"s3cr3t-demo"}"#, Some("bob.test.near"), Some(ts1)).contains("only the recipient may claim"));
    assert!(run(&w, "escrowRefund", r#"{"id":"1"}"#, Some("owner.test.near"), Some(ts1)).contains("not yet timed out"));
    assert!(run(&w, "escrowClaim", r#"{"id":"1","secret":"s3cr3t-demo"}"#, Some("alice.test.near"), Some(ts1)).contains(&format!("claimed:{esc1}")));
    assert!(run(&w, "ftBalanceOf", r#"{"who":"alice.test.near"}"#, None, Some(ts1)).contains(&format!("📄 {esc1}")));
    assert!(run(&w, "escrowClaim", r#"{"id":"1","secret":"s3cr3t-demo"}"#, Some("alice.test.near"), Some(ts1)).contains("not pending"));
    assert!(run(&w, "escrowRefund", r#"{"id":"1"}"#, Some("owner.test.near"), Some(ts2)).contains("not pending"));
    // escrow#2: NON-ROUND timestamp + 1s timeout — exactness proof
    assert!(run(&w, "escrowNew", &format!(r#"{{"recipient":"bob.test.near","secret":"x2","amount":"{esc2}","timeoutSec":"1"}}"#), Some("owner.test.near"), Some(ts3)).contains("escrow:2"));
    assert!(run(&w, "escrowClaim", r#"{"id":"2","secret":"x2"}"#, Some("bob.test.near"), Some(ts3 + 1_000_000_001)).contains("timed out"));
    assert!(run(&w, "escrowRefund", r#"{"id":"2"}"#, Some("owner.test.near"), Some(ts3 + 1_000_000_001)).contains(&format!("refunded:{esc2}")));
    assert!(run(&w, "ftBalanceOf", r#"{"who":"owner.test.near"}"#, None, Some(ts3)).contains(&format!("📄 {}", mint - esc1)));
}

#[test]
fn sha256_hash_is_hex_and_exact() {
    // digests machine-derived via python hashlib
    let src = r#"
        export function t(secret: string): string {
          return near.sha256Hash(secret);
        }
    "#;
    let w = compile(src, "hex");
    let _lock = state_lock();
    let _ = std::fs::remove_file(lisp_rlm_wasm::near_mock_state_file());
    for (input, digest) in [
        ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        ("s3cr3t-demo", "4821d08ebe7bae764da7fe2488b251b25b113280ac2eb8a02fcfdf2946f139ad"),
    ] {
        let out = run(&w, "t", &format!(r#"{{"secret":"{input}"}}"#), None, None);
        assert!(out.contains(digest), "sha256({input:?}): {out}");
    }
}

#[test]
fn chained_jsonset_with_hash_no_longer_loses_keys() {
    // regression for bug #12: raw-binary digests derailed __json_set's
    // scanner → the next chained set returned a fresh {"amt":…} object,
    // silently dropping every earlier key. Hex digests are scanner-safe.
    let src = include_str!("../fixtures/repro_jsonset_chain_aliasing.ts");
    let w = compile(src, "r12");
    let _lock = state_lock();
    let _ = std::fs::remove_file(lisp_rlm_wasm::near_mock_state_file());
    let out = run(&w, "t", r#"{"recipient":"alice.test.near","secret":"s3cr3t-demo","amount":"500000000000000000000000","timeoutSec":"600"}"#, Some("owner.test.near"), Some(1_800_000_000_000_000_000));
    // 2026-09-01: jsonSet now self-encodes values (bug #22) — records are
    // VALID JSON, so values are quoted. Keys-present assertions unchanged.
    assert!(out.contains(r#""sender":"owner.test.near""#), "record lost its keys: {out}");
    assert!(out.contains(r#""hashlock":"4821d08e"#), "hashlock not hex-embedded: {out}");
}
