//! REAL-CONTRACT scenario fixtures — the chaos runner driven by actual
//! compiled contracts, not hand-written WAT:
//!   1. flashpool/flashborrower (fixtures/*.ts → near wasm via the repo
//!      toolchain): honest flash loan settles (receipt chain
//!      transfer→callback→settle incl. child-returned-promise ordering);
//!      a STIFF loan is an inverted tripwire — the scenario MUST exit
//!      nonzero with the pool short (transfer receipt committed) and the
//!      storage fork diverged from the post-honest snapshot.
//!   2. guestbook (a real near-sdk Rust contract, register ABI): sign /
//!      count / list, then snapshot→sign→restore proves state forking on
//!      SDK storage that near-mock runs as opaque key/value bytes.
//!
//! guestbook test skips unless the wasm exists (set GUESTBOOK_WASM or the
//! default ~/.openclaw workspace path).

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn has_near_mock() -> bool {
    Command::new("./target/release/near-mock")
        .args(["--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn next_tag() -> usize {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Compile a TS fixture to NEAR-target wasm via the repo toolchain.
fn compile_ts_fixture(ts_path: &str) -> Vec<u8> {
    let src =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ts_path))
            .unwrap_or_else(|e| panic!("read {ts_path}: {e}"));
    let ir = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src).expect("ts→lisp");
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("compile near wasm")
}

/// Run a scenario against a manifest of account=wasm paths.
/// Returns (exit_success, combined output).
fn run_scenario_json(name: &str, manifest: &str, steps: &str) -> (bool, String) {
    let tag = format!("fl_{}_{}_{}", name, std::process::id(), next_tag());
    let state = std::env::temp_dir().join(format!("nm_{tag}_state.bin"));
    let _ = std::fs::remove_file(&state);
    let scenario = std::env::temp_dir().join(format!("nm_{tag}.json"));
    std::fs::write(
        &scenario,
        format!(
            r#"{{"name": "{tag}", "manifest": "{}", "state": "{}", "steps": {}}}"#,
            manifest,
            state.display(),
            steps
        ),
    )
    .unwrap();
    let out = Command::new("./target/release/near-mock")
        .args(["scenario"])
        .arg(&scenario)
        .output()
        .expect("near-mock should run");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn flashloan_manifest() -> (String, String) {
    let pool = compile_ts_fixture("fixtures/flashpool.ts");
    let borrower = compile_ts_fixture("fixtures/flashborrower.ts");
    let tag = format!("fl_{}", std::process::id());
    let pool_p = std::env::temp_dir().join(format!("{tag}_pool.wasm"));
    let bor_p = std::env::temp_dir().join(format!("{tag}_borrower.wasm"));
    std::fs::write(&pool_p, &pool).unwrap();
    std::fs::write(&bor_p, &borrower).unwrap();
    let manifest = format!(
        "pool.c.test.near={},borrower.c.test.near={}",
        pool_p.display(),
        bor_p.display()
    );
    (manifest, tag)
}

const DEPOSITS: &str = r#"[
    {"contract": "pool.c.test.near", "method": "deposit", "attach": "400", "expect": "pool:400"},
    {"contract": "pool.c.test.near", "method": "deposit", "attach": "300", "expect": "pool:700"},
    {"contract": "pool.c.test.near", "method": "deposit", "attach": "300", "expect": "pool:1000"},
    {"contract": "borrower.c.test.near", "method": "deposit", "attach": "10", "expect": "borrower:10"},"#;

#[test]
fn flashloan_honest_scenario_passes() {
    if !has_near_mock() {
        return;
    }
    let _l = lock();
    let (manifest, _tag) = flashloan_manifest();
    let steps = format!(
        r#"{DEPOSITS}
    {{"contract": "pool.c.test.near", "method": "flashLoan", "args": {{"amount": "500", "borrower": "borrower.c.test.near"}}, "expect": "settled:"}},
    {{"contract": "borrower.c.test.near", "method": "lastBorrow", "view": true, "contains": "500"}},
    {{"contract": "pool.c.test.near", "method": "balance", "view": true, "contains": "1005"}}
  ]"#
    );
    let (ok, out) = run_scenario_json("honest", &manifest, &steps);
    assert!(ok, "honest flash-loan scenario must pass: {out}");
    assert!(out.contains("settled:1005"), "settle receipt output: {out}");
    assert!(
        out.contains("3 pass / 0 fail") || out.contains("pass / 0 fail"),
        "counters: {out}"
    );
}

