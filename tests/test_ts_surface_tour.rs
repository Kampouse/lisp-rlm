//! Surface Tour 2 — runtime leg of the 2026-09-01 TS-surface audit.
//! Compile-only assertions for the exotic crypto live in the same fixture
//! (surface_tour2_exotic.ts) and are gated by ts compile fixtures already;
//! here we RUN every mock-supported host call surface_tour2.ts exposes.

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

const TOUR: &str = include_str!("../fixtures/surface_tour2.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn wasm() -> Vec<u8> {
    let ir = ts_to_lisp_source(TOUR).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    compile_near_from_exprs(&exprs).unwrap()
}

struct Call<'a> { method: &'a str, args: &'a str }

fn run(c: Call) -> String {
    let _l = lock();
    let p = std::env::temp_dir().join(format!("st2_{}.wasm", std::process::id()));
    std::fs::write(&p, wasm()).unwrap();
    let manifest = format!("st2.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross").arg("{}").arg(&manifest)
        .arg("st2.t.near").arg(c.method).arg(c.args)
        .output()
        .expect("near-mock spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // mock prints the return value on a `📄 <json>` line
    stdout
        .lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&stdout.to_string())
        .to_string()
}

#[test]
fn tour2_str_methods() {
    let out = run(Call { method: "strMethods", args: "{}" });
    // startsWith(S) endsWith(E) includes(I) indexOf(6) charAt(e) slice(hello)
    // concat(...) length(14) — source order is S then E then I
    assert!(out.contains("SEI(6)"), "predicates failed: {out}");
    assert!(out.contains("ehello"), "charAt+slice failed: {out}");
    assert!(out.contains("lisp-rlm!"), "concat failed: {out}");
    assert!(out.contains("[14]"), "length failed: {out}");
}

#[test]
fn tour2_syntax() {
    let out = run(Call { method: "syntaxTour", args: "{}" });
    // abc:31 — for..of + i++/+= + template
    // (arrow removed: lambda-in-let breaks the template fold —
    //  see tour2_regression_lambda_template_fold)
    assert!(out.contains("abc:31"), "syntax tour: {out}");
}

#[test]
fn tour2_context_hosts() {
    let out = run(Call { method: "ctx", args: "{}" });
    // 7 colon-separated fields. Mock semantics verified 2026-09-01:
    // randomSeed → 64 hex chars (hex-encoded 32B), signerAccountPk 51 chars
    // (base58), gas fields true, depositGte(0,0) → bool true; `${ok == 1}`
    // compares across tags (bool vs num) → false per interpreter parity.
    assert!(out.contains("64:51:true:true:true:true:false"), "ctx hosts: {out}");
}

#[test]
fn tour2_input() {
    let out = run(Call { method: "inputEcho", args: "{\"k\":123}" });
    assert!(out.contains("k"), "input() should echo args json: {out}");
}

#[test]
fn tour2_iter_noop_safe() {
    let out = run(Call { method: "iterProbe", args: "{}" });
    assert!(out.contains("iter"), "iter prefix/next trapped: {out}");
}

#[test]
fn tour2_num_storage_roundtrip() {
    let out = run(Call { method: "numStorage", args: "{}" });
    assert!(
        out.contains("340282366920938463463374607431768211455"),
        "u128 num-storage roundtrip: {out}"
    );
}

#[test]
fn tour2_money_noop_safe() {
    let out = run(Call { method: "money", args: "{}" });
    assert!(out.contains("money-ok"), "transfer/batch-create trapped: {out}");
}

#[test]
fn tour2_json_arr() {
    let out = run(Call { method: "jsonArr", args: "{\"ks\":[\"alpha\",\"beta\"]}" });
    assert!(out.contains("alpha"), "jsonArr[0]: {out}");
}

#[test]
fn tour2_exotic_compiles() {
    // compile-only: keccak512/ripemd160/p256/ecrecover/altBn128/bls12381
    // hosts are absent from the mock — instantiating would fail linking.
    // The audit gate is frontend + checker + codegen.
    let ir = ts_to_lisp_source(include_str!("../fixtures/surface_tour2_exotic.ts")).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    compile_near_from_exprs(&exprs).unwrap();
}

/// FIXED (2026-09-01): a lambda bound in a let inside the function no longer
/// clobbers the subsequent template-literal fold. Root cause: call.rs's
/// local-closure dispatch emitted `Call; Return` — a tail-call opt that
/// returned from the ENCLOSING function, silently discarding everything
/// after a non-tail lambda call (later statements, template siblings).
/// Now uses the result-local pattern (dynamic_call.rs): every dispatch arm
/// stores to one local, read once after the chain.
#[test]
fn tour2_regression_lambda_template_fold() {
    let src = r#"export function m(): string {
  let n = 31;
  let double = (v: bigint): bigint => v * 2n;
  return `x:${n}:y:${double(21n)}`;
}"#;
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("st2reg_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross").arg("{}").arg(format!("reg.t.near={}", p.display()))
        .arg("reg.t.near").arg("m").arg("{}")
        .output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("x:31:y:42"), "lambda+template fold still broken: {s}");
}
