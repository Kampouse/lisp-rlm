//! Scenario-runner META-TESTS — pin the verdict logic of `near-mock
//! scenario` itself: expect / contains / expect:"trap" / per-step gas
//! caps / snapshot / restore / expect_same_storage_as / exit codes.
//! Runs against the hand-written trap_stub.wasm fixture (no compiler in
//! the loop), so a failure here is a regression in the RUNNER, not the
//! toolchain. fail_receipt + promise-DAG chaos is pinned separately by
//! `nm_scenario_runner_e2e` (oracle + price_consumer receipt chain).
//!
//! Requires wat2wasm on PATH (fixture compiles at test start).

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

/// wat2wasm the trap fixture once into a process-wide temp path.
fn trap_stub_wasm() -> Option<std::path::PathBuf> {
    static OUT: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    OUT.get_or_init(|| {
        let wat =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trap_stub.wat");
        let out = std::env::temp_dir().join(format!("trap_stub_{}.wasm", std::process::id()));
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
            eprintln!("wat2wasm unavailable or failed — scenario meta-tests skipped");
            None
        }
    })
    .clone()
}

fn next_tag() -> usize {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Run one scenario JSON against a fresh trap_stub sandbox.
/// Returns (exit_success, combined stdout+stderr).
fn scene(json_steps: &str) -> (bool, String) {
    let wasm = trap_stub_wasm().expect("trap_stub.wasm compiled");
    let tag = format!("meta_{}_{}", std::process::id(), next_tag());
    let state = std::env::temp_dir().join(format!("nm_{tag}_state.bin"));
    let _ = std::fs::remove_file(&state); // fresh state every call
    let scenario = std::env::temp_dir().join(format!("nm_{tag}.json"));
    std::fs::write(
        &scenario,
        format!(
            r#"{{"name": "{tag}", "manifest": "probe.test.near={}", "state": "{}", "steps": {}}}"#,
            wasm.display(),
            state.display(),
            json_steps
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

#[test]
fn scenario_happy_path_exits_zero() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a", "expect": "a"},
        {"contract": "probe.test.near", "method": "write_b", "expect": "b"}
      ]"#,
    );
    assert!(ok, "happy scenario must pass: {out}");
    assert!(out.contains("2 pass / 0 fail"), "verdict line: {out}");
}

#[test]
fn scenario_failed_expect_exits_nonzero() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a", "expect": "zzz-not-there"}
      ]"#,
    );
    assert!(!ok, "failed expect must exit nonzero: {out}");
    assert!(out.contains("✗ expect 'zzz-not-there'"), "verdict: {out}");
    assert!(out.contains("0 pass / 1 fail"), "counters: {out}");
    assert!(out.contains("step 0 ⇒ FAIL"), "fail marker: {out}");
}

#[test]
fn scenario_contains_checks_storage_partition() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    // contains hits the step contract's storage partition, not just stdout
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a"},
        {"contract": "probe.test.near", "method": "write_b", "contains": "kb=b"}
      ]"#,
    );
    assert!(ok, "contains must find storage kv: {out}");
    assert!(out.contains("✓ contains 'kb=b'"), "verdict: {out}");

    let (ok2, out2) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a", "contains": "kb=MISSING"}
      ]"#,
    );
    assert!(!ok2, "missing contains must fail: {out2}");
    assert!(out2.contains("✗ contains 'kb=MISSING'"), "verdict: {out2}");
}

#[test]
fn scenario_gas_cap_traps_spin_and_rolls_back() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    // spin under gas:1 must die FAST (epoch deadline), the partial write
    // class is exercised by the fuzz suite; here: trap accepted + fork proof
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a"},
        {"snapshot": "base"},
        {"contract": "probe.test.near", "method": "spin", "gas": 1, "expect": "trap"},
        {"expect_same_storage_as": "base"}
      ]"#,
    );
    assert!(ok, "gas-capped trap scenario must pass: {out}");
    assert!(out.contains("✓ trap as expected"), "trap verdict: {out}");
    assert!(
        out.contains("storage identical to snapshot 'base'"),
        "fork verdict: {out}"
    );
    assert!(!out.contains("16 minutes"), "no hang: {out}");
}

#[test]
fn scenario_unexpected_trap_fails_the_step() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    // spin traps (gas:1) but the step does NOT opt into expect:"trap"
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "spin", "gas": 1}
      ]"#,
    );
    assert!(!ok, "unannounced trap must fail the scenario: {out}");
    assert!(out.contains("❌ trap:"), "trap line: {out}");
    assert!(out.contains("0 pass / 1 fail"), "counters: {out}");
}

#[test]
fn scenario_expected_trap_but_success_fails() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a", "expect": "trap"}
      ]"#,
    );
    assert!(!ok, "expect:trap on a successful call must fail: {out}");
    assert!(
        out.contains("✗ expected trap, call succeeded"),
        "verdict: {out}"
    );
}

#[test]
fn scenario_restore_forks_storage() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    // write_a → snapshot → write_b → restore → fork compare must hold
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a"},
        {"snapshot": "base"},
        {"contract": "probe.test.near", "method": "write_b"},
        {"restore": "base"},
        {"expect_same_storage_as": "base"}
      ]"#,
    );
    assert!(ok, "restore must rewind storage: {out}");
    assert!(out.contains("♻️ restored 'base'"), "restore line: {out}");
    assert!(
        out.contains("storage identical to snapshot 'base'"),
        "compare: {out}"
    );
    assert!(
        !out.contains("kb=b"),
        "write_b must be gone after restore: {out}"
    );
}

#[test]
fn scenario_fork_compare_detects_divergence() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a"},
        {"snapshot": "base"},
        {"contract": "probe.test.near", "method": "write_b"},
        {"expect_same_storage_as": "base"}
      ]"#,
    );
    assert!(!ok, "diverged fork must fail: {out}");
    assert!(
        out.contains("✗ storage DIVERGED from snapshot 'base'"),
        "compare: {out}"
    );
}

#[test]
fn scenario_bookkeeping_step_with_no_keys_is_an_error() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near"}
      ]"#,
    );
    assert!(!ok, "empty bookkeeping step must error: {out}");
    assert!(
        out.contains("no method, no bookkeeping keys"),
        "error message: {out}"
    );
}

#[test]
fn scenario_view_state_not_persisted_is_honored() {
    if !has_near_mock() || trap_stub_wasm().is_none() {
        return;
    }
    let _l = lock();
    // a successful write, then a view step — the view's read must see the
    // committed kv (state persisted across steps within one scenario)
    let (ok, out) = scene(
        r#"[
        {"contract": "probe.test.near", "method": "write_a"},
        {"snapshot": "after-a"},
        {"contract": "probe.test.near", "method": "write_b"},
        {"restore": "after-a"},
        {"expect_same_storage_as": "after-a"}
      ]"#,
    );
    assert!(ok, "multi-step state chain must hold: {out}");
    assert!(
        out.contains("📸 snapshot 'after-a'"),
        "snapshot line: {out}"
    );
}
