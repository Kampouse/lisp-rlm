/// build.rs: compile schnorr-wasm to wasm32-unknown-unknown and place it where
/// the compiler can include_bytes! it.
///
/// This runs on `cargo build` of the main lisp-rlm crate (host, not wasm).
/// The output is a .wasm file in OUT_DIR that the compiler embeds at runtime.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let target = PathBuf::from(&out_dir).join("schnorr_verify.wasm");

    // Only rebuild if schnorr-wasm/src changed
    println!("cargo::rerun-if-changed=schnorr-wasm/src");
    println!("cargo::rerun-if-changed=schnorr-wasm/Cargo.toml");

    // schnorr-wasm is EXCLUDED from workspace members (0021a67: no_std +
    // panic=abort needs its own profile root). Build it standalone from its
    // own dir — `-p` from the root no longer resolves it.
    let schnorr_dir = PathBuf::from(&manifest_dir).join("schnorr-wasm");
    let status = Command::new("cargo")
        .args([
            "build",
            "--profile=wasm-release",
            "--target=wasm32-unknown-unknown",
        ])
        .env("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS", "-C target-cpu=mvp")
        .current_dir(&schnorr_dir)
        .status()
        .expect("failed to run cargo build for schnorr-wasm");

    if !status.success() {
        panic!("schnorr-wasm build failed");
    }

    let wasm_path = schnorr_dir
        .join("target/wasm32-unknown-unknown/wasm-release/lisp_rlm_schnorr_wasm.wasm");

    std::fs::copy(&wasm_path, &target).unwrap_or_else(|e| {
        panic!("failed to copy schnorr WASM: {}", e);
    });

    println!("cargo::rustc-env=SCHNORR_WASM_PATH={}", target.display());
}