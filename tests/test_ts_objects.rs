//! M2 objects (2026-08-31): object literals, property reads, jsonSet.
//! Objects are JSON-string values — literals fold into (json-set "{}" k v),
//! reads lower to (near/json_get_str "k" obj), rebuilds via jsonSet/toStr.
//! Compile-level tests here; execution-level in test_json_set.rs (runtime).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn lower(src: &str) -> String {
    ts_to_lisp_source(src).expect("must lower")
}

fn compile(src: &str) {
    let ir = lower(src);
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("must typecheck");
    let wasm = lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("must compile");
    assert!(wasm.len() > 100);
}

// ── literals ─────────────────────────────────────────────────────────────

#[test]
fn object_literal_string_value_folds_json_set() {
    let out = lower("function f(): string { return { name: \"bob\" }; }");
    assert!(
        out.contains(r#"(json-set "{}" "name" (json-quote "bob"))"#),
        "string literal must json-quote: {out}"
    );
}

#[test]
fn object_literal_number_value_bare() {
    let out = lower("function f(): string { return { votes: 42 }; }");
    assert!(
        out.contains(r#"(json-set "{}" "votes" (to-string 42))"#),
        "numeric literal encodes bare: {out}"
    );
}

#[test]
fn object_literal_bool_bare() {
    let out = lower("function f(): string { return { active: true }; }");
    assert!(
        out.contains(r#""active" "true""#),
        "boolean literal lowers to bare true: {out}"
    );
}

#[test]
fn object_literal_multi_key_folds_nested() {
    let out = lower(
        "function f(): string { return { name: \"bob\", votes: 1, active: false }; }",
    );
    assert!(out.contains("(json-set (json-set (json-set"), "3 keys → 3 nested folds: {out}");
}

#[test]
fn object_literal_annotated_number_param_bare() {
    let out = lower(
        "export function f(votes: number): string {\n  return { votes: votes };\n}",
    );
    assert!(
        out.contains(r#"(json-set "{}" "votes" (to-string votes))"#),
        "': number' param must encode bare, not json-quote: {out}"
    );
}

#[test]
fn object_literal_string_param_quoted() {
    let out = lower(
        "export function f(name: string): string {\n  return { name: name };\n}",
    );
    assert!(
        out.contains(r#"(json-set "{}" "name" (json-quote name))"#),
        "string params encode via json-quote: {out}"
    );
}

// ── property reads ───────────────────────────────────────────────────────

#[test]
fn property_read_lowers_to_json_get() {
    let out = lower(
        "export function f(u: string): string {\n  return u.name;\n}",
    );
    assert!(
        out.contains(r#"(json-get-str "name" u)"#),
        "member read → json-get-str: {out}"
    );
}

#[test]
fn nested_property_read_folds_inline() {
    let out = lower(
        "export function f(cfg: string): number {\n  return strToNum(cfg.server.port);\n}",
    );
    assert!(
        out.contains(r#"(str->num (json-get-str "server.port" cfg))"#),
        "nested reads fold into ONE dot-path call: {out}"
    );
}

// ── jsonSet / jsonQuote globals ──────────────────────────────────────────

#[test]
fn json_set_global_maps() {
    let out = lower(
        "export function f(u: string): string {\n  return jsonSet(u, \"k\", \"1\");\n}",
    );
    assert!(out.contains("(json-set u"), "jsonSet → json-set: {out}");
}

#[test]
fn json_quote_global_maps() {
    let out = lower(
        "export function f(s: string): string {\n  return jsonQuote(s);\n}",
    );
    assert!(out.contains("(json-quote s"), "jsonQuote → json-quote: {out}");
}

// ── end-to-end compile (needs runtime json-set — added same day) ────────

#[test]
fn object_round_trip_compiles() {
    compile(
        "export function make(name: string): string {\n  return { name: name, votes: 0 };\n}\n",
    );
}

#[test]
fn object_read_and_rebuild_compiles() {
    compile(
        "export function bump(u: string): string {\n  let nv = strToNum(u.votes) + 1;\n  return jsonSet(u, \"votes\", toStr(nv));\n}\n",
    );
}

// ── errors ───────────────────────────────────────────────────────────────

#[test]
fn property_assignment_hard_errors() {
    let err = ts_to_lisp_source(
        "export function f(u: string): void {\n  u.k = \"x\";\n}",
    )
    .expect_err("member assignment must hard-error");
    assert!(
        err.contains("property assignment not supported"),
        "helpful message: {err}"
    );
}

#[test]
fn object_spread_hard_errors() {
    let err = ts_to_lisp_source(
        "export function f(a: string): string {\n  return { ...a };\n}",
    )
    .expect_err("spread must hard-error");
    assert!(err.contains("spread"), "{err}");
}

// ── M2+: object-typed params (inline literal annotations) ───────────────

#[test]
fn object_param_numeric_prop_auto_decodes() {
    let out = lower(
        "export function f(u: { name: string; votes: number }): number {\n  return u.votes;\n}",
    );
    assert!(
        out.contains(r#"(str->num (json-get-str "votes" u))"#),
        "annotated numeric prop auto str->num: {out}"
    );
}

#[test]
fn object_param_string_prop_plain_read() {
    let out = lower(
        "export function f(u: { name: string }): string {\n  return u.name;\n}",
    );
    assert!(
        out.contains(r#"(json-get-str "name" u)"#),
        "string prop reads plain: {out}"
    );
}

#[test]
fn object_param_embeds_raw_in_literal() {
    let out = lower(
        "export function f(u: { name: string }): string {\n  return { wrapped: u, n: 1 };\n}",
    );
    assert!(
        out.contains(r#"(json-set "{}" "wrapped" u)"#),
        "obj param embeds RAW (no json-quote): {out}"
    );
}

#[test]
fn object_param_type_alias_resolves() {
    let out = lower(
        "type U = { name: string; votes: number };\nexport function f(u: U): number {\n  return u.votes;\n}",
    );
    assert!(
        out.contains(r#"(str->num (json-get-str "votes" u))"#),
        "type alias resolves with numeric prop: {out}"
    );
}

#[test]
fn unknown_named_type_hard_errors() {
    let err = ts_to_lisp_source(
        "export function f(u: Missing): string {\n  return u.a;\n}",
    )
    .expect_err("unknown type refs must hard-error");
    assert!(
        err.contains("inline object literal type"),
        "hint at inline: {err}"
    );
}

#[test]
fn object_param_round_trip_executes() {
    // full pipeline: typed param → read → arithmetic → rebuild → return
    compile(
        "export function vote(u: { name: string; votes: number }): string {\n  let nv = u.votes + 1;\n  return u.name + \" -> \" + toStr(nv);\n}\n",
    );
}

#[test]
fn ts_percent_compiles() {
    // `%` used to lower to a nonexistent lisp `%` (checker reject); now
    // compiles to the JS-exact truncated remainder. Sign cases verified
    // in tests/test_api_sweep.rs matrix_ts_percent_js_semantics.
    compile("export function m(): number { return -7 % 2; }\n");
}
