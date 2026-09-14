//! Top-level `const` with non-literal initializers (2026-09-13).
//! `const K = <literal>` always worked (CONST_FOLDS inline substitution), but
//! expression/array initializers lower to `(define K <expr>)` — and the
//! from_exprs NEAR compile path SILENTLY SKIPPED value defines (only
//! parse_and_compile_opts handled them). Every use then failed with
//! "undefined variable 'K'". The value-define arm now registers a 0-param
//! fn + value_defines so references CALL it (interpreter value semantics).

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

const SRC: &str = r#"
const NAME = "jp" + "x";          // expression init (str-cat)
const SEED = [1, 2, 3];           // array init
const DERIVED = 6 * 7;            // arithmetic init
const MIXED = SEED[0] + SEED[1] + SEED[2]; // references another const

export function tstr(): string { return toStr(strLength(NAME)); }      // 3
export function tderived(): string { return toStr(DERIVED); }          // 42
export function tmixed(): string { return toStr(MIXED + DERIVED); }    // 48
export function tarr(): string { return toStr(SEED[1]); }              // 2
"#;

fn run(method: &str) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("tlc_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("tlc_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("tlc.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("tlc.t.near")
        .arg(method)
        .arg("{}")
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all.lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&all)
        .to_string()
}

#[test]
fn const_expr_init() {
    let r = run("tstr");
    assert!(r.contains("3"), "tstr: {r}");
}

#[test]
fn const_arith_init() {
    let r = run("tderived");
    assert!(r.contains("42"), "tderived: {r}");
}

#[test]
fn const_referencing_const() {
    let r = run("tmixed");
    assert!(r.contains("48"), "tmixed: {r}");
}

#[test]
fn const_array_indexed() {
    let r = run("tarr");
    assert!(r.contains("2"), "tarr: {r}");
}
