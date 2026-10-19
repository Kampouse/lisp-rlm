//! Regression: same-module calls to EXPORTED functions (2026-10-19).
//!
//! Exported `function` declarations lower params to JSON reads — their
//! entry define has lisp arity 0 — so any in-module call failed the
//! checker with "arity mismatch: 0 args vs 1 args". Fix: every param'd
//! export splits into a real-param `__impl_*` twin + a thin JSON entry,
//! and call sites route through the twin (EXPORTED_FNS registry).
//! Zero-param exports stay entry-callable by name (no split, no rewrite).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

const SRC: &str = r#"
export function isq(n: string): string {
  if (u128Eq(n, "0")) {
    return "0";
  }
  return u128Add(n, "1");
}
// in-module calls to BOTH a string-param export and a zero-param export
export function callIt(n: string): string { return isq(n); }
export function callZero(): string { return zero(); }
export function zero(): string { return "42"; }
"#;

#[test]
fn exported_fn_calls_lower_typecheck_and_compile() {
    let ir = ts_to_lisp_source(SRC).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    // the call-site rewrite: in-module calls target __impl_*, the entries
    // keep their names
    assert!(ir.contains("(__impl_isq "), "impl routing missing:\n{ir}");
    assert!(ir.contains("(define (__impl_isq n)"), "impl define missing");
    assert!(ir.contains("(zero)"), "zero-param call must NOT rewrite");
    assert!(!ir.contains("__impl_zero"), "zero-param export must not split");
    // the full gate: typecheck (the old failure point) + compile
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("typecheck — exported same-module call");
    compile_near_from_exprs(&exprs).expect("compile");
}
