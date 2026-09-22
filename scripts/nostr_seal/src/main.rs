//! Sealed-root Nostr worker — Rust sidecar (production storage path).
//!
//! Root: generated on first init from host entropy, sealed in worker storage
//! (encrypted, worker-private, never in input/output). Per-user BIP-340 keys
//! derived in-worker: sk = SHA256(root || 0x1f || caller), reduced into [1,n).
//!
//! stdin JSON ops:
//!   {"op":"init"}                       -> {"root_commitment":"..."}  (idempotent)
//!   {"op":"derive","caller":"alice"}    -> {"caller":"alice","pk":"..."}
//!   {"op":"sign","caller":"alice","ts":"...","kind":"1","content":"..."}
//!                                       -> {"id":"...","pubkey":"...","sig":"..."}

use outlayer::storage;
use sha2::{Digest, Sha256};

const ROOT_KEY: &str = "nostr:root:v1";

fn main() {
    let mut input = String::new();
    use std::io::Read;
    std::io::stdin().read_to_string(&mut input).expect("stdin");
    let v: serde_json::Value =
        serde_json::from_str(input.trim()).unwrap_or(serde_json::json!({}));
    let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("");

    let out = match op {
        "init" => op_init(),
        "claim" => op_claim(),
        "derive" => op_derive(v.get("caller").and_then(|x| x.as_str()).unwrap_or("")),
        "sign" => op_sign(
            v.get("caller").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("ts").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("kind").and_then(|x| x.as_str()).unwrap_or("1"),
            v.get("content").and_then(|x| x.as_str()).unwrap_or(""),
        ),
        _ => err("unknown op; use init | derive | sign"),
    };
    println!("{}", out);
}

fn err(msg: &str) -> String {
    serde_json::json!({ "error": msg }).to_string()
}

/// init: create root once (race-free via set_if_absent), output commitment only.
fn op_init() -> String {
    // placeholder reservation; if we won the race, write the real root
    let inserted = match storage::set_if_absent(ROOT_KEY, b"__init__") {
        Ok(v) => v,
        Err(e) => return err(&e.0),
    };
    if inserted {
        let root = rand_root();
        if let Err(e) = storage::set_worker(ROOT_KEY, &root) {
            return err(&e.0);
        }
    }
    let root = match storage::get_worker(ROOT_KEY) {
        Ok(Some(b)) if b.len() == 32 => b,
        _ => {
            // crashed earlier between reserve and write: repair now
            let root = rand_root();
            if let Err(e) = storage::set_worker(ROOT_KEY, &root) {
                return err(&e.0);
            }
            root.to_vec()
        }
    };
    let com = hex(&Sha256::digest(&root));
    serde_json::json!({ "root_commitment": com }).to_string()
}

/// derive: per-user binding — output ONLY public material.
fn op_derive(caller: &str) -> String {
    let Some(root) = read_root() else {
        return err("no root: run init first");
    };
    let (pk_hex, _sk) = derive_user(&root, caller);
    serde_json::json!({ "caller": caller, "pk": pk_hex }).to_string()
}

/// sign: re-derive sk in-enclave from sealed root; output id+pk+sig only.
fn op_sign(caller: &str, ts: &str, kind: &str, content: &str) -> String {
    let Some(root) = read_root() else {
        return err("no root: run init first");
    };
    let (pk_hex, sk) = derive_user(&root, caller);

    // NIP-01 event id serialization: [0,pubkey,created_at,kind,tags,content]
    let ser = serde_json::json!([0u8, pk_hex, ts, kind, [], content]).to_string();
    let id: [u8; 32] = Sha256::digest(ser.as_bytes()).into();

    // deterministic BIP-340 (aux = 32 zero bytes, the "empty aux" spec convention)
    let sig = match sk.sign_raw(&id, &[0u8; 32]) {
        Ok(s) => s,
        Err(e) => return err(&format!("sign failed: {e}")),
    };
    let sig_bytes: [u8; 64] = sig.to_bytes().into();

    // self-verify (matching raw pair) before output
    use k256::schnorr::signature::hazmat::PrehashVerifier;
    if sk.verifying_key().verify_prehash(&id, &sig).is_err() {
        return err("internal: signature failed self-verify");
    }

    serde_json::json!({
        "id": hex(&id),
        "pubkey": pk_hex,
        "sig": hex(&sig_bytes),
    })
    .to_string()
}

/// claim: read NOSTR_ROOT from injected env (secrets channel), seed storage.
/// One-time bootstrap; afterwards storage-first keeps the root stable.
fn op_claim() -> String {
    match std::env::var("NOSTR_ROOT") {
        Ok(s) if s.len() == 64 => {
            let root = match (0..32)
                .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
            {
                Ok(b) => b,
                Err(_) => return err("NOSTR_ROOT is not valid hex"),
            };
            match storage::set_worker(ROOT_KEY, &root) {
                Ok(()) => {
                    let com = hex(&Sha256::digest(&root));
                    serde_json::json!({ "claimed": true, "root_commitment": com }).to_string()
                }
                Err(e) => err(&e.0),
            }
        }
        Ok(_) => err("NOSTR_ROOT must be 64 hex chars"),
        Err(_) => err("NOSTR_ROOT not found in env (set project secret first)"),
    }
}

// ---------- internals ----------

fn read_root() -> Option<Vec<u8>> {
    match storage::get_worker(ROOT_KEY) {
        Ok(Some(b)) if b.len() == 32 => Some(b),
        _ => None,
    }
}

/// KDF: sk = SHA256(root || 0x1f || caller); re-hash with counter in the
/// astronomically rare case the bytes are not a valid scalar.
fn derive_user(root: &[u8], caller: &str) -> (String, k256::schnorr::SigningKey) {
    let mut h = Sha256::new();
    h.update(root);
    h.update([0x1f]);
    h.update(caller.as_bytes());
    let mut bytes: [u8; 32] = h.finalize().into();
    let sk = loop {
        if let Ok(k) = k256::schnorr::SigningKey::from_bytes(&bytes.into()) {
            break k;
        }
        let mut h2 = Sha256::new();
        h2.update(bytes);
        h2.update(0u32.to_le_bytes());
        bytes = h2.finalize().into();
    };
    // x-only pubkey (BIP-340) straight from the verifying key
    let pk_hex = hex(sk.verifying_key().to_bytes().as_slice());
    (pk_hex, sk)
}

fn rand_root() -> [u8; 32] {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("host entropy");
    buf
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
