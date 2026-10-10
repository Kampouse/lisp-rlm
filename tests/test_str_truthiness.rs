//! String truthiness in conditions (2026-10-10, t5).
//!
//! `if (s)` / `while (s)` / `s && x` with a STRING cond: the wasm emitters'
//! tag-aware truthiness treats any tagged string (non-zero i64: ptr+len
//! packed) as truthy — `if ("")` took the then-arm. JS semantics: "" is
//! falsy, non-empty is truthy. Fix: stringy conds lower to `(str-length s)`
//! so numeric truthiness (len ≠ 0) decides.
//!
//! Two backend bugs the fix exposed (both fixed here, both were silent):
//! 1. while-arm bare-raw fast path (2026-09-14) stacked I64Eqz + I32Eqz —
//!    inverted polarity: the loop EXITED while the cond was truthy.
//! 2. discard_normalize rewrote the while-exit gate `(if (= __wl_brk 0)
//!    COND 0)` into `(begin COND 0)` — constant-0 cond, loop never ran.
//!
//! Detector discipline: cases 5a/5b were RED on the unfixed toolchain
//! (0 iterations), all cases green after.

use std::process::Command;

fn uniq_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("str_truth_{}_{}_{}", tag, std::process::id(), n))
}

/// Compile TS via the legacy compile bin (single-file → TS frontend), run
/// via near-mock, return stdout.
fn run_ts(tag: &str, src: &str) -> Result<String, String> {
    let dir = uniq_dir(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let inp = dir.join("main.ts");
    let wasm = dir.join("out.wasm");
    std::fs::write(&inp, src).map_err(|e| e.to_string())?;
    let out = Command::new(env!("CARGO_BIN_EXE_compile"))
        .arg(&inp)
        .arg(&wasm)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "compile failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_near-mock"))
        .arg(&wasm)
        .arg("run")
        .current_dir(&dir)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(stdout)
}

/// `if (s)` on a local: "" falsy, "x" truthy.
#[test]
fn if_local_string_truthiness() {
    let out = run_ts(
        "iflocal",
        r#"export function run(): string {
  const s: string = "";
  if (s) { return "truthy"; }
  return "falsy";
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("falsy"), "empty string must be falsy: {out}");

    let out = run_ts(
        "iflocal2",
        r#"export function run(): string {
  const s: string = "x";
  if (s) { return "truthy"; }
  return "falsy";
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("truthy"), "non-empty must be truthy: {out}");
}

/// `if (s)` on a string-typed PARAM: `{"s":""}` falsy, `{"s":"abc"}` truthy.
#[test]
fn if_param_string_truthiness() {
    let dir = uniq_dir("ifparam");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let inp = dir.join("main.ts");
    let wasm = dir.join("out.wasm");
    std::fs::write(
        &inp,
        r#"export function run(s: string): string {
  if (s) { return "truthy"; }
  return "falsy";
}"#,
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_compile"))
        .arg(&inp)
        .arg(&wasm)
        .output()
        .expect("compile");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    for (arg, want) in [(r#"{"s":""}"#, "falsy"), (r#"{"s":"abc"}"#, "truthy")] {
        let out = Command::new(env!("CARGO_BIN_EXE_near-mock"))
            .arg(&wasm)
            .arg("run")
            .arg(arg)
            .current_dir(&dir)
            .output()
            .expect("run");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains(want), "arg {arg}: want {want}: {stdout}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Number 0 must STAY falsy and 1 truthy (numeric truthiness untouched).
#[test]
fn numeric_truthiness_unchanged() {
    let out = run_ts(
        "num0",
        r#"export function run(): string {
  const n = 0;
  if (n) { return "truthy"; }
  return "falsy";
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("falsy"), "0 must be falsy: {out}");
}

/// `while (s)` with a break: must run len iterations (while-exit gate must
/// survive discard_normalize; the bare-raw cond fast path must keep
/// polarity). Was RED: 0 iterations from either backend bug.
#[test]
fn while_string_cond_with_break() {
    let out = run_ts(
        "whilebrk",
        r#"export function run(): number {
  let s: string = "abc";
  let n = 0;
  while (s) {
    n = n + 1;
    if (n === 3) { break; }
    s = s.slice(1);
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("3"), "want 3 iterations: {out}");
}

/// `while (s)` WITHOUT break: natural termination when the string empties.
/// Was RED (inverted fast-path polarity).
#[test]
fn while_string_cond_natural_exit() {
    let out = run_ts(
        "whilenat",
        r#"export function run(): number {
  let s: string = "abc";
  let n = 0;
  while (s) {
    n = n + 1;
    s = s.slice(1);
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("3"), "want 3 iterations: {out}");
}

/// `!s` on empty string must stay truthy (was already correct pre-fix —
/// regression guard against double-wrapping).
#[test]
fn not_string_emptiness_unchanged() {
    let out = run_ts(
        "notstr",
        r#"export function run(): string {
  const s: string = "";
  if (!s) { return "was-falsy"; }
  return "was-truthy";
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("was-falsy"), "!\"\" must be truthy: {out}");
}

/// Numeric while cond with break keeps working (regression: the exit-gate
/// fix must not disturb bool/int conds).
#[test]
fn while_numeric_cond_with_break_unchanged() {
    let out = run_ts(
        "whilenum",
        r#"export function run(): number {
  let s: string = "abc";
  let n = 0;
  while (n < 3) {
    n = n + 1;
    if (n === 3) { break; }
    s = s.slice(1);
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("3"), "want 3: {out}");
}
