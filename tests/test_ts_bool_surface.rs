//! Bool-first-class surface (2026-09-30): TS boolean literals lower as
//! Bool, `: boolean` maps to :: bool, ternaries mixing bool expressions
//! with literals unify. Prior behavior: literals were Num(1|0), so
//! `hay.length > 0 ? hay.includes(x) : false` died with
//! "if: branch types disagree — bool ≠ int".
//!
//! Executed on the near-mock (pure functions, no hosts needed) plus a
//! compile gate. Expected values follow JS `===` semantics for
//! boolean-to-boolean equality.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = include_str!("../fixtures/bool_surface.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn compile(src: &str, what: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("{what}: lowering: {e}"));
    if std::env::var("TS_BOOL_DEBUG").is_ok() {
        eprintln!("── lowered IR ──\n{ir}\n────────────────");
    }
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("{what}: parse: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("{what}: typecheck: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("{what}: codegen: {e}"))
}

fn run(what: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let wasm = compile(SRC, what);
    let tmp = std::env::temp_dir().join(format!("nm_bool_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock binary (cargo build --release first)");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{what}/{method}: near-mock failed:\n{s}");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return rest.trim_end().to_string();
        }
    }
    panic!("{what}/{method}: no 📄 result line in mock output:\n{s}")
}

#[test]
fn compiles_and_typechecks() {
    let _l = lock();
    compile(SRC, "bool_surface");
}

#[test]
fn ternary_with_bool_literal_branches() {
    // the motivating construct
    assert_eq!(
        run("bool_surface", "ternaryIncludes", r#"{"hay":"hello world","needle":"world"}"#),
        "true"
    );
    assert_eq!(
        run("bool_surface", "ternaryIncludes", r#"{"hay":"hello","needle":"xyz"}"#),
        "false"
    );
    assert_eq!(
        run("bool_surface", "ternaryIncludes", r#"{"hay":"","needle":"x"}"#),
        "false"
    );
}

#[test]
fn const_flag_truthiness() {
    assert_eq!(run("bool_surface", "flagTruth", "{}"), "yes");
}

#[test]
fn annotated_boolean_returns() {
    assert_eq!(run("bool_surface", "annBoolRender", r#"{"x":2}"#), "true");
    assert_eq!(run("bool_surface", "annBoolRender", r#"{"x":1}"#), "false");
    assert_eq!(run("bool_surface", "annBoolUse", r#"{"x":5}"#), "gt");
    assert_eq!(run("bool_surface", "annBoolUse", r#"{"x":0}"#), "le");
}

#[test]
fn boolean_equality_with_literals() {
    assert!(run("bool_surface", "eqBool", r#"{"x":3}"#).contains("eq-true"));
    assert!(run("bool_surface", "eqBool", r#"{"x":0}"#).contains("eq-false"));
}

#[test]
fn logic_chains_mix_literals_and_predicates() {
    assert_eq!(run("bool_surface", "logicMix", r#"{"hay":"aaa"}"#), "and");
    assert_eq!(run("bool_surface", "logicMix", r#"{"hay":"zzz"}"#), "or");
}

#[test]
fn boolean_template_rendering() {
    assert_eq!(
        run("bool_surface", "boolRender", r#"{"flag":true}"#),
        "flag=true"
    );
    assert_eq!(
        run("bool_surface", "boolRender", r#"{"flag":false}"#),
        "flag=false"
    );
}

#[test]
fn comparison_ternary_literal_branches() {
    assert_eq!(run("bool_surface", "compareTern", r#"{"a":2,"b":1}"#), "true");
    assert_eq!(run("bool_surface", "compareTern", r#"{"a":1,"b":2}"#), "false");
}
