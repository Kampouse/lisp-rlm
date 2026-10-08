//! Ops-spec — single source of truth for the op surface (TASK-DX-AGREE
//! item 7, PoC). One const table listing every core op: name, arity
//! window, backend availability (interp / wasm), and a TS signature.
//!
//! Consumers (keep this list honest):
//!   * tests/ops_spec_test.rs — asserts wasm-flagged names exist in
//!     BUILTIN_NAMES (the real emitter surface), regenerates and
//!     golden-compares ../types.gen.ts (drift detector for the TS
//!     frontend).
//!
//! Deliberately NOT exhaustive yet: the core numeric / bitwise /
//! conversion / predicate surface that both backends share, plus the
//! documented wrap-* policy ops. Extending to the full 300+ op surface
//! is mechanical — the point of the PoC is that ONE table exists and
//! is machine-checked against the emitter.

pub struct OpSpec {
    pub name: &'static str,
    /// Minimum argument count (inclusive).
    pub min_arity: u8,
    /// Maximum argument count; None = variadic.
    pub max_arity: Option<u8>,
    /// Available in the bytecode interp backend.
    pub interp: bool,
    /// Available in the wasm emitter backend.
    pub wasm: bool,
    /// TypeScript signature fragment for types.gen.ts.
    pub ts: &'static str,
}

pub const OPS_SPEC: &[OpSpec] = &[
    // ── arithmetic (checked; hard-error policy) ──
    OpSpec { name: "+", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "-", min_arity: 1, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "*", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "/", min_arity: 1, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "mod", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(a: number, b: number) => number" },
    OpSpec { name: "abs", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(x: number) => number" },
    // ── arithmetic (explicit wrapping policy) ──
    OpSpec { name: "wrap-add", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "wrap-sub", min_arity: 1, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    OpSpec { name: "wrap-mul", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => number" },
    // ── comparison / predicates ──
    OpSpec { name: "<", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => boolean" },
    OpSpec { name: ">", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => boolean" },
    OpSpec { name: "<=", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => boolean" },
    OpSpec { name: ">=", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: number[]) => boolean" },
    OpSpec { name: "=", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: unknown[]) => boolean" },
    OpSpec { name: "!=", min_arity: 2, max_arity: None, interp: true, wasm: true, ts: "(...xs: unknown[]) => boolean" },
    OpSpec { name: "not", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(x: unknown) => boolean" }, // special form, not BUILTIN_NAMES
    // ── bitwise (payload-checked shl; see item 5 audit) ──
    OpSpec { name: "shl", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(x: number, n: number) => number" },
    OpSpec { name: "shr", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(x: number, n: number) => number" },
    OpSpec { name: "band", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(a: number, b: number) => number" },
    OpSpec { name: "bor", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(a: number, b: number) => number" },
    OpSpec { name: "bnot", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(x: number) => number" },
    // ── conversion (payload-range gated since item 5) ──
    OpSpec { name: "str->num", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(s: string) => number | false" },
    OpSpec { name: "number->string", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(n: number) => string" },
    OpSpec { name: "to-float", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(x: unknown) => number" },
    // ── lists ──
    OpSpec { name: "list", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: unknown[]) => unknown[]" },
    OpSpec { name: "length", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(xs: unknown[] | string) => number" },
    OpSpec { name: "car", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(xs: unknown[]) => unknown" },
    OpSpec { name: "cdr", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(xs: unknown[]) => unknown[]" },
    OpSpec { name: "cons", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(x: unknown, xs: unknown[]) => unknown[]" },
    OpSpec { name: "append", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xss: unknown[][]) => unknown[]" },
    // ── strings ──
    OpSpec { name: "str-cat", min_arity: 0, max_arity: None, interp: true, wasm: true, ts: "(...xs: unknown[]) => string" },
    OpSpec { name: "str-join", min_arity: 2, max_arity: Some(2), interp: true, wasm: true, ts: "(sep: string, xs: unknown[]) => string" },
    OpSpec { name: "str-len", min_arity: 1, max_arity: Some(1), interp: true, wasm: true, ts: "(s: string) => number" },
    // ── control (special forms; not BUILTIN_NAMES members) ──
    OpSpec { name: "if", min_arity: 2, max_arity: Some(3), interp: true, wasm: true, ts: "(c: boolean | unknown, t: unknown, e?: unknown) => unknown" },
];

/// Render types.gen.ts from OPS_SPEC.
pub fn render_types_gen_ts() -> String {
    let mut out = String::from(
        "// GENERATED from src/ops_spec.rs (ops-spec PoC, TASK-DX-AGREE item 7) — do not edit.\n\
         // Regenerate: cargo test --test ops_spec_test (golden-compared, fails on drift).\n\n\
         export interface LispOps {\n",
    );
    for op in OPS_SPEC {
        let name = if op.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            op.name.to_string()
        } else {
            format!("{:?}", op.name)
        };
        out.push_str(&format!("  {}: {};\n", name, op.ts));
    }
    out.push_str("}\n");
    out
}
