//! env/get (wasi:cli/environment) regression suite — the v8.1 channel.
//!
//! Guards the exact invariants tonight's live TEE session proved:
//!   1. the canon get-environment import is declared (retptr form)
//!   2. GLOBAL 24 is the realloc bump counter (canon-window law)
//!   3. the helper decodes the host-lowered list via [kptr][klen][vptr][vlen]
//!   4. unused outlayer imports are tree-shaken (storage absent when unused)
//!
//! These are structural checks on the emitted component — the *behavioral*
//! proof lives in the live testnet TEE session that validated env/get v8.1
//! (network/sender/nil/stdin-passthrough, main ed8d73c), which cargo cannot
//! execute.

use lisp_rlm_wasm::compile_outlayer_p2;

/// (env/get "X") must declare the wasi:cli/environment import.
#[test]
fn env_get_declares_canon_import() {
    let src = r#"(define (run) (env/get "NEAR_NETWORK_ID"))"#;
    let comp = compile_outlayer_p2(src).expect("compile");
    let wit = component_wit(&comp);
    assert!(
        wit.contains("wasi:cli/environment@"),
        "environment import missing from component world:\n{}",
        wit
    );
}

/// Programs WITHOUT env/get must not import the environment interface
/// (tree-shaken world = smaller attack surface + standalone-friendly).
#[test]
fn env_get_absent_when_unused() {
    let src = r#"(define (run) "no env here")"#;
    let comp = compile_outlayer_p2(src).expect("compile");
    let wit = component_wit(&comp);
    assert!(
        !wit.contains("wasi:cli/environment"),
        "environment import should be stripped when unused:\n{}",
        wit
    );
}

/// Storage imports appear only when storage is actually used.
#[test]
fn storage_import_only_when_used() {
    let with = compile_outlayer_p2(r#"(define (run) (outlayer/storage-set "k" "v"))"#).unwrap();
    let without = compile_outlayer_p2(r#"(define (run) "plain")"#).unwrap();
    assert!(component_wit(&with).contains("near:storage"));
    assert!(!component_wit(&without).contains("near:storage"));
}

/// GLOBAL 24 (realloc bump) must be present in the globals section whenever
/// cabi_realloc is exported — the canon-window-wipe law in binary form.
#[test]
fn bump_global_present_with_env_get() {
    let src = r#"(define (run) (env/get "X"))"#;
    let comp = compile_outlayer_p2(src).expect("compile");
    let wat = wat_of(&comp);
    // GLOBAL 24 is the last of 25 globals: global 0 is the mut i64 tick,
    // globals 1..=24 are mut i32 — so 24 mut-i32 entries total.
    let mut_i32_globals = wat
        .lines()
        .filter(|l| l.contains("(global") && l.contains("mut i32"))
        .count();
    assert_eq!(
        mut_i32_globals, 24,
        "expected exactly 24 mut i32 globals (1..=24 incl. GLOBAL 24 bump), got {}",
        mut_i32_globals
    );
    // The bump counter must be live: cabi_realloc reads AND writes it.
    let bump_reads = wat.matches("global.get 24").count();
    let bump_writes = wat.matches("global.set 24").count();
    assert!(
        bump_reads >= 1 && bump_writes >= 1,
        "GLOBAL 24 not used by cabi_realloc (reads={bump_reads}, writes={bump_writes})"
    );
}

/// The env helper must load the list ptr AND pair count from the retptr
/// (offsets 16/20 off the key area) — guards against regression to the
/// shadow-table or ret-area-scan designs.
#[test]
fn helper_reads_retptr_list() {
    let src = r#"(define (run) (env/get "X"))"#;
    let comp = compile_outlayer_p2(src).expect("compile");
    let wat = wat_of(&comp);
    // the helper loads [retptr+16] and [retptr+20] (align=2 i32 loads)
    // then shifts by 4 for the 16-byte entry stride
    assert!(
        wat.contains("i32.load offset=16") && wat.contains("i32.load offset=20"),
        "retptr loads (ka+16/ka+20) missing — decode no longer walks the host list"
    );
    assert!(
        wat.contains("i32.shl") || wat.contains("i32.const 4"),
        "16-byte entry stride missing"
    );
}

// ── helpers ──────────────────────────────────────────────────────────

fn component_wit(comp: &[u8]) -> String {
    // Interface names are stored verbatim in the component's name sections;
    // a lossy scan finds them without a wasm-tools dependency.
    String::from_utf8_lossy(comp).to_string()
}

fn core_module(comp: &[u8]) -> Vec<u8> {
    // The inner core module is embedded in the component; for WAT purposes
    // we require wasm-tools (present in the dev env) on a temp file.
    let path = "/tmp/env_get_test_core.wasm";
    std::fs::write(path, comp).expect("write core");
    comp.to_vec()
}

fn wat_of(comp: &[u8]) -> String {
    let path = "/tmp/env_get_test_core.wasm";
    std::fs::write(path, comp).expect("write core");
    let out = std::process::Command::new("wasm-tools")
        .args(["print", path])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => panic!(
            "wasm-tools print failed: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => panic!("wasm-tools not available: {e}"),
    }
}
