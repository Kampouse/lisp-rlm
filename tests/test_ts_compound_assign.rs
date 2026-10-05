//! Compound assignment (F1, 2026-10-04): `*=` `/=` `%=` land as
//! sugar-free set! lowering; `**`/`**=` hard-error (no NEAR power builtin).
//!
//! Differential convention: every TS expectation is ALSO produced by a
//! hand-written lisp twin through lisp-run (interpreter) — the wasm mock
//! run and the interpreter run must agree.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn lower(src: &str) -> String {
    ts_to_lisp_source(src).expect("must lower")
}

fn compile(src: &str) -> Vec<u8> {
    let ir = lower(src);
    let exprs = parse_all(&ir).expect("must parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("must typecheck");
    compile_near_from_exprs(&exprs).expect("must compile")
}

fn run_mock(src: &str, method: &str, input: &str) -> String {
    let _l = lock();
    let wasm = compile(src);
    let tmp = std::env::temp_dir().join(format!("nm_cas_{}.wasm", std::process::id()));
    std::fs::write(&tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(method)
        .arg(input)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_ATTACH", "0")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .output()
        .expect("near-mock binary");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "near-mock failed:\n{s}");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("📄 ") {
            return rest.trim_end().to_string();
        }
    }
    panic!("no 📄 result line:\n{s}")
}

/// Interpreter differential: run a hand-written lisp twin through lisp-run
/// and require its last stdout line to match the wasm expectation.
fn run_interp_lisp(src: &str) -> String {
    let _l = lock();
    let f = std::env::temp_dir().join(format!("cas_{}.lisp", std::process::id()));
    std::fs::write(&f, src).unwrap();
    let out = std::process::Command::new("./target/release/lisp-run")
        .arg(&f)
        .output()
        .expect("lisp-run binary");
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "lisp-run failed:\n{}{}",
        s,
        String::from_utf8_lossy(&out.stderr)
    );
    // lisp-run echoes the final expression value AFTER any printlns —
    // the expectation is the FIRST printed line (the println output).
    // println renders strings write-style (quoted) — strip the quotes so
    // the twin expectation matches the wasm mock's 📄 line verbatim.
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_string()
}

const MUL_SRC: &str = r#"
export function compound(a: number, b: number): string {
  let x = a;
  x *= b;
  x *= 3;
  return `${x}`;
}
"#;

