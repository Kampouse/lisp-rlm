//! STATE IMPORT — taking part of an account's on-chain trie as initial
//! mock state. Pipeline:
//!   scripts/fetch_near_state.sh <account> <out.json>   (live RPC, b64)
//!   near-mock state import <state.bin> <out.json>      [--replace-acct]
//!   near-mock state dump <state.bin> [acct-prefix]     (inspect, b64)
//!   near-mock cross  <state.bin> ...                   (contracts see it)
//!
//! These tests prove the full chain minus the live fetch: RPC-shaped b64
//! dump -> namespaced trie -> contract-visible storage.

use std::io::Write;
use std::process::Command;
use std::sync::OnceLock;

fn nm() -> &'static str {
    "./target/release/near-mock"
}

/// wat2wasm the trap fixture once (same pattern as test_scenario_meta).
fn trap_stub_wasm() -> Option<std::path::PathBuf> {
    static OUT: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    OUT.get_or_init(|| {
        let wat =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trap_stub.wat");
        let out =
            std::env::temp_dir().join(format!("trap_stub_import_{}.wasm", std::process::id()));
        let ok = Command::new("wat2wasm")
            .arg(&wat)
            .arg("-o")
            .arg(&out)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            Some(out)
        } else {
            eprintln!("wat2wasm unavailable or failed — state-import tests skipped");
            None
        }
    })
    .clone()
}

fn next_tag() -> usize {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// base64 (RPC wire format)
fn b64(s: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

struct Run {
    ok: bool,
    text: String,
}

fn run(args: &[&str], stdin_dump: Option<&str>) -> Run {
    let mut cmd = Command::new(nm());
    cmd.args(args);
    if let Some(dump) = stdin_dump {
        cmd.arg("-"); // read dump from stdin
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("near-mock should spawn");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(dump.as_bytes())
            .unwrap();
        let out = child.wait_with_output().expect("near-mock should finish");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        return Run {
            ok: out.status.success(),
            text,
        };
    }
    let out = cmd.output().expect("near-mock should run");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Run {
        ok: out.status.success(),
        text,
    }
}

fn rpc_dump(account: &str, values: &[(&[u8], &[u8])]) -> String {
    let vals: Vec<String> = values
        .iter()
        .map(|(k, v)| format!("{{\"key\": \"{}\", \"value\": \"{}\"}}", b64(k), b64(v)))
        .collect();
    format!(
        "{{\"account\": \"{}\", \"block_height\": 123456, \"values\": [{}]}}",
        account,
        vals.join(", ")
    )
}

const ACCT: &str = "chain.kampy.test.near";

#[test]
fn import_dump_contract_read_and_absence() {
    let Some(wasm) = trap_stub_wasm() else { return };

    // -- import a single RPC-shaped key and dump it back --------------------
    let tag = format!("imp_{}_{}", std::process::id(), next_tag());
    let state = std::env::temp_dir().join(format!("nm_{tag}.bin"));
    let _ = std::fs::remove_file(&state);

    let r = run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&rpc_dump(ACCT, &[(b"ka", b"from-chain")])),
    );
    assert!(r.ok, "import failed: {}", r.text);
    assert!(r.text.contains("imported 1 keys"), "count: {}", r.text);

    let r = run(&["state", "dump", state.to_str().unwrap(), ACCT], None);
    assert!(r.ok, "dump failed: {}", r.text);
    assert!(
        r.text.contains(&b64(b"from-chain")),
        "value round-trips as b64: {}",
        r.text
    );

    // -- a CONTRACT sees the imported key ------------------------------------
    let r = run(
        &[
            "cross",
            state.to_str().unwrap(),
            &format!("{ACCT}={}", wasm.display()),
            ACCT,
            "read",
            "{}",
            "--view",
        ],
        None,
    );
    assert!(r.ok, "cross failed: {}", r.text);
    assert!(
        r.text.contains("📄 yes"),
        "contract must observe imported state: {}",
        r.text
    );

    // -- absence: empty partition -> contract reports "no" -------------------
    let tag2 = format!("imp_{}_{}", std::process::id(), next_tag());
    let state2 = std::env::temp_dir().join(format!("nm_{tag2}.bin"));
    let _ = std::fs::remove_file(&state2);
    let r = run(
        &["state", "import", state2.to_str().unwrap(), "-"],
        Some(&rpc_dump(ACCT, &[])),
    );
    assert!(r.ok, "empty import failed: {}", r.text);

    let r = run(
        &[
            "cross",
            state2.to_str().unwrap(),
            &format!("{ACCT}={}", wasm.display()),
            ACCT,
            "read",
            "{}",
            "--view",
        ],
        None,
    );
    assert!(r.ok, "cross failed: {}", r.text);
    assert!(
        r.text.contains("📄 no"),
        "contract must observe ABSENCE too: {}",
        r.text
    );
}

