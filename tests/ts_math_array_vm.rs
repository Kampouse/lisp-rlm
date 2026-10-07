//! TS-surface Math/array VM parity (2026-10-05).
//! Root cause chain from the RLM TS-arm traces: the frontend advertised a
//! surface (Math.*, array literals, `.length`) that only the wasm backend
//! implemented. run_program executes via the bytecode VM, whose
//! BUILTIN_NAMES whitelist rejected `array` / minted unknown `Math/pow`.
//! These tests lock the frontend↔backend contract for that surface.

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

/// Math.pow lowers to the runtime's expt builtin — never a Math/pow symbol.
#[test]
fn math_pow_lowers_to_expt() {
    let ir = lower("function f(): number { return Math.pow(2, 16); }");
    assert!(ir.contains("(expt 2 16)"), "got: {ir}");
    assert!(!ir.contains("Math/pow"), "got: {ir}");
}

/// Math.pow end-to-end (lower + typecheck + wasm compile).
#[test]
fn math_pow_compiles() {
    compile("function f(): number { return Math.pow(3, 5) + Math.abs(-2); }");
}

/// sqrt/floor join the previously-shimmed abs/max/min.
#[test]
fn math_sqrt_floor_compile() {
    compile("function f(x: number): number { return Math.floor(Math.sqrt(x)); }");
}

/// ceil/round complete the rounding trio (ceil → ceiling builtin).
#[test]
fn math_ceil_round_compile() {
    compile("function f(x: number): number { return Math.ceil(x) + Math.round(x / 2); }");
}

#[test]
fn math_ceil_lowers_to_ceiling() {
    let ir = lower("function f(x: number): number { return Math.ceil(x); }");
    assert!(ir.contains("(ceiling "), "got: {ir}");
}

/// Unsupported Math CALLS hard-error at the frontend instead of minting
/// unknown `Math/<name>` symbols that die later at compile time.
/// (Plain member reads like Math.PI fall through to object-property
/// semantics — different path, unchanged behavior.)
#[test]
fn math_unknown_hard_errors() {
    let e = lower_err("function f(): number { return Math.cos(1); }");
    assert!(e.contains("Math.cos not supported"), "got: {e}");
    let e = lower_err("function f(): number { return Math.sign(2.7); }");
    assert!(e.contains("Math.sign not supported"), "got: {e}");
}

/// Array literals lower to (array ...) and must be executable by BOTH
/// backends — the bytecode VM gained the arm in this change set.
#[test]
fn array_literal_compiles() {
    compile(
        "function f(xs: number[]): number {\n  const a: number[] = [1, 2, 3];\n  return a.length + xs.length;\n}\n",
    );
}

/// `.length` lowers to vec-length (polymorphic), not vec-len.
#[test]
fn dot_length_lowers_to_vec_length() {
    let ir = lower("function f(xs: number[]): number { return xs.length; }");
    assert!(ir.contains("(vec-length "), "got: {ir}");
    assert!(!ir.contains("(vec-len "), "got: {ir}");
}

/// The td_primesum@ts shape that died on `unknown 'array'`:
/// array literal + .reduce + Math.max pipeline.
#[test]
fn array_reduce_mathmax_pipeline_compiles() {
    compile(
        "function f(xs: number[]): number {\n  const a: number[] = [11, 13, 17, 19];\n  const sum: number = a.reduce((x: number, y: number): number => x + y, 0);\n  return Math.max(sum, xs.length);\n}\n",
    );
}

/// Free-function strJoin — declared in the d.ts since 2026-08-30 but never
/// mapped (caught by ts_surface_dts_parity on its first run, 2026-10-05).
#[test]
fn strjoin_free_function_compiles() {
    compile(
        "function f(parts: string[]): string { return strJoin(\",\", parts); }",
    );
}

/// near_* free functions — same catch class as strJoin.
#[test]
fn near_free_functions_compiles() {
    compile("function f(): string { return near_predecessor_account_id(); }");
}
