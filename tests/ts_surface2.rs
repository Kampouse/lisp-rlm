//! TS surface batch 2 (2026-10-05): strict gate, object property writes,
//! switch (all-break if-chains), do-while, Number/parseInt aliases.
//! Driven by live evidence: brain-written lisp gamed the checker via the
//! unknown-name passthrough; `o.x = v` and `switch` were the top
//! practical gaps after loops shipped.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn lower(src: &str) -> String {
    ts_to_lisp_source(src).expect("must lower")
}

fn lower_err(src: &str) -> String {
    ts_to_lisp_source(src).expect_err("must NOT lower")
}

fn compile(src: &str) {
    let ir = lower(src);
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

// ── strict gate ────────────────────────────────────────────────────────

/// Bare lisp builtins no longer pass through as "TS" — the exact vector
/// the brain used to game t5_vowels/td_prime_sum.
#[test]
fn strict_gate_rejects_lisp_passthrough() {
    for src in [
        "reduce((a: number, b: number): number => a + b, 0, xs);",
        "let n: number = vec-length(xs);",
        "array(1, 2, 3);",
    ] {
        let e = lower_err(src);
        assert!(
            e.contains("unknown function") || e.contains("not in M1"),
            "{src}: {e}"
        );
    }
}

#[test]
fn strict_gate_allows_user_fns_and_rlm_api() {
    let ir = lower(
        "function double(x: number): number { return x * 2; }\nrlm_set(\"d\", double(21));\n",
    );
    assert!(ir.contains("(double 21)"), "{ir}");
    assert!(ir.contains("rlm_set"), "{ir}");
}

/// Hoisting: user fn called before its definition still passes the gate.
#[test]
fn strict_gate_fn_hoisting() {
    let ir = lower(
        "const y: number = helper(1);\nfunction helper(x: number): number { return x + 1; }\nrlm_set(\"y\", y);\n",
    );
    assert!(ir.contains("(helper 1)"), "{ir}");
}

// ── object property writes ─────────────────────────────────────────────

#[test]
fn object_write_rebinds() {
    let ir = lower(
        "let o: string = { votes: 5, name: \"zed\" };\no.votes = 9;\nrlm_set(\"v\", o.votes);\n",
    );
    assert!(ir.contains("(set! o (json-set o"), "{ir}");
    compile(
        "export function bump(): string {\n  let o: string = { votes: 5 };\n  o.votes = 9;\n  return o.votes;\n}\n",
    );
}

#[test]
fn object_write_nested_target_rejected() {
    let e = lower_err("let o: string = { a: \"{}\" };\no.a.b = 1;");
    assert!(e.contains("single-level"), "{e}");
}

#[test]
fn object_write_compound_rejected() {
    let e = lower_err("let o: string = { votes: 5 };\no.votes += 1;");
    assert!(e.contains("compound"), "{e}");
}

// ── switch ─────────────────────────────────────────────────────────────

#[test]
fn switch_lowers_to_if_chain() {
    let ir = lower(
        "let fee: number = 0;\nswitch (\"gold\") { case \"gold\": fee = 10; break; case \"silver\": fee = 5; break; default: fee = 1; break; }\nrlm_set(\"fee\", fee);\n",
    );
    assert!(ir.contains("(if (= \"gold\" \"gold\")"), "{ir}");
    assert!(ir.contains("(set! fee 10)"), "{ir}");
}

#[test]
fn switch_number_discriminant_compiles() {
    compile(
        "export function tier(n: number): number {\n  let fee: number = 0;\n  switch (n) { case 1: fee = 10; break; case 2: fee = 20; break; default: fee = 1; break; }\n  return fee;\n}\n",
    );
}

#[test]
fn switch_fallthrough_rejected() {
    let e = lower_err("switch (1) { case 1: let x: number = 1; }");
    assert!(e.contains("break"), "{e}");
    let e = lower_err("switch (1) { case 1: case 2: let x: number = 1; break; }");
    assert!(e.contains("empty case"), "{e}");
}

#[test]
fn switch_return_rejected_with_hint() {
    // switch mid-function (not tail) so the prefix-arm validation fires
    let e = lower_err(
        "export function f(n: number): number { let r: number = 0; switch (n) { case 1: return 10; default: r = 0; break; } r = r + 1; return r; }",
    );
    assert!(e.contains("return inside a case"), "{e}");
}

// ── do-while ───────────────────────────────────────────────────────────

#[test]
fn do_while_lowers_pre_run_then_while() {
    let ir = lower("let i: number = 10;\ndo { i = i + 1; } while (i < 3);\nrlm_set(\"i\", i);\n");
    // body runs ONCE unconditionally (i=11 > 3), then the while is dead
    assert!(ir.contains("(while (< i 3)"), "{ir}");
    let parts: Vec<&str> = ir.lines().collect();
    assert!(
        parts
            .iter()
            .any(|l| l.contains("(begin (set! i (+ i 1))") && l.contains("while")),
        "{ir}"
    );
}

#[test]
fn do_while_compiles() {
    compile(
        "export function once(n: number): number {\n  do { n = n + 1; } while (n < 0);\n  return n;\n}\n",
    );
}

// ── number parse aliases ───────────────────────────────────────────────

#[test]
fn number_parse_int_aliases() {
    let ir = lower(
        "let a: number = Number(\"42\");\nlet b: number = parseInt(\"7\");\nlet c: number = parseFloat(\"0.5\");\nrlm_set(\"sum\", a + b);\n",
    );
    assert!(ir.contains("(str->num \"42\")"), "{ir}");
    assert!(ir.contains("(str->num \"7\")"), "{ir}");
}
