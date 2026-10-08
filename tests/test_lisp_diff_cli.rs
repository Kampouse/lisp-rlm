// lisp-diff CLI end-to-end (TASK-DX-AGREE item 3 follow-up)
//
// The task text demands an in-repo test proving the divergence reporting
// path works end-to-end, not just manual smoke runs. We spawn the real
// bin (cargo sets CARGO_BIN_EXE_lisp-diff for integration tests) against:
//   * a deliberately divergent fixture — the documented json-get
//     negative-number gap (tests/differential.rs header, wat-audited
//     2026-10-07): interp returns -7, wasm auto-parse returns 7.
//     When that gap is closed upstream, swap the fixture for another
//     forced divergence — the CONTRACT under test is the reporting
//     path (DIVERGENT + both backends' results + exit 1), not the gap.
//   * an agreeing program — IDENTICAL + exit 0.

use std::process::Command;

fn lisp_diff() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lisp-diff"));
    c.current_dir(env!("CARGO_MANIFEST_DIR"));
    c
}

fn tmpfile(tag: &str, body: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("lisp_diff_cli_{tag}_{}.lisp", std::process::id()));
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn reports_first_divergent_form_and_exits_1() {
    let f = tmpfile(
        "div",
        // form 0 agrees; form 1 is the documented negative-number gap
        "(json-get \"{\\\"x\\\":42}\" \"x\")\n(json-get \"{\\\"n\\\":-7}\" \"n\")\n",
    );
    let out = lisp_diff().arg(&f).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() == false,
        "divergent file must exit nonzero (got {:?}); stderr: {stderr}",
        out.status.code()
    );
    assert_eq!(out.status.code(), Some(1), "divergence exit code is 1");
    assert!(stdout.contains("DIVERGENT"), "stdout must name the divergence:\n{stdout}");
    // First divergent form only — form 0 agreed, so the report must be
    // about the json-get negative-number form.
    assert!(
        stdout.contains("interp : Num(-7)"),
        "interp result must be printed:\n{stdout}"
    );
    assert!(
        stdout.contains("wasm   : Num(7)"),
        "wasm result must be printed:\n{stdout}"
    );
    let _ = std::fs::remove_file(&f);
}

#[test]
fn identical_program_exits_0() {
    let f = tmpfile(
        "ok",
        "(define (double x) (* x 2))\n(double 21)\n(str-cat \"a\" \"b\")\n",
    );
    let out = lisp_diff().arg(&f).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "agreeing exit code is 0");
    assert!(stdout.trim_end().ends_with("IDENTICAL"), "stdout:\n{stdout}");
    let _ = std::fs::remove_file(&f);
}

#[test]
fn missing_file_is_usage_error_exit_2() {
    let out = lisp_diff().arg("/nonexistent/nope.lisp").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "usage/IO errors exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read"), "stderr: {stderr}");
}
