//! Raw object/array-valued args decode as balanced spans (TASK-json-get-str-object-args).
//!
//! The task found (2026-09-03) that raw `{...}`/`[...]` values in tx input
//! decoded to empty — the scanner stopped at the first inner `,`/`}`/`]`.
//! Fixed 2026-09-14 (depth-tracked balanced scan in json_get_str +
//! __json_get's object branch); the JSON API v3 input-handle getters and the
//! TS object-param surface (root-keyed convention) inherit the fix.
//!
//! These tests pin the behavior so it can't silently regress:
//! 1. wasm: raw object/array values round-trip as raw JSON text
//! 2. wasm: dot-path + array-index reads THROUGH a raw span work
//! 3. TS frontend: `o.a.b` nested handle reads + typed object params compile
//! 4. near-mock e2e: the playground Objects example shapes on real wasm
//!
//! Convention note (pinned by test_ts_objects): object-param fields read
//! from the args ROOT — the param name is erased. `{ "b": {...} }` is the
//! WRONG call shape for `cast(b: Ballot)`; pass `{ "title": ..., ... }`.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near, compile_near_from_exprs, parse_all};

fn to_wat(wasm: &[u8]) -> String {
    wasmprinter::print_bytes(wasm).expect("wasmprinter")
}

/// Compile + run via near-mock; per-test unique paths (parallel-safe),
/// pipeline parity asserted in-process (source vs exprs byte-identical).
fn run_near_mock(src: &str, method: &str, args: &str) -> String {
    let wasm = compile_near(src).unwrap_or_else(|e| panic!("compile_near failed: {e}"));
    let exprs = parse_all(src).unwrap_or_else(|e| panic!("parse_all failed: {e}"));
    let wasm2 = compile_near_from_exprs(&exprs)
        .unwrap_or_else(|e| panic!("compile_near_from_exprs failed: {e}"));
    assert_eq!(wasm, wasm2, "pipeline divergence: source vs exprs wasm");
    static NM_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tag = format!(
        "{}_{}",
        std::process::id(),
        NM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let tmp = std::env::temp_dir().join(format!("nm_objargs_{tag}.wasm"));
    std::fs::write(&tmp, &wasm).unwrap();
    let state = std::env::temp_dir().join(format!("nm_objargs_state_{tag}.bin"));
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(args)
        .env("NEAR_MOCK_STATE", &state)
        .env("NEAR_MOCK_QUIET", "1")
        .output()
        .expect("near-mock should run");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&state);
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn extract_return(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.starts_with("📄 "))
        .map(|l| l.to_string())
}

fn has_near_mock() -> bool {
    std::path::Path::new("./target/release/near-mock").exists()
}

// ── Tier 1: compile-shape pin ─────────────────────────────────────────

#[test]
fn raw_object_value_json_get_str_compiles() {
    let src = r#"
(define (cast resp)
    (json-get-str "b.title" resp))
"#;
    let wasm = compile_near(src).expect("dot-path through raw object compiles");
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit calls");
}

// ── Tier 2: near-mock execution (raw spans decode to raw JSON text) ───

#[test]
fn nm_raw_object_value_returns_raw_span() {
    if !has_near_mock() {
        return;
    }
    // Object VALUE extracted whole: inner getter pulls the `data` span
    // from tx input, outer pulls `cfg` out of it → the raw nested span.
    // (Deep-key misses over spans were corrupted until 2026-10-09 —
    // pinned by nm_miss_over_*_span below.)
    let src = r#"
(define (main)
  (json-get-str "cfg" (near/json_get_str "data")))
"#;
    let out = run_near_mock(
        src,
        "_run",
        r#"{"data": {"cfg": {"server": {"port": "80"}}}}"#,
    );
    let ret = extract_return(&out).expect("should have return value");
    assert!(
        ret.contains("server"),
        "raw object value should return the balanced span, got: {ret}"
    );
}

