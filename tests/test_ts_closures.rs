//! F3 (2026-10-04): scoped closures — local `const f = arrow` support.
//!
//! PROBED REALITY (see docs/tasks/TASK-ts-usability-PROGRESS.md):
//! - local arrow + DIRECT call-by-name: works on BOTH backends (wasm +
//!   interpreter), INCLUDING capture of immutable locals — supported.
//! - arrow capturing a set!-assigned local: T4 landmine (wasm closure
//!   cells not per-invocation) — frontend HARD-ERRORS naming the var.
//! - lambda-valued local called from inside .map/.filter/.reduce
//!   callback: emits an INVALID module (dispatch-freeze) — HARD-ERROR.
//! - arrow literal as call argument to a user fn: unknown-function at
//!   emit — HARD-ERROR.
//!
//! Positive cases compile → typecheck → near-mock run (differential with
//! the lisp probes already banked); negative cases assert the exact
//! hard-error text fragments.
//!
//! FINDING (2026-10-04): `.map((x) => x * 3)` over a json array param
//! TRAPS at runtime (wasm unreachable) — pre-existing, and NO existing
//! test exercises a plain TS pipeline (grepped the whole suite). The
//! map/filter/reduce surface claims in the header doc need their own
//! verification pass; out of F3 scope.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

fn compile_ts(src: &str, what: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("{what}: {e}"));
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("{what} parse: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("{what} TYPECHECK: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("{what} codegen: {e}"))
}

fn mock_run(wasm: &[u8], method: &str, input: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "f3cl_{}_{}_{}.wasm",
        method,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .output()
        .expect("near-mock run");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            if let Some(i) = rest.find(" (raw i64") {
                return rest[..i].to_string();
            }
            return rest.trim_end().to_string();
        }
    }
    panic!("no 📄 result line in mock output:\n{s}");
}

fn err_of(src: &str) -> String {
    ts_to_lisp_source(src)
        .err()
        .unwrap_or_else(|| panic!("expected error, got Ok"))
}

// ── positives ──────────────────────────────────────────────────────────

#[test]
fn local_arrow_direct_call_no_capture() {
    let w = compile_ts(
        r#"
export function run(a: number): number {
  const f = (x: number): number => x * 2;
  return f(a);
}"#,
        "direct",
    );
    assert_eq!(mock_run(&w, "run", r#"{"a":21}"#), "42");
}

#[test]
fn local_arrow_immutable_capture() {
    // probed end-to-end 2026-10-04: wasm + interpreter both correct
    let w = compile_ts(
        r#"
export function run(a: number): number {
  const n = 3;
  const f = (x: number): number => x + n;
  return f(a);
}"#,
        "cap-imm",
    );
    assert_eq!(mock_run(&w, "run", r#"{"a":5}"#), "8");
}

#[test]
fn local_arrow_block_body_and_loop() {
    let w = compile_ts(
        r#"
export function run(a: number): number {
  const f = (x: number): number => { return x + 1; };
  let s = 0;
  for (let i = 0; i < a; i++) {
    s = f(s);
  }
  return s;
}"#,
        "loop",
    );
    assert_eq!(mock_run(&w, "run", r#"{"a":4}"#), "4");
}

// ── negatives (hard errors, exact fragments) ───────────────────────────

#[test]
fn mutable_capture_hard_errors_t4() {
    let e = err_of(
        r#"
export function run(a: number): number {
  let c = 0;
  c = a + 1;
  const f = (x: number): number => x + c;
  return f(1);
}"#,
    );
    assert!(e.contains("mutable local `c`"), "got: {e}");
    assert!(e.contains("T4"), "got: {e}");
}

#[test]
fn compound_assign_marks_mutable_for_capture() {
    // F1 interplay: `c += …` is also set! — same T4 capture error
    let e = err_of(
        r#"
export function run(a: number): number {
  let c = 0;
  c += a;
  const f = (x: number): number => x + c;
  return f(1);
}"#,
    );
    assert!(e.contains("mutable local `c`"), "got: {e}");
}

#[test]
fn map_callback_calling_lambda_local_hard_errors() {
    // probed 2026-10-04: this shape emits an INVALID wasm module — the
    // frontend must hard-error BEFORE codegen with the dispatch-freeze msg
    let e = err_of(
        r#"
export function run(xs: number[]): number {
  const f = (x: number): number => x * 3;
  const out = xs.map((x: number): number => f(x));
  let s = 0;
  for (const v of out) { s += v; }
  return s;
}"#,
    );
    assert!(e.contains("dispatch-freeze"), "got: {e}");
    assert!(e.contains("`f`"), "got: {e}");
}

#[test]
fn arrow_as_call_argument_hard_errors() {
    let e = err_of(
        r#"
function apply2(g: any, v: number): number { return 0; }
export function run(a: number): number {
  return apply2((x: number): number => x * 2, a);
}"#,
    );
    assert!(e.contains("arrow as call argument"), "got: {e}");
}
