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
    let cheat = fs::read_to_string(CHEATSHEET)
        .expect("cheatsheet-ts.txt readable (run scripts/gen-cheatsheet-ts.py)");

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

/// The near-compile crate embeds its own copy of the contract
/// (include_str!("../skills/ts-template-lisp-rlm.d.ts"), scaffolded into
/// new projects). Two copies drift in both directions — the skills copy
/// still said `promiseThen(deposit: number)` after the str-deposit
/// migration (caught 2026-10-07). Root is the source of truth.
#[test]
fn dts_copies_are_identical() {
    let root = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ts/lisp-rlm.d.ts"))
        .expect("ts/lisp-rlm.d.ts readable");
    let skills = fs::read_to_string(DTS).expect("skills d.ts readable");
    assert_eq!(
        root, skills,
        "ts/lisp-rlm.d.ts and crates/near-compile/skills/ts-template-lisp-rlm.d.ts \
         diverged — root is the source of truth: `cp ts/lisp-rlm.d.ts \
         crates/near-compile/skills/ts-template-lisp-rlm.d.ts`"
    );
}

/// Every method on the d.ts `near` object must sit in the frontend's
/// KNOWN_NEAR_MEMBERS gate, and vice versa — both directions, no silent
/// drift. (Before this was enforced the gate was missing ALL the
/// BLS/EC-precompile members → spurious typo-warnings for documented
/// calls, while 27 accepted members were undocumented.)
#[test]
fn near_member_surface_matches_gate() {
    let fe = fs::read_to_string(FRONTEND).expect("ts_frontend.rs readable");
    let dts = fs::read_to_string(DTS).expect("d.ts readable");

    // d.ts side: `  name(` lines inside the `declare const near: { … }` block
    let block = dts
        .split("declare const near: {")
        .nth(1)
        .expect("d.ts has a `declare const near` block")
        .split("\n};")
        .next()
        .unwrap();
    let mut documented: Vec<String> = Vec::new();
    // `db: { … }` namespace block: its method lines (key/put/has/del/keys)
    // are DB surface, parsed separately against KNOWN_DB_MEMBERS — the
    // namespace entry itself is documented by the block's PRESENCE.
    let db_block = dts
        .split("  db: {\n")
        .nth(1)
        .and_then(|rest| rest.split("\n  };").next());
    let mut db_documented: Vec<String> = Vec::new();
    if let Some(dbb) = db_block {
        for line in dbb.lines() {
            let l = line.trim_start();
            if l.starts_with("//") {
                continue;
            }
            if let Some(paren) = l.find('(') {
                let name = &l[..paren];
                if !name.is_empty()
                    && !name.contains(':')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && name.chars().next().map_or(false, |c| c.is_lowercase())
                {
                    db_documented.push(name.to_string());
                }
            }
        }
    }
    assert!(
        !db_documented.is_empty(),
        "d.ts has no near.db methods in its `db: {{ … }}` block — \
         expected key/put/has/del/keys"
    );
    for line in block.lines() {
        let l = line.trim_start();
        if l.starts_with("//") {
            continue;
        }
        // Skip lines that belong to the nested db block (they were parsed
        // above); a cheap containment check keeps the main parser honest.
        if db_block.map_or(false, |dbb| dbb.contains(l)) && l.contains('(') && l.contains(':')
        {
            continue;
        }
        if let Some(paren) = l.find('(') {
            let name = &l[..paren];
            // method signature: `name(args…): ret;` — name is a bare
            // camelCase identifier (rejects `key: string` param lines,
            // which contain ": " BEFORE the paren, and index signatures)
            if !name.is_empty()
                && !name.contains(':')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && name.chars().next().map_or(false, |c| c.is_lowercase())
            {
                documented.push(name.to_string());
            }
        }
    }
    // The `db` namespace entry is documented by its block's presence (the
    // `db: {` line itself has no parens, so the line parser can't see it).
    if db_block.is_some() {
        documented.push("db".to_string());
    }
    assert!(
        documented.len() >= 80,
        "expected the full near.* surface (~100), parsed {} — parser drift? got {documented:?}",
        documented.len()
    );

    // frontend side: strings in the KNOWN_NEAR_MEMBERS table
    let table = fe
        .split("pub const KNOWN_NEAR_MEMBERS: &[&str] = &[\n")
        .nth(1)
        .expect("KNOWN_NEAR_MEMBERS table present")
        .split("\n];")
        .next()
        .unwrap();
    let mut gated: Vec<String> = Vec::new();
    // Parse line-wise: a `// group` comment line must not swallow the
    // first entry of the next line (token-wise `split("//")` ate it —
    // first entry after every comment went missing).
    for line in table.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("//") {
            continue;
        }
        for tok in l.trim_end_matches(',').split(',') {
            let t = tok.trim();
            let t = t.strip_prefix('"').unwrap_or(t);
            let t = t.strip_suffix('"').unwrap_or(t);
            if !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                gated.push(t.to_string());
            }
        }
    }
    assert!(!gated.is_empty(), "gate table parsed empty");

    // dedupe, keep order-stable
    fn uniq(v: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for x in v {
            if !out.contains(&x) {
                out.push(x);
            }
        }
        out
    }
    let documented = uniq(documented);
    let gated = uniq(gated);

    for m in &documented {
        assert!(
            gated.contains(m),
            "d.ts documents near.{m} but KNOWN_NEAR_MEMBERS omits it — \
             documented calls must not raise the typo warning"
        );
    }
    for m in &gated {
        assert!(
            documented.contains(m),
            "KNOWN_NEAR_MEMBERS accepts near.{m} but the d.ts never documents it — \
             add a d.ts entry in the same commit (the gate is derived data now)"
        );
    }

    // near.db.* methods: d.ts `db: { … }` block ↔ KNOWN_DB_MEMBERS,
    // both directions — same no-drift rule as the main gate.
    let db_table = fe
        .split("pub const KNOWN_DB_MEMBERS: &[&str] = &[")
        .nth(1)
        .expect("KNOWN_DB_MEMBERS table present")
        .split("];")
        .next()
        .unwrap();
    let mut db_gated: Vec<String> = Vec::new();
    for tok in db_table.split(',') {
        let t = tok.trim().trim_matches('"');
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            db_gated.push(t.to_string());
        }
    }
    assert!(!db_gated.is_empty(), "KNOWN_DB_MEMBERS parsed empty");
    for m in &db_documented {
        assert!(
            db_gated.contains(m),
            "d.ts documents near.db.{m} but KNOWN_DB_MEMBERS omits it — \
             update the table in the same commit"
        );
    }
    for m in &db_gated {
        assert!(
            db_documented.contains(m),
            "KNOWN_DB_MEMBERS accepts near.db.{m} but the d.ts db block never documents it — \
             add a db-block entry in the same commit"
        );
    }
}
