// Regression tests for TASK-concat-bug.md: single-expression multi-operand
// string concat `a + b + c + d` over HOST-RESULT strings (storage_get values)
// returned corrupted output on NEAR testnet (2026-09-02, lisp7 debug series):
// prefix operands as NULs / 8-byte tagged-pointer leaks, while the
// statement-wise growing path `buf = buf + x` was correct.
//
// Root cause (see fix in src/wasm_emit/call_string.rs emit_poly_add): TS
// lowers `a + b + ...` to generic (+ ...) whenever neither operand is a
// literal/string-local/str-method shape — vars bound to storage_get /
// json_get_str / host results fall through. The checker explicitly types
// str+str as concat ("coerce ... is the emitter's job"), but the emitter's
// `+` unconditionally did a TAGGED i64 add of the two TAG_STR descriptors:
// (raw1<<3|5) + (raw2<<3|5) == ((raw1+raw2+1)<<3)|2 — a corrupted
// descriptor (wrong tag, pointer past every live buffer).
//
// These tests drive the near-mock (same harness as test_ts_bls_msig.rs):
// long operands at the exact repro sizes (192/384/192/384 hex = 1152 total)
// stored via storageSet, then concatenated in three shapes. Expected values
// are plain Rust string concat.

use lisp_rlm_wasm::compile_near_from_exprs;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::parse_all;
use std::sync::MutexGuard;

const CONCAT_TS: &str = r#"
export function setup(a: string, b: string, c: string, d: string): string {
  near.storageSet("sc:a", a);
  near.storageSet("sc:b", b);
  near.storageSet("sc:c", c);
  near.storageSet("sc:d", d);
  return "ok";
}
export function flat(): string {
  let a = near.storageGet("sc:a") ?? "";
  let b = near.storageGet("sc:b") ?? "";
  let c = near.storageGet("sc:c") ?? "";
  let d = near.storageGet("sc:d") ?? "";
  return a + b + c + d;
}
export function halves(): string {
  let a = near.storageGet("sc:a") ?? "";
  let b = near.storageGet("sc:b") ?? "";
  let c = near.storageGet("sc:c") ?? "";
  let d = near.storageGet("sc:d") ?? "";
  let h1 = a + b;
  let h2 = c + d;
  return h1 + h2;
}
export function looped(): string {
  let a = near.storageGet("sc:a") ?? "";
  let b = near.storageGet("sc:b") ?? "";
  let c = near.storageGet("sc:c") ?? "";
  let d = near.storageGet("sc:d") ?? "";
  let buf = "";
  buf = buf + a;
  buf = buf + b;
  buf = buf + c;
  buf = buf + d;
  return buf;
}
export function numPlusStr(x: number, s: string): string {
  return s + x;
}
"#;

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn compile(src: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    compile_near_from_exprs(&exprs).unwrap()
}

fn call(wasm: &[u8], method: &str, args: &str, tag: &str, fresh: bool) -> String {
    let _l = lock();
    let p = std::env::temp_dir().join(format!("strcat_long_{}_{}.wasm", std::process::id(), tag));
    std::fs::write(&p, wasm).unwrap();
    let state = std::env::temp_dir().join(format!("strcat_long_{}_{}.bin", std::process::id(), tag));
    if fresh {
        let _ = std::fs::remove_file(&state);
    }
    let manifest = format!("c.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(&state)
        .arg(&manifest)
        .arg("c.t.near")
        .arg(method)
        .arg(args)
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default()
}

fn ret_line(out: &str) -> String {
    out.lines()
        .find(|l| l.contains("📄"))
        .map(|l| l.trim().trim_start_matches('📄').trim().to_string())
        .unwrap_or_else(|| "<no return>".into())
}

/// Deterministic long operand at the TASK repro sizes:
/// a=192, b=384, c=192, d=384 chars → 1152 total.
fn piece(seed: u8, n: usize) -> String {
    (0..n)
        .map(|i| char::from(b'a' + ((seed + i as u8) % 26)))
        .collect()
}

#[test]
fn strcat_single_expression_long_operands() {
    let wasm = compile(CONCAT_TS);
    let a = piece(0, 192);
    let b = piece(7, 384);
    let c = piece(14, 192);
    let d = piece(21, 384);
    let setup = format!(
        r#"{{"a":"{a}","b":"{b}","c":"{c}","d":"{d}"}}"#
    );
    let r = call(&wasm, "setup", &setup, "long", true);
    assert!(r.contains("ok"), "setup: {r}");

    let expected = format!("{a}{b}{c}{d}");
    assert_eq!(expected.len(), 1152);

    // (1) THE repro shape: 4-operand single expression return.
    let r = ret_line(&call(&wasm, "flat", "{}", "long", false));
    assert_eq!(r, expected, "flat 4-operand single-expression concat");

    // (2) two halves: let h1 = a + b; let h2 = c + d; return h1 + h2;
    let r = ret_line(&call(&wasm, "halves", "{}", "long", false));
    assert_eq!(r, expected, "halves concat");

    // (3) the known-good growing-buffer statement path must stay correct.
    let r = ret_line(&call(&wasm, "looped", "{}", "long", false));
    assert_eq!(r, expected, "looped growing-buffer concat");

    // (4) mixed str + num coerces the num through to-string (checker's
    // documented contract for polymorphic +).
    let r = ret_line(&call(&wasm, "numPlusStr", r#"{"x":7,"s":"v"}"#, "long", false));
    assert_eq!(r, "v7", "str+num coercion");
}

#[test]
fn strcat_single_expression_short_operands() {
    // Short strings through the same shapes — the old tagged-add garbage
    // was length-dependent only by accident; all sizes must be exact.
    let wasm = compile(CONCAT_TS);
    let a = "ab".to_string();
    let b = "cdef".to_string();
    let c = "g".to_string();
    let d = "hij".to_string();
    let setup = format!(r#"{{"a":"{a}","b":"{b}","c":"{c}","d":"{d}"}}"#);
    let r = call(&wasm, "setup", &setup, "short", true);
    assert!(r.contains("ok"), "setup short: {r}");
    let expected = format!("{a}{b}{c}{d}");
    let r = ret_line(&call(&wasm, "flat", "{}", "short", false));
    assert_eq!(r, expected, "flat short operands");
    let r = ret_line(&call(&wasm, "halves", "{}", "short", false));
    assert_eq!(r, expected, "halves short operands");
}
