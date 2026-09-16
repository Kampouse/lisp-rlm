//! Forward references (2026-09-15) — definition-before-use.
//!
//! TS semantics hoist function declarations and consts; the compiler had
//! three order-dependent gaps that made forward references fail while
//! normal TS code written top-down just worked... until it didn't:
//!
//! 1. EMITTER (from_exprs path): defines compiled strictly in source
//!    order — a call to a later-defined helper died with "unknown
//!    function". The checker PRE-REGISTERS function names (mutual
//!    recursion), so the checker passed and the emitter failed — a trap.
//!    The source path (compile_near) always had the pre-scan; the
//!    from_exprs path (used by every test + the CLI) didn't.
//!    Fix: the from_exprs driver pre-registers all function + value
//!    define names before emitting any body.
//! 2. CHECKER: value defines (consts) weren't pre-registered — a const
//!    referenced above its define failed with "undefined variable".
//!    Fix: the pre-pass registers value defines too.
//! 3. FRONTEND: single-pass lowering meant a function ABOVE a literal
//!    const couldn't fold it (CONST_FOLDS was empty at that point) —
//!    the reference emitted as a bare Sym nobody defined.
//!    Fix: pre-scan of all top-level literal consts before any lowering.
//!
//! Found compiling the PLONK verifier (functions reordered for
//! dependency hygiene hit all three).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function caller(): number {
  return helper(3) + USE_AHEAD() + strToNum(AHEAD_CONST);
}
function helper(n: number): number { return n * 2; }
export function USE_AHEAD(): number { return 7; }
const AHEAD_CONST = "10";
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

#[test]
fn forward_refs_compile_and_run() {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).expect("frontend (const pre-scan)");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .expect("checker (value-define pre-registration)");
    let wasm = compile_near_from_exprs(&exprs).expect("emit (from_exprs pre-scan)");
    assert!(!wasm.is_empty());

    let p = std::env::temp_dir().join(format!("fwd_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg("/tmp/fwd-test-state.bin")
        .arg(format!("t={}", p.display()))
        .arg("t")
        .arg("caller")
        .arg("{}")
        .arg("--json")
        .output()
        .expect("near-mock spawn");
    let _ = std::fs::remove_file("/tmp/fwd-test-state.bin");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    // number returns render as raw LE bytes — assert on the JSON blob
    // (return "\u0017..." = 0x17 = 23 = helper(3) + 7 + 10)
    assert!(
        text.contains("\\u0017"),
        "caller must return 23 (0x17): {text}"
    );
}

#[test]
fn mutual_recursion_through_from_exprs() {
    // the classic case the checker always allowed — now the emitter too.
    // NOTE: only NON-EXPORTED fns can call each other with args — exported
    // fns are 0-lisp-param entry points (JSON-arg convention), and calling
    // one as a helper with args is an arity error in ANY order (pre-
    // existing, orthogonal to forward refs).
    let _l = lock();
    let src = r#"
function isEven(n: number): number {
  if (n == 0) { return 1; }
  return isOdd(n - 1);
}
function isOdd(n: number): number {
  if (n == 0) { return 0; }
  return isEven(n - 1);
}
export function check(): number { return isEven(10); }
"#;
    let ir = ts_to_lisp_source(src).expect("lowering");
    let exprs = parse_all(&ir).expect("parse");
    let wasm = compile_near_from_exprs(&exprs).expect("mutual recursion emits");
    assert!(!wasm.is_empty());
}