#[test]
fn import_merges_and_replace_acct_wipes_partition() {
    let Some(wasm) = trap_stub_wasm() else { return };
    let _ = wasm;

    let tag = format!("imp_{}_{}", std::process::id(), next_tag());
    let state = std::env::temp_dir().join(format!("nm_{tag}.bin"));
    let _ = std::fs::remove_file(&state);

    // two accounts, two keys
    run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&rpc_dump(ACCT, &[(b"k1", b"v1")])),
    );
    run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&rpc_dump("other.test.near", &[(b"k2", b"v2")])),
    );

    // merge (default): same account, NEW key — old key must survive
    let r = run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&rpc_dump(ACCT, &[(b"k3", b"v3")])),
    );
    assert!(r.ok, "merge import failed: {}", r.text);
    let r = run(&["state", "dump", state.to_str().unwrap()], None);
    for (needle, what) in [
        (&b64(b"v1").to_string(), "k1 survived merge"),
        (&b64(b"v3").to_string(), "k3 landed"),
        (&b64(b"v2").to_string(), "other account untouched"),
    ] {
        assert!(r.text.contains(needle), "{what}: {}", r.text);
    }

    // --replace-acct: ACCT's partition drops (k1,k3 gone), other survives
    let r = run(
        &[
            "state",
            "import",
            state.to_str().unwrap(),
            "-",
            "--replace-acct",
        ],
        Some(&rpc_dump(ACCT, &[(b"k9", b"v9")])),
    );
    assert!(r.ok, "replace import failed: {}", r.text);
    assert!(
        r.text.contains("partition replaced"),
        "flag notice: {}",
        r.text
    );
    let r = run(&["state", "dump", state.to_str().unwrap()], None);
    assert!(
        !r.text.contains(&b64(b"v1")),
        "k1 must be wiped: {}",
        r.text
    );
    assert!(
        !r.text.contains(&b64(b"v3")),
        "k3 must be wiped: {}",
        r.text
    );
    assert!(r.text.contains(&b64(b"v9")), "k9 landed: {}", r.text);
    assert!(
        r.text.contains(&b64(b"v2")),
        "other account untouched: {}",
        r.text
    );
}

#[test]
fn import_rejects_malformed_dump_atomically() {
    let tag = format!("imp_{}_{}", std::process::id(), next_tag());
    let state = std::env::temp_dir().join(format!("nm_{tag}.bin"));
    let _ = std::fs::remove_file(&state);

    // good key first
    run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&rpc_dump(ACCT, &[(b"good", b"v")])),
    );

    // malformed: bad base64 in entry 2 — must fail WITHOUT half-applying
    let bad = format!(
        "{{\"account\": \"{ACCT}\", \"values\": [{{\"key\": \"{}\", \"value\": \"{}\"}}, {{\"key\": \"!!!not-b64!!!\", \"value\": \"{}\"}}]}}",
        b64(b"a"),
        b64(b"x"),
        b64(b"y")
    );
    let r = run(
        &["state", "import", state.to_str().unwrap(), "-"],
        Some(&bad),
    );
    assert!(!r.ok, "malformed dump must fail: {}", r.text);
    assert!(r.text.contains("bad base64"), "reason surfaced: {}", r.text);

    // state unchanged
    let r = run(&["state", "dump", state.to_str().unwrap()], None);
    assert!(
        !r.text.contains(&b64(b"x")),
        "entry 1 must NOT have half-applied: {}",
        r.text
    );
    assert!(
        r.text.contains(&b64(b"v")),
        "original key intact: {}",
        r.text
    );
}

#[test]
fn dump_reports_missing_and_corrupt_files() {
    let tag = format!("imp_{}_{}", std::process::id(), next_tag());
    let missing = std::env::temp_dir().join(format!("nm_{tag}_nope.bin"));

    let r = run(&["state", "dump", missing.to_str().unwrap()], None);
    assert!(!r.ok, "missing file must fail");
    assert!(
        r.text.contains("create one via a scenario or import"),
        "hint: {}",
        r.text
    );

    let corrupt = std::env::temp_dir().join(format!("nm_{tag}_bad.bin"));
    std::fs::write(&corrupt, b"this is not bincode").unwrap();
    let r = run(&["state", "dump", corrupt.to_str().unwrap()], None);
    assert!(!r.ok, "corrupt file must fail");
    assert!(
        r.text.contains("not a near-mock state file"),
        "reason: {}",
        r.text
    );
}
