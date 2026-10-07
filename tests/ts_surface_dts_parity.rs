//! d.ts ↔ frontend ↔ cheatsheet surface parity (2026-10-05).
//! ts-template-lisp-rlm.d.ts says "KEEP IN SYNC — when a builtin is added
//! or renamed in ts_frontend, update this file in the same commit". That
//! was a comment, not a constraint — the Math surface drifted for days
//! until the VM-parity work caught it. This test makes the sync a hard
//! gate: every Math.* the frontend accepts must be documented in the d.ts
//! AND in the TS-arm cheatsheet; every free function the d.ts declares
//! must exist in the frontend source.

use std::fs;

const FRONTEND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ts_frontend.rs");
const DTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/crates/near-compile/skills/ts-template-lisp-rlm.d.ts"
);
const CHEATSHEET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/rlm/dream/cheatsheet-ts.txt"
);

fn math_names_frontend(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("(\"Math\", \"") {
        let after = &rest[i + 10..];
        if let Some(j) = after.find('"') {
            let name = &after[..j];
            if name != "other" && !out.contains(&name) {
                out.push(name);
            }
        }
        rest = &rest[i + 10..];
    }
    out
}

#[test]
fn math_surface_is_documented_in_dts_and_cheatsheet() {
    let fe = fs::read_to_string(FRONTEND).expect("ts_frontend.rs readable");
    let dts = fs::read_to_string(DTS).expect("d.ts readable");
    let cheat = fs::read_to_string(CHEATSHEET).expect("cheatsheet-ts.txt readable (run scripts/gen-cheatsheet-ts.py)");

    let math = math_names_frontend(&fe);
    assert!(
        math.len() >= 8,
        "expected the full Math set, found {:?} — parser drift?",
        math
    );
    for m in &math {
        assert!(
            dts.contains(&format!("Math.{m}")),
            "Math.{m} accepted by ts_frontend but missing from the d.ts — \
             update ts-template-lisp-rlm.d.ts in the same commit"
        );
        assert!(
            cheat.contains(&format!("Math.{m}")),
            "Math.{m} accepted by ts_frontend but missing from the TS-arm \
             cheatsheet — regenerate: python3 scripts/gen-cheatsheet-ts.py"
        );
    }
}

#[test]
fn dts_free_functions_exist_in_frontend() {
    let fe = fs::read_to_string(FRONTEND).expect("ts_frontend.rs readable");
    let dts = fs::read_to_string(DTS).expect("d.ts readable");

    let mut declared = Vec::new();
    for line in dts.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("declare function ") {
            if let Some(end) = rest.find('(') {
                declared.push(rest[..end].trim().to_string());
            }
        }
    }
    assert!(!declared.is_empty(), "d.ts declares no free functions?");

    for f in declared {
        assert!(
            fe.contains(&f),
            "d.ts declares `{f}` but ts_frontend.rs never mentions it — \
             removed builtin, stale d.ts, or missing lowering"
        );
    }
}
