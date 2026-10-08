// Emit determinism (TASK-DX-AGREE item 6)
//
// The wasm emitter must be a pure function of its source: compiling the
// same file twice (fresh emitter instances) must produce byte-identical
// wasm — and identical errors on the files that fail. Anything keyed off
// HashMap iteration order, wall-clock, process ids, or an unseeded RNG
// shows up here as a flake. There is no RNG in the emit path (fixed
// random-seed: N/A — none used); this test exists to keep it that way.
//
// Corpus: every .lisp under tests/ (compiler-torture matrix, p2 sources,
// regression programs — including deliberate error files) plus fixtures/.
// Both compile modes (fuzz + near) are checked.

use std::path::PathBuf;

fn corpus() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![PathBuf::from("tests"), PathBuf::from("fixtures")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("lisp") {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(
        files.len() >= 60,
        "corpus unexpectedly small ({} files) — did tests/ move?",
        files.len()
    );
    files
}

fn assert_deterministic(path: &PathBuf, mode: &str, compile: impl Fn(&str) -> Result<Vec<u8>, String>) {
    let Ok(src) = std::fs::read_to_string(path) else { return };
    let first = compile(&src);
    let second = compile(&src);
    match (&first, &second) {
        (Ok(a), Ok(b)) => {
            assert!(
                a == b,
                "{mode}: {} emitted {} vs {} bytes — nondeterministic emit",
                path.display(),
                a.len(),
                b.len()
            );
        }
        (Err(e1), Err(e2)) => {
            assert_eq!(
                e1, e2,
                "{mode}: {} produced different errors across runs",
                path.display()
            );
        }
        _ => panic!(
            "{mode}: {} flip-flops between Ok and Err across runs — nondeterministic",
            path.display()
        ),
    }
}

#[test]
fn test_emit_deterministic() {
    std::thread::Builder::new()
        .name("emit-determinism".into())
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let files = corpus();
            let mut checked = 0usize;
            for path in &files {
                assert_deterministic(path, "fuzz", |src| {
                    lisp_rlm_wasm::wasm_emit::compile_fuzz(src)
                });
                assert_deterministic(path, "near", |src| {
                    lisp_rlm_wasm::wasm_emit::compile_near(src)
                });
                checked += 1;
            }
            eprintln!("emit determinism: {checked} files x 2 modes x 2 runs — all byte-identical");
        })
        .expect("spawn determinism thread")
        .join()
        .expect("determinism thread panicked");
}
