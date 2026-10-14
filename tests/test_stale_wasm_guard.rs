//! Stale-wasm guard (2026-10-10): near-compile stamps every emitted wasm
//! with the sha256 of its ORIGINAL (pre-lowering) source; near-mock warns —
//! never blocks — when that source's hash has moved.
//!
//! Guards the trap where a failed rebuild leaves an OLD wasm beside the
//! project and every later near-mock run silently tests dead code (cost a
//! real session 3 phantom-red scenario suites on 2026-10-09).

use std::path::PathBuf;
use std::process::Command;

/// near-compile is a SEPARATE workspace package, so its binary is not
/// exposed via CARGO_BIN_EXE_* (that only covers the package under test).
/// Locate it the way the regression harness does — pinned target dir.
fn near_compile_bin() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/near-compile");
    assert!(
        p.exists(),
        "{} not found — run: cargo build --release -p near-compile",
        p.display()
    );
    p
}

#[test]
fn stale_wasm_guard_warns_on_edited_source() {
    let dir = std::env::temp_dir().join(format!("stale_guard_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");

    let near_json = r#"{ "name": "staleguard", "src": "src/probe.ts", "output": "probe.wasm", "target": "near" }"#;
    std::fs::write(dir.join("near.json"), near_json).expect("write near.json");
    std::fs::write(
        dir.join("src/probe.ts"),
        "export function run(): string {\n  return \"v1\";\n}\n",
    )
    .expect("write probe");

    // Build v1 → stamp must exist and name the source.
    let out = Command::new(near_compile_bin())
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("run near-compile build");
    assert!(
        out.status.success(),
        "build v1 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stamp_raw = std::fs::read_to_string(dir.join("probe.wasm.stamp")).expect("stamp written");
    let stamp: serde_json::Value = serde_json::from_str(&stamp_raw).expect("stamp is JSON");
    assert!(
        stamp["src"].as_str().unwrap_or("").ends_with("probe.ts"),
        "stamp src field: {stamp_raw}"
    );
    assert_eq!(
        stamp["sha256"].as_str().unwrap_or("").len(),
        64,
        "sha256 hex len"
    );

    let run_mock = || {
        Command::new(env!("CARGO_BIN_EXE_near-mock"))
            .current_dir(&dir)
            .args(["probe.wasm", "run"])
            .output()
            .expect("run near-mock")
    };

    // Fresh build → silent.
    let out = run_mock();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains("STALE"), "fresh build must not warn: {all}");
    assert!(all.contains("v1"), "fresh run returns v1: {all}");

    // Edit source WITHOUT rebuilding → warn, but still run (warn-not-block).
    std::fs::write(
        dir.join("src/probe.ts"),
        "export function run(): string {\n  return \"v2\";\n}\n",
    )
    .expect("edit probe");
    let out = run_mock();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("STALE WASM"),
        "must warn on stale wasm: {stderr}"
    );
    assert!(
        out.status.success() && stdout.contains("v1"),
        "stale run still executes OLD build: {stdout}"
    );

    // Rebuild → fresh again.
    let out = Command::new(near_compile_bin())
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("rebuild");
    assert!(out.status.success(), "rebuild failed");
    let out = run_mock();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains("STALE"), "rebuilt wasm must not warn: {all}");
    assert!(all.contains("v2"), "rebuilt run returns v2: {all}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Legacy single-file path (`near-compile x.ts out.wasm`) stamps too, and
/// the stamp hashes the TS source, not the lowered lisp.
#[test]
fn legacy_compile_path_stamps_original_source() {
    let dir = std::env::temp_dir().join(format!("stale_guard_legacy_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let ts = dir.join("probe.ts");
    std::fs::write(
        &ts,
        "export function run(): string {\n  return \"legacy\";\n}\n",
    )
    .expect("write");

    let wasm = dir.join("legacy.wasm");
    let out = Command::new(near_compile_bin())
        .args([ts.to_str().unwrap(), wasm.to_str().unwrap()])
        .output()
        .expect("run compile");
    assert!(
        out.status.success(),
        "legacy compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stamp_raw =
        std::fs::read_to_string(dir.join("legacy.wasm.stamp")).expect("legacy stamp written");
    let stamp: serde_json::Value = serde_json::from_str(&stamp_raw).expect("stamp is JSON");
    assert!(
        stamp["src"].as_str().unwrap_or("").ends_with("probe.ts"),
        "stamp names the TS source: {stamp_raw}"
    );

    // Stale check fires through the legacy path as well.
    std::fs::write(
        &ts,
        "export function run(): string {\n  return \"edited\";\n}\n",
    )
    .expect("edit");
    let out = Command::new(env!("CARGO_BIN_EXE_near-mock"))
        .current_dir(&dir)
        .args(["legacy.wasm", "run"])
        .output()
        .expect("run near-mock");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("STALE WASM"),
        "legacy path must warn: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
