//! Executed-wasm tests for the `json-set` builtin (NEAR target).
//!
//! Harness mirrors tests/test_regression.rs (near_mock tier): compile in-process
//! with `compile_near` (type-checked), execute with ./target/release/near-mock,
//! assert the 📄 return line. `(define (main) ...)` returning a string makes
//! near-mock print the string contents in return_data (see nm_string_hello).
//!
//! Semantics under test (2026-08-31 spec):
//!   (json-set json key encoded-value) → new JSON object string
//!   - key exists at top level → value replaced IN PLACE, order preserved
//!   - key missing → `"key":<encoded-value>` inserted before the final '}'
//!   - empty/invalid json arg → treated as '{}'
//!   - encoded-value is ALREADY-ENCODED JSON value text (strings keep quotes)

use std::process::Command;

fn has_near_mock() -> bool {
    Command::new("./target/release/near-mock")
        .args(["--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile lisp to NEAR wasm (type-checked) and execute via near-mock.
/// Asserts the run succeeded (✅) and returns stdout.
fn run_near_mock(src: &str) -> String {
    let wasm = lisp_rlm_wasm::compile_near(src)
        .unwrap_or_else(|e| panic!("compile_near failed: {}", e));
    let tmp = std::env::temp_dir().join("nm_json_set.wasm");
    std::fs::write(&tmp, &wasm).unwrap();

    let output = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg("_run")
        .arg("{}")
        .output()
        .expect("near-mock should run");
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        out.contains("✅"),
        "near-mock execution failed:\nstdout:\n{}\nstderr:\n{}",
        out,
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// Extract the 📄 return-value line.
fn ret_line(out: &str) -> String {
    out.lines()
        .find(|l| l.starts_with("📄 "))
        .expect("should have 📄 return line")
        .to_string()
}

// ── 1. set on {} → {"a":"x"} ──
#[test]
fn wasm_set_on_empty_object() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out = run_near_mock(r#"(define (main) (json-set "{}" "a" "\"x\""))"#);
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"a":"x"}"#),
        "expected {{\"a\":\"x\"}}, got: {}",
        ret
    );
}

// ── 2. insert new key preserves existing order → {"a":"x","b":42} ──
#[test]
fn wasm_insert_preserves_order() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out = run_near_mock(r#"(define (main) (json-set "{\"a\":\"x\"}" "b" "42"))"#);
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"a":"x","b":42}"#),
        "expected {{\"a\":\"x\",\"b\":42}}, got: {}",
        ret
    );
}

// ── 3. replace existing key → {"a":9,"b":2} ──
#[test]
fn wasm_replace_existing_key() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out =
        run_near_mock(r#"(define (main) (json-set "{\"a\":1,\"b\":2}" "a" "9"))"#);
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"a":9,"b":2}"#),
        "expected {{\"a\":9,\"b\":2}} (order preserved), got: {}",
        ret
    );
}

// ── 4. empty-string json arg → treated as {} ──
#[test]
fn wasm_empty_string_json_arg() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out = run_near_mock(r#"(define (main) (json-set "" "a" "\"x\""))"#);
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"a":"x"}"#),
        "empty json arg should be treated as {{}}, got: {}",
        ret
    );
}

// ── 5. string value with internal quote+escape round-trips ──
// lisp literal "\"he said \\\"hi\\\"\"" → arg text: "he said \"hi\""
// (a JSON string containing escaped quotes) → output {"q":"he said \"hi\""}
#[test]
fn wasm_value_with_internal_quote_escape() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out = run_near_mock(
        r#"(define (main) (json-set "{\"z\":0}" "q" "\"he said \\\"hi\\\"\""))"#,
    );
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"z":0,"q":"he said \"hi\""}"#),
        "escaped-quote value should round-trip, got: {}",
        ret
    );
}

// ── 6. 3-key build chain (nested json-set) → full object ──
#[test]
fn wasm_three_key_build_chain() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    let out = run_near_mock(
        r#"(define (main)
             (json-set
               (json-set
                 (json-set "{}" "a" "1")
                 "b" "\"two\"")
               "c" "[1,2]"))"#,
    );
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"a":1,"b":"two","c":[1,2]}"#),
        "nested chain should build full object, got: {}",
        ret
    );
}

