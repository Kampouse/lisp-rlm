//! String methods repeat/padStart/padEnd (2026-09-13).
//! expr_returns_str_method listed them for `+` concat typing since tour2,
//! but no lowering existed — `"0".repeat(5)` fell through to callee_name
//! and died with "nested member chains not in M1" (literal receiver) or
//! mapped to a nonexistent near/… builtin (identifier receiver).
//!
//! Lowerings: repeat → native (str-repeat s n); pads → let-bound
//! repeat+slice expression with JS semantics (default pad " ", no
//! truncation when already long enough).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function rep(): string { return "0".repeat(5); }
export function rep128(): string { return "0".repeat(128); }
export function dynRep(): string {
  const s = "ab";
  const n = 3;
  return s.repeat(n);
}
export function padStart(): string { return "7".padStart(4, "0"); }
export function padStartDefault(): string { return "ab".padStart(4); }
export function padStartNoTrunc(): string { return "abcdef".padStart(3, "0"); }
export function padEnd(): string { return "7".padEnd(4, "*"); }
export function padHex(): string { return "ff".padStart(6, "0"); }
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(method: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("srep_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("srep_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("srep.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("srep.t.near")
        .arg(method)
        .arg("{}")
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all.lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&all)
        .to_string()
}

#[test]
fn repeat_literal() {
    assert!(run("rep").contains("00000"));
}

#[test]
fn repeat_128() {
    assert!(run("rep128").contains(&"0".repeat(128)));
}

#[test]
fn repeat_dynamic() {
    assert!(run("dynRep").contains("ababab"));
}

#[test]
fn pad_start() {
    assert!(run("padStart").contains("0007"));
}

#[test]
fn pad_start_default_space() {
    assert!(run("padStartDefault").contains("  ab"));
}

#[test]
fn pad_start_no_truncation() {
    assert!(run("padStartNoTrunc").contains("abcdef"));
}

#[test]
fn pad_end() {
    assert!(run("padEnd").contains("7***"));
}

#[test]
fn pad_hex_zero_fill() {
    assert!(run("padHex").contains("0000ff"));
}
