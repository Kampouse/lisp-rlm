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
        let Some(s) = entry.find("source: `") else { continue };
        let lit = &entry[s + 9..];
        let Some(e) = lit.find("`,") else { continue };
        let code = &lit[..e];
        if code.contains("export function") {
            out.push((name, code.to_string()));
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