// ── extra: replace a key whose value is a nested object/array (value-extent
//    scanner must jump over nested braces/strings without matching inside) ──
#[test]
fn wasm_replace_nested_value_and_skip_inner_keys() {
    if !has_near_mock() {
        eprintln!("SKIP: near-mock not built");
        return;
    }
    // "b" also appears INSIDE the nested object of "n" — the scanner must
    // only consider top-level keys: n is replaced, "inner b" untouched.
    let out = run_near_mock(
        r#"(define (main)
             (json-set "{\"keep\":1,\"n\":{\"b\":2,\"c\":\"}\"},\"z\":3}" "n" "null"))"#,
    );
    let ret = ret_line(&out);
    assert!(
        ret.contains(r#"{"keep":1,"n":null,"z":3}"#),
        "nested-value replace must preserve order + not match inner keys, got: {}",
        ret
    );
}

// ── interp parity: eval_builtin json-set agrees with the wasm semantics ──
#[test]
fn interp_json_set_parity() {
    use lisp_rlm_wasm::bytecode::eval_builtin;
    use lisp_rlm_wasm::types::LispVal;

    let cases: &[(&str, &str, &str, &str)] = &[
        ("{}", "a", "\"x\"", "{\"a\":\"x\"}"),
        ("{\"a\":\"x\"}", "b", "42", "{\"a\":\"x\",\"b\":42}"),
        ("{\"a\":1,\"b\":2}", "a", "9", "{\"a\":9,\"b\":2}"),
        ("", "a", "\"x\"", "{\"a\":\"x\"}"),
        (
            "{\"z\":0}",
            "q",
            "\"he said \\\"hi\\\"\"",
            "{\"z\":0,\"q\":\"he said \\\"hi\\\"\"}",
        ),
        (
            "{\"keep\":1,\"n\":{\"b\":2,\"c\":\"}\"},\"z\":3}",
            "n",
            "null",
            "{\"keep\":1,\"n\":null,\"z\":3}",
        ),
        ("{ }", "a", "1", "{ \"a\":1}"), // whitespace-only object → no leading comma (interior space preserved)
        ("not json", "a", "1", "{\"a\":1}"), // invalid → fresh {}
    ];
    for (json, key, val, want) in cases {
        let got = eval_builtin(
            "json-set",
            &[
                LispVal::Str(json.to_string()),
                LispVal::Str(key.to_string()),
                LispVal::Str(val.to_string()),
            ],
            None,
            None,
        )
        .unwrap_or_else(|e| panic!("interp json-set {:?} failed: {}", (json, key, val), e));
        assert_eq!(
            got,
            LispVal::Str(want.to_string()),
            "interp mismatch for ({:?}, {:?}, {:?})",
            json,
            key,
            val
        );
    }

    // arity guard: 2 args must hard-error, not silently misbehave
    let bad = eval_builtin(
        "json-set",
        &[LispVal::Str("{}".into()), LispVal::Str("a".into())],
        None,
        None,
    );
    assert!(bad.is_err(), "2-arg call must error");
}

// ── shared scanner unit test (helpers::json_set_impl) ──
#[test]
fn helpers_json_set_impl_unit() {
    use lisp_rlm_wasm::helpers::json_set_impl;
    assert_eq!(json_set_impl("{}", "a", "\"x\""), r#"{"a":"x"}"#);
    assert_eq!(
        json_set_impl("{\"a\":\"x\"}", "b", "42"),
        r#"{"a":"x","b":42}"#
    );
    assert_eq!(
        json_set_impl("{\"a\":1,\"b\":2}", "a", "9"),
        r#"{"a":9,"b":2}"#
    );
    assert_eq!(json_set_impl("", "a", "\"x\""), r#"{"a":"x"}"#);
    // Leading whitespace before '{' is dropped; trailing text after '}' is
    // preserved (both consistent with the emitted __json_set helper).
    assert_eq!(
        json_set_impl("  {\"a\":1}  ", "b", "2"),
        "{\"a\":1,\"b\":2}  "
    );
}
