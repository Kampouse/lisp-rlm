//! compile CLI data-loss guard (2026-09-15).
//!
//! The legacy `compile` binary's default output for .ts inputs was
//! `args[1].replace(".lisp", ".wasm")` — a NO-OP for .ts paths — so the
//! wasm AND the symbolication map were written straight over the user's
//! source file. `-o` was silently ignored in the same block (only
//! `--output` was recognized), which made the clobber reachable through
//! the most natural invocation:
//!     compile foo.ts --near -o out.wasm   ← destroyed foo.ts
//! Two fixture files were lost to this during the flashloan session
//! (recovered from git). Pinned here: output resolution, the
//! never-write-onto-input guard, and map sidecar placement.

use std::process::Command;

const SRC: &str = "export function ping(): string { return \"pong\"; }\n";

fn bin() -> Command {
    // CARGO_BIN_EXE_compile: cargo sets this for integration tests of a
    // package's bins automatically — it always points at the bin cargo
    // built for THIS test run ($CARGO_TARGET_DIR/debug/compile), so it
    // works under any target-dir override without a pre-built release
    // binary (TASK-DX-AGREE item 4; the task text's canonical fix).
    let mut c = Command::new(env!("CARGO_BIN_EXE_compile"));
    c.current_dir(env!("CARGO_MANIFEST_DIR"));
    c
}

#[test]
fn bin_helper_resolves_under_cargo_target_dir() {
    // Regression (item 4): the spawned compile binary must exist and live
    // inside the ACTIVE target dir, so the suite cannot depend on a stale
    // ./target/release built by some other invocation.
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "./target".into());
    let resolved = std::fs::canonicalize(env!("CARGO_BIN_EXE_compile"))
        .expect("cargo-provided bin missing");
    let base = std::fs::canonicalize(&dir)
        .unwrap_or_else(|_| std::path::PathBuf::from(dir.clone()));
    assert!(
        resolved.starts_with(&base),
        "resolved {resolved:?} must live under CARGO_TARGET_DIR {dir:?}"
    );
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cli_guard_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn o_flag_writes_wasm_and_map_source_untouched() {
    let d = tmpdir("o");
    let src = d.join("victim.ts");
    std::fs::write(&src, SRC).unwrap();

    let out = d.join("out.wasm");
    let status = bin()
        .arg(&src)
        .arg("--near")
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn compile");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );

    // the source must be byte-identical
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        SRC,
        "SOURCE CLOBBERED"
    );
    // wasm exists at the -o path, map beside it
    assert!(out.exists(), "wasm not written to -o path");
    let map = d.join("out.wasm.map");
    assert!(map.exists(), "map sidecar missing");
    // map is JSON (not the source, not the wasm)
    let m = std::fs::read_to_string(&map).unwrap();
    assert!(m.contains("ping"), "map contents: {m}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn default_output_swaps_extension_not_source() {
    let d = tmpdir("def");
    let src = d.join("victim.ts");
    std::fs::write(&src, SRC).unwrap();

    let status = bin().arg(&src).output().expect("spawn compile");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );

    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        SRC,
        "SOURCE CLOBBERED"
    );
    assert!(
        d.join("victim.wasm").exists(),
        "default output must be victim.wasm"
    );
    assert!(
        d.join("victim.wasm.map").exists(),
        "map beside default output"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn explicit_output_onto_source_refuses() {
    let d = tmpdir("guard");
    let src = d.join("victim.ts");
    std::fs::write(&src, SRC).unwrap();

    let status = bin()
        .arg(&src)
        .arg("--output")
        .arg(&src)
        .output()
        .expect("spawn compile");
    assert!(
        !status.status.success(),
        "--output onto the input must exit nonzero"
    );
    let err = String::from_utf8_lossy(&status.stderr);
    assert!(err.contains("refusing"), "guard message: {err}");
    assert_eq!(
        std::fs::read_to_string(&src).unwrap(),
        SRC,
        "SOURCE CLOBBERED"
    );
    let _ = std::fs::remove_dir_all(&d);
}