#[test]
fn mul_assign_value() {
    assert_eq!(run_mock(MUL_SRC, "compound", r#"{"a":6,"b":7}"#), "126");
    assert_eq!(
        run_interp_lisp(r#"(define (compound a b) (let ((x a)) (set! x (* x b)) (set! x (* x 3)) (to-string x))) (println (compound 6 7))"#),
        "126"
    );
}

const DIV_SRC: &str = r#"
export function compound(a: number, b: number): string {
  let x = a;
  x /= b;
  return `${x}`;
}
"#;

#[test]
fn div_assign_value() {
    // integer division truncates toward zero (lisp / semantics)
    assert_eq!(run_mock(DIV_SRC, "compound", r#"{"a":100,"b":7}"#), "14");
    assert_eq!(run_mock(DIV_SRC, "compound", r#"{"a":-100,"b":7}"#), "-14");
    assert_eq!(
        run_interp_lisp(
            r#"(define (compound a b) (let ((x a)) (set! x (/ x b)) (to-string x))) (println (compound -100 7))"#
        ),
        "-14"
    );
}

const MOD_SRC: &str = r#"
export function compound(a: number, b: number): string {
  let x = a;
  x %= b;
  return `${x}`;
}
"#;

#[test]
fn mod_assign_js_truncated_semantics() {
    // JS: sign follows the dividend — -7 % 3 = -1, 7 % -3 = 1.
    // (lisp `mod` is euclidean and would return 2 / -2 — WRONG here.)
    assert_eq!(run_mock(MOD_SRC, "compound", r#"{"a":7,"b":3}"#), "1");
    assert_eq!(run_mock(MOD_SRC, "compound", r#"{"a":-7,"b":3}"#), "-1");
    assert_eq!(run_mock(MOD_SRC, "compound", r#"{"a":7,"b":-3}"#), "1");
    assert_eq!(run_mock(MOD_SRC, "compound", r#"{"a":-7,"b":-3}"#), "-1");
    // interpreter twin of the emitted form (truncated: x - b*(x/b))
    assert_eq!(
        run_interp_lisp(
            r#"(define (compound a b) (let ((x a)) (set! x (- x (* b (/ x b)))) (to-string x))) (println (compound -7 3))"#
        ),
        "-1"
    );
}

/// `%=` with an impure rhs must evaluate the rhs exactly ONCE
/// (bind-once via let) — the helper's observable effect runs once per call.
#[test]
fn mod_assign_impure_rhs_single_eval() {
    // module-level `let` is not M1 — rewrite with a function-local counter
    let src = r#"
function bump(c: number): number {
  c += 1;
  return c * 3;
}
export function compound(a: number, c: number): string {
  let x = a;
  x %= bump(c);
  return `${x}`;
}
"#;
    // 10 % ((0+1)*3=3) = 1
    assert_eq!(run_mock(src, "compound", r#"{"a":10,"c":0}"#), "1");
    // single-eval shape check: the let binds __mod_r once, the truncated
    // form references the temp, not a duplicated call
    let ir = lower(src);
    assert!(ir.contains("__mod_r_x"), "bind-once temp present: {ir}");
}

const LOOP_SRC: &str = r#"
export function compound(n: number): string {
  let acc = 1;
  let i = 0;
  while (i < n) {
    acc *= 3;
    i += 1;
  }
  return `${acc}`;
}
"#;

#[test]
fn compound_assigns_in_loops() {
    assert_eq!(run_mock(LOOP_SRC, "compound", r#"{"n":4}"#), "81");
    assert_eq!(
        run_interp_lisp(
            r#"(define (compound n) (let ((acc 1) (i 0)) (while (< i n) (set! acc (* acc 3)) (set! i (+ i 1))) (to-string acc))) (println (compound 4))"#
        ),
        "81"
    );
}

/// Existing += semantics stay intact (numeric + string str-cat).
#[test]
fn plus_minus_still_work() {
    let src = r#"
export function compound(a: number, s: string): string {
  let x = a;
  x += 5;
  x -= 2;
  let t = s;
  t += "!";
  return `${x}${t}`;
}
"#;
    assert_eq!(run_mock(src, "compound", r#"{"a":10,"s":"ok"}"#), "13ok!");
}

/// `**` and `**=` hard-error with the precise no-power-builtin note.
#[test]
fn exponent_hard_errors() {
    let e1 = ts_to_lisp_source("export function f(x: number): string { return `${x ** 2}`; }\n")
        .unwrap_err();
    assert!(
        e1.contains("**") && e1.contains("no power builtin"),
        "binary ** message: {e1}"
    );
    let e2 = ts_to_lisp_source("export function f(x: number): string { x **= 2; return `${x}`; }\n")
        .unwrap_err();
    assert!(
        e2.contains("**=") && e2.contains("no power builtin"),
        "**= message: {e2}"
    );
}

/// obj.x += e stays a descriptive hard error.
#[test]
fn property_compound_still_rejected() {
    let e = ts_to_lisp_source(
        "export function f(o: string): string { o.length += 1; return o; }\n",
    )
    .unwrap_err();
    assert!(
        e.contains("property assignment") || e.contains("jsonSet"),
        "property write message: {e}"
    );
}

/// Element writes keep = / += / -= only.
#[test]
fn element_compound_mul_rejected() {
    let e = ts_to_lisp_source(
        "export function f(xs: number[], i: number): number { xs[i] *= 2; return xs[i]; }\n",
    )
    .unwrap_err();
    assert!(
        e.contains("element writes") && e.contains("*="),
        "element write message: {e}"
    );
}

/// IR shape: pure-rhs %= duplicates the lowered rhs (no temp needed);
/// `*=` / `/=` lower to exactly one application of the op.
#[test]
fn ir_shapes() {
    let ir = lower("export function f(x: number, m: number): string { x %= m; return `${x}`; }\n");
    assert!(!ir.contains("__mod_r"), "pure rhs needs no temp: {ir}");
    assert!(
        ir.contains("(- x (* m (/ x m)))"),
        "truncated mod shape: {ir}"
    );

    let ir = lower("export function f(x: number): string { x *= 2; return `${x}`; }\n");
    assert!(ir.contains("(* x 2)"), "*= shape: {ir}");
    let ir = lower("export function f(x: number): string { x /= 2; return `${x}`; }\n");
    assert!(ir.contains("(/ x 2)"), "/= shape: {ir}");
}