#[test]
fn nm_raw_array_value_returns_raw_span() {
    if !has_near_mock() {
        return;
    }
    let src = r#"
(define (main)
  (json-get-str "tags" (near/json_get_str "cfg")))
"#;
    let out = run_near_mock(
        src,
        "_run",
        r#"{"cfg": {"server": {"port": "8080"}, "tags": ["a","b"]}}"#,
    );
    let ret = extract_return(&out).expect("should have return value");
    assert!(
        ret.contains(r#"["a","b"]"#),
        "raw array value should return the balanced span, got: {ret}"
    );
}

#[test]
fn nm_empty_object_value() {
    if !has_near_mock() {
        return;
    }
    // Depth-1 close on the FIRST byte: {} must terminate the span.
    let src = r#"
(define (main)
  (json-get-str "x" "{\"x\":{},\"y\":2}"))
"#;
    let out = run_near_mock(src, "_run", "{}");
    let ret = extract_return(&out).expect("should have return value");
    assert!(
        ret.contains("{}"),
        "empty object value should return '{{}}', got: {ret}"
    );
}

#[test]
fn nm_dot_path_through_raw_span() {
    if !has_near_mock() {
        return;
    }
    // The v3 nested-handle shape: top key via the input getter (its span
    // is a full JSON value), remainder via the buffer dot-path scanner.
    let src = r#"
(define (main)
  (json-get-str "server.port" (near/json_get_str "cfg")))
"#;
    let out = run_near_mock(src, "_run", r#"{"cfg": {"server": {"port": "9090"}}}"#);
    let ret = extract_return(&out).expect("should have return value");
    assert!(ret.contains("9090"), "dot-path into raw span, got: {ret}");
}

#[test]
fn nm_array_index_through_raw_span() {
    if !has_near_mock() {
        return;
    }
    let src = r#"
(define (main)
  (json-get-str "items[1].name" (near/json_get_str "data")))
"#;
    let out = run_near_mock(
        src,
        "_run",
        r#"{"data": {"items": [{"name":"first"},{"name":"second"}]}}"#,
    );
    let ret = extract_return(&out).expect("should have return value");
    assert!(ret.contains("second"), "index into raw span, got: {ret}");
}

#[test]
fn nm_string_encoded_still_works() {
    if !has_near_mock() {
        return;
    }
    // Legacy string-encoded form must keep working beside the raw form.
    // 2-arg json-get-str over a string-encoded span (the pre-2026-09-14
    // calling convention — callers that escaped the object themselves).
    let src = r#"
(define (main)
  (json-get-str "cfg" "{\"cfg\":{\"server\":{\"port\":\"80\"}}}"))
"#;
    let out = run_near_mock(src, "_run", "{}");
    let ret = extract_return(&out).expect("should have return value");
    assert!(ret.contains("80"), "string-encoded form, got: {ret}");
}

// ── Tier 2b: miss-over-span regressions (2026-10-09 fix) ──────────────
//
// History: a 2-arg json-get-str MISS over a buffer that came from a raw
// span returned TAIL GARBAGE instead of empty ({"cfg":{"server":{"port":
// "XYZ"}}}, key "nope" → `YZ"`; key over a string span → `2345`). Root
// cause: __json_get's `temp` local doubled as byte-scratch and match
// flag — when the final scan iteration skipped the compare (depth != 1),
// the bounds-exit left a raw buffer byte in temp and the not-found gate
// misread it as "match found", running the value extractor from
// end-of-buffer. Fixed by forcing temp=0 on the bounds-exit path.
// Miss over a literal buffer was never affected (its last iteration
// always ran a compare, leaving temp=0).

#[test]
fn nm_miss_over_object_span_returns_empty() {
    if !has_near_mock() {
        return;
    }
    let src = r#"
(define (main)
  (json-get-str "nope" (near/json_get_str "cfg")))
"#;
    let out = run_near_mock(src, "_run", r#"{"cfg": {"server": {"port": "XYZ"}}}"#);
    assert!(
        !out.contains("📄"),
        "miss over an object span must return empty (no value line), got: {out}"
    );
}

#[test]
fn nm_miss_over_string_span_returns_empty() {
    if !has_near_mock() {
        return;
    }
    // String-valued span: the whole buffer is inside a quote, so every
    // scan iteration takes the depth != 1 path (the old corruption source).
    let src = r#"
(define (main)
  (json-get-str "zzz" (near/json_get_str "cfg")))
"#;
    let out = run_near_mock(src, "_run", r#"{"cfg": "hello world 12345"}"#);
    assert!(
        !out.contains("📄"),
        "miss over a string span must return empty (no value line), got: {out}"
    );
}

#[test]
fn nm_deep_key_hit_still_works() {
    if !has_near_mock() {
        return;
    }
    // The fix must not break deep matches: key sits past a nested object.
    let src = r#"
(define (main)
  (json-get-str "name" (near/json_get_str "info")))
"#;
    let out = run_near_mock(src, "_run", r#"{"info": {"name": "ABCDEFGHI"}}"#);
    let ret = extract_return(&out).expect("deep hit should return a value");
    assert!(
        ret.contains("ABCDEFGHI"),
        "deep key match should extract the value, got: {ret}"
    );
}

// ── Tier 3: TS frontend (the surface playground authors use) ──────────

fn ts_compile(src: &str) -> Vec<u8> {
    let lisp = ts_to_lisp_source(src).expect("ts frontend");
    let exprs = parse_all(&lisp).expect("parse lowered lisp");
    // Pipeline parity guard: exprs entrypoint must match the source path.
    let via_source = compile_near(&lisp).expect("source-path compile");
    let via_exprs = compile_near_from_exprs(&exprs).expect("exprs-path compile");
    assert_eq!(
        via_source, via_exprs,
        "pipeline divergence on TS-lowered source"
    );
    via_source
}

#[test]
fn ts_nested_handle_read_compiles() {
    // o.cfg.server.port → top key via input getter, rest via dot-path.
    let wasm = ts_compile(
        r#"
export function getPort(): string {
  const o = near.input();
  return o.cfg.server.port;
}
"#,
    );
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit calls");
}

#[test]
fn ts_typed_object_param_raw_args_compile() {
    let wasm = ts_compile(
        r#"
export function cast(b: { title: string, votes: number }): string {
  return b.title + ":" + (b.votes + 1);
}
"#,
    );
    let wat = to_wat(&wasm);
    assert!(wat.contains("call"), "should emit calls");
}
