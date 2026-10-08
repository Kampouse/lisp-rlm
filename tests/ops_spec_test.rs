// ops-spec PoC checks (TASK-DX-AGREE item 7)
//
// 1. wasm=true ops must exist in BUILTIN_NAMES (the real emitter surface)
//    — minus special forms, which are handled by the parser, not the
//    builtin dispatch.
// 2. Names unique.
// 3. types.gen.ts golden: regenerate and byte-compare the committed file,
//    so the TS declaration cannot drift from OPS_SPEC.

use lisp_rlm_wasm::ops_spec::{render_types_gen_ts, OPS_SPEC};
use lisp_rlm_wasm::BUILTIN_NAMES;

#[test]
fn ops_spec_wasm_names_exist_in_emitter_surface() {
    let surface: std::collections::HashSet<&str> = BUILTIN_NAMES.iter().copied().collect();
    let mut missing = Vec::new();
    for op in OPS_SPEC {
        // Special forms are parsed/compiled, not dispatched as builtins.
        const SPECIAL_FORMS: &[&str] = &["if", "not"];
        if op.wasm && !surface.contains(op.name) && !SPECIAL_FORMS.contains(&op.name) {
            missing.push(op.name);
        }
    }
    assert!(
        missing.is_empty(),
        "ops_spec marks these wasm=true but the emitter surface (BUILTIN_NAMES) \
         does not carry them: {missing:?} — fix the table or the emitter, \
         then regenerate types.gen.ts"
    );
}

#[test]
fn ops_spec_names_unique() {
    let mut seen = std::collections::HashSet::new();
    for op in OPS_SPEC {
        assert!(seen.insert(op.name), "duplicate ops_spec entry: {}", op.name);
    }
}

#[test]
fn regen_types_gen_ts() {
    // REGEN=1 cargo test --test ops_spec_test regen -- writes the golden.
    if std::env::var("REGEN").is_ok() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/types.gen.ts");
        std::fs::write(path, render_types_gen_ts()).expect("write types.gen.ts");
        eprintln!("regenerated types.gen.ts");
    }
}

#[test]
fn types_gen_ts_golden() {
    let generated = render_types_gen_ts();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/types.gen.ts");
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        generated == committed,
        "types.gen.ts is stale — regenerate by writing this to the file:\n{}",
        generated
    );
}
