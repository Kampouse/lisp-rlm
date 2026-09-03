//! Playground examples — dual-gate: every TS example embedded in the
//! browser IDE (`crates/browser-compiler/web-app/src/lib/examples.ts`)
//! must compile through the SAME pipeline as fixtures.
//!
//! These are string blobs invisible to tsc; before this gate they rott
//! silently (2026-09-01: HTLC Escrow + Atomic Swap shipped broken on the
//! live site). Now every `source:` containing `export function` is
//! extracted, frontended, type-checked, and compiled. An example failing
//! here means the playground shows a compile error to users.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};

const EXAMPLES_TS: &str =
    include_str!("../crates/browser-compiler/web-app/src/lib/examples.ts");

/// Parse `name: 'X', ... source: \`...\`` entries out of examples.ts.
/// Playground entries are flat template-literal blocks; we only keep the
/// TS-dialect ones (lisp sexp sources are skipped — different pipeline).
fn extract() -> Vec<(String, String)> {
    let text = EXAMPLES_TS;
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("name: '") {
        rest = &rest[i + 7..];
        let Some(name_end) = rest.find('\'') else { break };
        let name = rest[..name_end].to_string();
        rest = &rest[name_end..];
        // source block for this entry (before the next entry's name)
        let next = rest.find("\n  {\n").map(|n| n).unwrap_or(rest.len());
        let entry = &rest[..next];
        // gate EVERY source block in the entry (main + sidecars — the app
        // compiles both; the playground must ship both clean)
        let mut cursor = 0usize;
        while let Some(rel) = entry[cursor..].find("source: `") {
            let s = cursor + rel;
            let lit = &entry[s + 9..];
            let Some(e) = lit.find("`,") else { break };
            let code = unescape_template_literal(&lit[..e]);
            cursor = s + 9 + e;
            if code.contains("export function") {
                out.push((name.clone(), code));
            }
        }
    }
    out
}

/// Mirror JS template-literal unescaping for the escape subset examples.ts
/// is allowed to use (`\\`, `\n`, `\r`, `\t`, `` \` ``, `\$`). Raw-text
/// extraction without this makes `\\"` in examples parse as `\\"` in TS
/// (Invalid Unicode escape) — while the real app evaluates it to `\"`.
/// Found 2026-09-03 via the Cross-Contract FT example.
fn unescape_template_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('`') => out.push('`'),
            Some('$') => out.push('$'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[test]
fn playground_ts_examples_compile() {
    let examples = extract();
    assert!(
        examples.len() >= 8,
        "expected >=8 TS playground examples, found {} — extractor broke?",
        examples.len()
    );
    let mut failures: Vec<String> = Vec::new();
    for (name, code) in &examples {
        let tag = format!("[{name}]");
        let ir = match ts_to_lisp_source(code) {
            Ok(ir) => ir,
            Err(e) => {
                failures.push(format!("{tag} frontend: {e}"));
                continue;
            }
        };
        let exprs = match parse_all(&ir) {
            Ok(x) => x,
            Err(e) => {
                failures.push(format!("{tag} parse: {e}"));
                continue;
            }
        };
        if let Err(e) = lisp_rlm_wasm::typing::type_check_program(&exprs, true) {
            failures.push(format!("{tag} checker: {e}"));
            continue;
        }
        if let Err(e) = compile_near_from_exprs(&exprs) {
            failures.push(format!("{tag} codegen: {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "playground examples failed to compile (site would show errors):\n{}",
        failures.join("\n")
    );
}
