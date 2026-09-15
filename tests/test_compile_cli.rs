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
    let mut c = Command::new("./target/release/compile");
    c.current_dir(env!("CARGO_MANIFEST_DIR"));
    c
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
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    // the source must be byte-identical
    assert_eq!(std::fs::read_to_string(&src).unwrap(), SRC, "SOURCE CLOBBERED");
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
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    assert_eq!(std::fs::read_to_string(&src).unwrap(), SRC, "SOURCE CLOBBERED");
    assert!(d.join("victim.wasm").exists(), "default output must be victim.wasm");
    assert!(d.join("victim.wasm.map").exists(), "map beside default output");
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
    assert_eq!(std::fs::read_to_string(&src).unwrap(), SRC, "SOURCE CLOBBERED");
    let _ = std::fs::remove_dir_all(&d);
}
