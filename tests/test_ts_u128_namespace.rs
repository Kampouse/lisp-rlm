//! u128 namespace is closed — unknown members hard-error at LOWERING with
//! the free-function suggestion (2026-10-10).
//!
//! Background: `u128.mulDiv(a,b,d)` silently lowered to `u128/mul_div` (an
//! op that doesn't exist) and failed only as a cryptic runtime/emit error.
//! The AFP CLMM port hit this. The real namespace members (add/sub/mul/
//! div/mod/lt/gt/eq/fromI64/toI64/isZero) must keep lowering unchanged.

use std::process::Command;

fn compile_ts(tag: &str, src: &str) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("ts_u128ns_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts_path = dir.join("probe.ts");
    std::fs::write(&ts_path, src).map_err(|e| e.to_string())?;
    let out = Command::new(env!("CARGO_BIN_EXE_compile"))
        .arg(ts_path.to_str().unwrap())
        .output()
        .map_err(|e| e.to_string())?;
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok(all)
    } else {
        Err(all)
    }
}

/// THE regression: u128.mulDiv used to lower to u128/mul_div (nonexistent).
#[test]
fn u128_muldiv_member_is_rejected_with_suggestion() {
    let err = compile_ts(
        "muldiv",
        r#"export function run(): string { return u128.mulDiv("6", "4", "8"); }"#,
    )
    .expect_err("u128.mulDiv must NOT compile");
    assert!(err.contains("u128.mulDiv does not exist"), "got: {err}");
    assert!(err.contains("u128MulDiv(a, b, d)"), "got: {err}");
}

/// Unknown member with no free-function twin still errors, naming the
/// closed surface.
#[test]
fn u128_unknown_member_is_rejected() {
    let err = compile_ts(
        "bogus",
        r#"export function run(): string { return u128.frobnicate("1"); }"#,
    )
    .expect_err("u128.frobnicate must NOT compile");
    assert!(err.contains("u128.frobnicate does not exist"), "got: {err}");
    assert!(err.contains("namespace is closed"), "got: {err}");
}

/// Control: REAL namespace members keep lowering + running.
#[test]
fn u128_real_members_still_work() {
    let out = compile_ts(
        "ctrl",
        r#"export function run(): string { return u128.add("2", "3"); }"#,
    )
    .expect("u128.add must compile");
    assert!(!out.contains("does not exist"), "got: {out}");
    assert!(out.contains("Parsed"), "compile ran: {out}");
}