#[test]
fn flashloan_stiff_scenario_diverges_tripwire() {
    if !has_near_mock() {
        return;
    }
    let _l = lock();
    let (manifest, _tag) = flashloan_manifest();
    // honest loan first (committed baseline), snapshot, then stiff:
    // the pool goes short (transfer receipt committed), the settle aborts,
    // and storage MUST diverge from the snapshot (theft leaves residue).
    // The scenario exiting nonzero IS the regression signal — if storage
    // does NOT diverge, the tripwire test below fails.
    let steps = format!(
        r#"{DEPOSITS}
    {{"contract": "pool.c.test.near", "method": "flashLoan", "args": {{"amount": "500", "borrower": "borrower.c.test.near"}}, "expect": "settled:"}},
    {{"snapshot": "post-honest"}},
    {{"contract": "borrower.c.test.near", "method": "goStiff", "expect": "stiff-on"}},
    {{"contract": "pool.c.test.near", "method": "flashLoan", "args": {{"amount": "600", "borrower": "borrower.c.test.near"}}}},
    {{"contract": "borrower.c.test.near", "method": "lastBorrow", "view": true, "contains": "500"}},
    {{"contract": "pool.c.test.near", "method": "balance", "view": true, "contains": "405"}},
    {{"expect_same_storage_as": "post-honest"}}
  ]"#
    );
    let (ok, out) = run_scenario_json("stiff", &manifest, &steps);
    assert!(!ok, "stiff scenario must FAIL (divergence tripwire): {out}");
    assert!(
        out.contains("DIVERGED from snapshot 'post-honest'"),
        "theft must leave residue: {out}"
    );
    assert!(
        out.contains("405"),
        "pool short (transfer committed): {out}"
    );
    assert!(
        out.contains("flash loan not repaid"),
        "settle abort message must surface: {out}"
    );
    assert!(
        out.contains("settled:1005"),
        "honest baseline settled first: {out}"
    );
}

fn guestbook_wasm_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("GUESTBOOK_WASM") {
        let pb = std::path::PathBuf::from(p);
        return pb.exists().then_some(pb);
    }
    let default = std::path::PathBuf::from(format!(
        "{}/.openclaw/workspace/guestbook-contract/target/wasm32-unknown-unknown/release/guestbook.wasm",
        std::env::var("HOME").unwrap_or_default()
    ));
    default.exists().then_some(default)
}

#[test]
fn guestbook_scenario_forks_and_restores_real_sdk_contract() {
    if !has_near_mock() {
        return;
    }
    let Some(gb) = guestbook_wasm_path() else {
        eprintln!("guestbook.wasm not found (set GUESTBOOK_WASM) — skipping");
        return;
    };
    let _l = lock();
    let manifest = format!("gb.test.near={}", gb.display());
    let steps = r#"[
    {"contract": "gb.test.near", "method": "sign", "args": {"message": "hello from near-mock"}, "expect": "ok"},
    {"contract": "gb.test.near", "method": "sign", "args": {"message": "second entry"}, "expect": "ok"},
    {"contract": "gb.test.near", "method": "get_signature_count", "view": true, "expect": "2"},
    {"snapshot": "two-sigs"},
    {"contract": "gb.test.near", "method": "sign", "args": {"message": "third entry"}, "expect": "ok"},
    {"contract": "gb.test.near", "method": "get_signature_count", "view": true, "expect": "3"},
    {"restore": "two-sigs"},
    {"contract": "gb.test.near", "method": "get_signature_count", "view": true, "expect": "2"},
    {"expect_same_storage_as": "two-sigs"}
  ]"#;
    let (ok, out) = run_scenario_json("guestbook", &manifest, steps);
    assert!(ok, "guestbook fork scenario must pass: {out}");
    assert!(
        out.contains("♻️ restored 'two-sigs'"),
        "restore line: {out}"
    );
    assert!(
        out.contains("storage identical to snapshot 'two-sigs'"),
        "fork compare: {out}"
    );
    assert!(
        out.contains("hello from near-mock") || out.contains("sign"),
        "sigs flowed: {out}"
    );
}
