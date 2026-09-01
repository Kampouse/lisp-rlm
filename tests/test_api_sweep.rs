//! API-surface sweep (2026-08-31).
//!
//! The corpus tests PROGRAMS; nothing asked "does every builtin compile at
//! all, and what does each do with a missing key / empty input?" — that gap
//! let near/log_num and near/json_get_int sit as never-compiled dead paths
//! and missing-key reads return input tail bytes.
//!
//! Tier 1: every pure builtin COMPILES and VALIDATES (dead-path detector).
//! Tier 2: the json input-read family runs against an edge-input matrix
//!         ({} / missing key / present key / pretty-printed whitespace)
//!         with exact-value assertions.
//!
//! Harness mirrors tests/test_json_set.rs (near-mock tier).

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn near_mock(src: &str, method: &str, input: &str) -> String {
    let wasm = lisp_rlm_wasm::compile_near(src)
        .unwrap_or_else(|e| panic!("compile_near failed: {}", e));
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "nm_api_sweep_{}_{}.wasm",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, &wasm).unwrap();
    let _ = method; // compile_near exports everything as `_run`
    let output = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg("_run")
        .arg(input)
        .output()
        .expect("near-mock should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Tier-1: the call compiles AND the produced wasm validates AND executes
/// without trapping on `{}` input. `(near/log (to-string ...))` the result
/// so execution reaches the value.
fn compiles_and_runs(name: &str, call: &str) {
    let src = format!(
        r#"(define (main) (near/log (to-string {call})))"#,
        call = call
    );
    let out = near_mock(&src, "main", "{}");
    assert!(
        out.contains("✅ Success"),
        "[{name}] must compile+validate+run: {}",
        out
    );
}

// ── Tier 1: pure surface — dead-path detector ─────────────────────────────

#[test]
fn sweep_input_reads() {
    compiles_and_runs("json_get_str", r#"(near/json_get_str "g")"#);
    compiles_and_runs("json_get_int", r#"(near/json_get_int "n")"#);
    compiles_and_runs("json_get_arr", r#"(near/json_get_arr "a")"#);
}

#[test]
fn sweep_json_builders() {
    compiles_and_runs("json-set", r#"(json-set "{}" "k" "v")"#);
    compiles_and_runs("json-get auto", r#"(json/get "k")"#);
    compiles_and_runs("json-get-str", r#"(json-get-str "k" "{}")"#);
    compiles_and_runs("json-quote", r#"(json-quote "hi")"#);
    compiles_and_runs("json-return", r#"(json-return "{}")"#);
}

#[test]
fn sweep_logging() {
    // near/log_num was a never-compiled dead path until 2026-08-31
    compiles_and_runs("log", r#"(near/log "x")"#);
    compiles_and_runs("log_num", r#"(near/log_num 7)"#);
    compiles_and_runs("log_num_neg", r#"(near/log_num -7)"#);
}

#[test]
fn sweep_returns() {
    compiles_and_runs("return_str", r#"(near/return_str "x")"#);
    compiles_and_runs("json_return_str", r#"(near/json_return_str "x")"#);
    compiles_and_runs("json_return_int", r#"(near/json_return_int 3)"#);
}

#[test]
fn sweep_context() {
    compiles_and_runs("signer_account_id", r#"(near/signer_account_id)"#);
    compiles_and_runs("random_seed", r#"(near/random_seed)"#);
}

// ── Tier 2: edge-input matrix with exact values ───────────────────────────

fn log_of(call: &str, input: &str) -> String {
    let src = format!(
        r#"(define (main) (near/log (to-string {call})))"#,
        call = call
    );
    let out = near_mock(&src, "main", input);
    let line = out
        .lines()
        .find(|l| l.trim_start().starts_with("LOG:"))
        .unwrap_or_else(|| panic!("no LOG line for [{call}] input={input}: {out}"))
        .trim_start()
        .to_string();
    // strip the [debug ...] tail
    line.split("  [debug").next().unwrap().to_string()
}

#[test]
fn matrix_json_get_str() {
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, r#"{"g":"hi"}"#), "LOG: hi");
    // miss → nil (d.ts `string | null`; bare use is visible)
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, "{}"), "LOG: nil");
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, r#"{"x":1}"#), "LOG: nil");
    // pretty-printed JSON (whitespace after colon) must still hit
    assert_eq!(
        log_of(r#"(near/json_get_str "g")"#, r#"{ "g" : "hi" }"#),
        "LOG: hi"
    );
    // empty input
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, ""), "LOG: nil");
}

#[test]
fn matrix_json_get_str_nullish() {
    assert_eq!(
        log_of(
            r#"(default (near/json_get_str "g") "fb")"#,
            "{}"
        ),
        "LOG: fb"
    );
    assert_eq!(
        log_of(
            r#"(default (near/json_get_str "g") "fb")"#,
            r#"{"g":"hi"}"#
        ),
        "LOG: hi"
    );
}

#[test]
fn matrix_json_get_int() {
    assert_eq!(log_of(r#"(near/json_get_int "n")"#, r#"{"n":42}"#), "LOG: 42");
    assert_eq!(log_of(r#"(near/json_get_int "n")"#, r#"{"n":-42}"#), "LOG: -42");
    assert_eq!(log_of(r#"(near/json_get_int "n")"#, "{}"), "LOG: nil");
    // bare-token value ({"by": 5} — no quotes around 5) must parse
    assert_eq!(log_of(r#"(near/json_get_int "n")"#, r#"{"n":5}"#), "LOG: 5");
    // hit-then-miss in one function: local reuse must not leak the hit
    let src = r#"(define (main) (begin (near/log_num (near/json_get_int "a")) (near/log (to-string (near/json_get_int "b")))))"#;
    let out = near_mock(src, "main", r#"{"a":5}"#);
    assert!(
        out.contains("LOG: 5") && out.contains("LOG: nil"),
        "hit-then-miss must not leak: {out}"
    );
}

#[test]
fn matrix_json_get_int_nullish() {
    assert_eq!(
        log_of(r#"(default (near/json_get_int "n") 7)"#, "{}"),
        "LOG: 7"
    );
    assert_eq!(
        log_of(r#"(default (near/json_get_int "n") 7)"#, r#"{"n":1}"#),
        "LOG: 1"
    );
}

#[test]
fn matrix_prefix_and_collision_keys() {
    // "g" must not match inside "gg" or "g2"
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, r#"{"gg":"v"}"#), "LOG: nil");
    assert_eq!(log_of(r#"(near/json_get_str "g")"#, r#"{"a":1,"g2":2}"#), "LOG: nil");
    // key later in the object
    assert_eq!(
        log_of(r#"(near/json_get_str "g")"#, r#"{"a":1,"g":"late"}"#),
        "LOG: late"
    );
}

#[test]
fn matrix_ts_percent_js_semantics() {
    // wasm executes the compiled TS (compile_near path won't parse TS —
    // use the lisp the TS lowers to, exact form)
    let cases = [
        (r#"(near/log_num (- 10 (* 3 (/ 10 3))))"#, "LOG: 1"),   // 10%3
        (r#"(near/log_num (- -7 (* 2 (/ -7 2))))"#, "LOG: -1"),  // -7%2 (JS: -1)
        (r#"(near/log_num (- 7 (* -2 (/ 7 -2))))"#, "LOG: 1"),   // 7%-2 (JS: 1)
        (r#"(near/log_num (- -7 (* -2 (/ -7 -2))))"#, "LOG: -1"),// -7%-2 (JS: -1)
    ];
    for (call, expect) in cases {
        let src = format!(r#"(define (main) {call})"#);
        let out = near_mock(&src, "main", "{}");
        assert!(
            out.contains(expect),
            "{call} must give {expect}: {out}"
        );
    }
}
