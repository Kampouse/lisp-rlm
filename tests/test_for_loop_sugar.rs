//! `for` loop sugar over while lowering (2026-10-10, t6).
//!
//! Recon found the full matrix already working — break, continue,
//! early-return, nested inner-break, body mutation of the loop var,
//! for-of, and the u128-string accumulator shape that FROZE before the
//! t1 parse-cache fix (loop-back force-invalidation). This suite pins the
//! matrix so the for→while sugar can't regress silently. Detector values
//! chosen so break/continue/skip each change the answer.

use std::process::Command;

fn uniq_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("for_sugar_{}_{}_{}", tag, std::process::id(), n))
}

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

/// break + continue + accumulate: i=2 skipped, i=4 breaks → 0+1+3 = 4.
#[test]
fn for_break_continue_accumulate() {
    let out = run_ts(
        "brkcont",
        r#"export function run(): number {
  let n = 0;
  for (let i = 0; i < 5; i++) {
    if (i === 2) { continue; }
    if (i === 4) { break; }
    n = n + i;
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(
        out.contains("4"),
        "want 4 (0+1+3; 2 skipped, 4 breaks): {out}"
    );
}

/// Early return from inside the loop: returns i*10 = 30, not the sentinel.
#[test]
fn for_early_return() {
    let out = run_ts(
        "earlyret",
        r#"export function run(): number {
  for (let i = 0; i < 5; i++) {
    if (i === 3) { return i * 10; }
  }
  return -1;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("30"), "want 30: {out}");
}

/// Nested loops: break exits ONLY the inner loop → 3 × 2 = 6.
#[test]
fn nested_for_inner_break_only() {
    let out = run_ts(
        "nested",
        r#"export function run(): number {
  let n = 0;
  for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      if (j === 2) { break; }
      n = n + 1;
    }
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("6"), "want 6: {out}");
}

/// Body mutates the loop var (i += 2): 5 iterations over 0,2,4,6,8.
#[test]
fn for_body_mutates_loop_var() {
    let out = run_ts(
        "mutvar",
        r#"export function run(): number {
  let n = 0;
  for (let i = 0; i < 10; i++) {
    n = n + 1;
    i = i + 1;
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("5"), "want 5: {out}");
}

/// for-of over a string array (element .length accumulation).
#[test]
fn for_of_string_array() {
    let out = run_ts(
        "forof",
        r#"export function run(): number {
  const xs: string[] = ["a", "bb", "ccc"];
  let n = 0;
  for (const x of xs) {
    n = n + x.length;
  }
  return n;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("6"), "want 6: {out}");
}

/// THE t1 freeze shape: u128-string accumulator mutated in a for body.
/// Before the parse-cache loop-back fix this froze at "2"; want "8".
#[test]
fn for_u128_accumulator_converges() {
    let out = run_ts(
        "u128acc",
        r#"export function run(): string {
  let acc: string = "0";
  for (let i = 0; i < 4; i++) {
    acc = u128Add(acc, "2");
  }
  return acc;
}"#,
    )
    .expect("compile+run");
    assert!(out.contains("8"), "want 8 (froze at 2 pre-t1): {out}");
}
