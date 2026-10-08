//! TS frontend statement-discard lowering (2026-10-06).
//!
//! Two shapes used to false-reject at typecheck (`str ≠ int`) because the
//! __fn_done statement guards wrap arbitrary statements as
//! `(if (= __fn_done 0) BODY 0)` while BODY's TYPE can be a str:
//!   1. a bare str-returning statement call (`poolSwap(amt);`)
//!   2. a nested guard-if inside a multi-statement branch
//! Both compiled BEFORE only via workarounds (assign-and-drop, hoisting).
//! `discard_normalize` rewrites the guard to `(begin BODY 0)` — the loop
//! lowering's existing idiom. The third test locks the paired lowering fix:
//! a branch ending in a direct `return` keeps its blanket commit even when
//! a PREFIX early-return carrier trips `is_commit_form` (found live — the
//! tail return's value was silently discarded; the old typecheck reject
//! had been masking it).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn check_ok(src: &str) -> String {
    let ir = ts_to_lisp_source(src).expect("frontend lowering");
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("parse IR");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("typecheck must accept: {}\nIR:\n{}", e, ir));
    ir
}

/// Bare str-returning statement call in a multi-return function.
#[test]
fn str_statement_call_typechecks() {
    let ir = check_ok(
        "function inner(s: string): string { return s; }\n\
         export function outer(): string {\n\
         \x20 if (strLength(\"x\") > 1) { return \"early\"; }\n\
         \x20 inner(\"side\");\n\
         \x20 return \"tail\";\n\
         }\n",
    );
    assert!(
        ir.contains("(begin (inner \"side\") 0)"),
        "statement call must be begin-0 discarding, got:\n{}",
        ir
    );
}

/// Nested guard-if inside a multi-statement branch of a multi-return fn.
#[test]
fn nested_guard_if_typechecks() {
    check_ok(
        "function f(op: string): string {\n\
         \x20 if (op === \"a\") { return \"A\"; }\n\
         \x20 if (op === \"b\") { if (strLength(op) < 1) { return \"x\"; } return \"B\"; }\n\
         \x20 return \"C\";\n\
         }\n\
         export function go(): string { return f(\"a\"); }\n",
    );
}

/// A branch ending in a direct `return` keeps the blanket commit even when
/// a prefix early-return carrier makes `is_commit_form` greedy-true. The
/// lowered IR must carry the (set! __fn_res …) blanket around the branch.
#[test]
fn tail_return_blanket_survives_prefix_carrier() {
    let ir = check_ok(
        "function f(op: string): string {\n\
         \x20 if (op === \"b\") { if (strLength(op) < 1) { return \"x\"; } return \"B\"; }\n\
         \x20 return \"C\";\n\
         }\n\
         export function go(): string { return f(\"b\"); }\n",
    );
    assert!(
        ir.matches("(set! __fn_res").count() >= 2,
        "nested carrier commit AND tail-return blanket must both be present:\n{}",
        ir
    );
}

/// Checker errors on TS source locate to the failing function + line via
/// prefix-bisect (sequential checker ⇒ first failing prefix = culprit form).
#[test]
fn locate_form_error_names_culprit() {
    let src = "function ok1(x: number): number { return x; }\n\
               \x20 function badTail(s: string): string {\n\
               \x20   const r = strLength(s) > 0 ? \"str\" : 5;\n\
               \x20   return r;\n\
               \x20 }\n\
               \x20 export function go(): string { return badTail(\"x\"); }\n";
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = lisp_rlm_wasm::parse_all(&ir).unwrap();
    let err = lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect_err("must fail");
    let map = lisp_rlm_wasm::ts_frontend::take_fn_def_offsets();
    let loc = lisp_rlm_wasm::ts_frontend::locate_form_error(&exprs, &map, src, &err)
        .expect("must locate");
    assert!(loc.contains("`badTail`"), "culprit fn named: {}", loc);
    assert!(
        loc.contains("ts line 2"),
        "definition line, not call site: {}",
        loc
    );
}
