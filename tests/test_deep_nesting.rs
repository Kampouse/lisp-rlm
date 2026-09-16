//! Compiler stack-depth robustness (2026-09-15).
//!
//! The wasm emitter's expr() recursion grows with IR nesting depth, and
//! the TS frontend chains sequential statements into nested let/begin
//! forms — one nesting level per statement. A long real contract (the
//! PLONK verifier, ~1300 lines) needs 4+ MB of stack; the default
//! 2 MiB spawned-thread stack ABORTS the process (stack overflow is not
//! a catchable panic).
//!
//! Fix: every public compile entry point runs on a dedicated 256 MiB
//! (virtual) big-stack thread (helpers::run_deep); TYPE_REGISTRY moved
//! from thread_local to a global Mutex so it survives the thread hop.
//!
//! These tests compile generated deep sources on the DEFAULT test
//! thread — pre-fix they aborted the test process.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

fn gen_deep_ts(n_stmts: usize) -> String {
    let mut src = String::from("export function deep(): string {\n  let s = \"x\";\n");
    for i in 0..n_stmts {
        match i % 3 {
            0 => src.push_str("  s = s + \"a\";\n"),
            1 => src.push_str("  s = strCat(s, \"b\");\n"),
            _ => src.push_str("  s = (s + \"c\");\n"),
        }
    }
    src.push_str("  return s;\n}\n");
    src
}

#[test]
fn deep_function_body_compiles_on_default_thread() {
    let src = gen_deep_ts(600);
    let ir = ts_to_lisp_source(&src).expect("frontend lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).expect("emit (deep stack)");
    assert!(!wasm.is_empty());
}

#[test]
fn deep_paren_nesting_compiles() {
    // deeply nested parens/binary ops: (((...(1 + 2)...)))
    let depth = 300;
    let mut src = String::from("export function deep2(): number {\n  return ");
    for _ in 0..depth {
        src.push_str("(");
    }
    src.push_str("1");
    for _ in 0..depth {
        src.push_str(" + 2)");
    }
    src.push_str(";\n}\n");
    let ir = ts_to_lisp_source(&src).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    let wasm = compile_near_from_exprs(&exprs).expect("emit");
    assert!(!wasm.is_empty());
}
