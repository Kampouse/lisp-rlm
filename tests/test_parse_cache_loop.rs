//! Parse-cache staleness on while/for loop-backs (2026-10-10).
//!
//! Level 1.5 u128 parse cache: a tagged Sym operand memoizes its limbs in a
//! per-name (flag, lo, hi) triple. Invalidation at set!/let sites is LAZY —
//! `emit_parse_cache_invalidate` is a no-op when the memo entry is first
//! allocated AFTER that site in emission order. In a while loop whose body
//! assigns a u128-string local before any read of it (`(set! s (sumq mid))`
//! then later `(u128/lt s y)`), iteration 1's read fills the memo and every
//! later read copies the stale limbs — the loop never converges (burns all
//! gas). Same class as the TC loop-back fix (mod.rs, 2026-10-06); while/for
//! just never got the force-allocate treatment.
//!
//! Detector discipline: the repros are the RED gate — they fail on the
//! unfixed emitter, green after the fix. The cond-only negative control
//! guards against over-invalidation.

use std::process::Command;

/// Unique per-run temp dir (mock temp-file collisions — atomically unique).
fn uniq_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("pc_loop_{}_{}_{}", tag, std::process::id(), n))
}

/// Compile `src` with the compile bin, run `method` (with `args` JSON) via
/// near-mock, return stdout.
fn run_method(tag: &str, src: &str, method: &str, args: &str) -> Result<String, String> {
    let dir = uniq_dir(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("src/main.lisp"), src).map_err(|e| e.to_string())?;
    let inp = dir.join("src/main.lisp");
    let wasm = dir.join("target/pcl.wasm");
    std::fs::create_dir_all(dir.join("target")).map_err(|e| e.to_string())?;
    let out = Command::new(env!("CARGO_BIN_EXE_compile"))
        .arg(&inp)
        .arg(&wasm)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "compile failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_near-mock"))
        .arg(&wasm)
        .arg(method)
        .arg(args)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(stdout)
}

/// Compile TS `src` through the TS frontend via the compile bin, run
/// `method` (with `args` JSON) via near-mock, return stdout.
fn run_method_ts(tag: &str, src: &str, method: &str, args: &str) -> Result<String, String> {
    let dir = uniq_dir(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let inp = dir.join("src/main.ts");
    let wasm = dir.join("target/pcl.wasm");
    std::fs::create_dir_all(dir.join("target")).map_err(|e| e.to_string())?;
    std::fs::write(&inp, src).map_err(|e| e.to_string())?;
    let out = Command::new(env!("CARGO_BIN_EXE_compile"))
        .arg(&inp)
        .arg(&wasm)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "compile failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_near-mock"))
        .arg(&wasm)
        .arg(method)
        .arg(args)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(stdout)
}

/// THE REPRO (red on unfixed emitter): bsearch with a tagged (non-limb)
/// body assignment read by the cond path. `s` is set! before any read →
/// stale memo → lo/hi freeze at iteration-1 values → 200 Tgas burn.
/// Expected: converges to "4000000000".
#[test]
fn loop_bsearch_must_converge() {
    let src = r#"
(define (sumq p)
  (u128/add (u128/muldiv "100300902708124373119359" (u128/sub p "1000000000") "1")
            (if (u128/lt "2000000000" p)
                (u128/muldiv "60180541624874623871615" (u128/sub p "2000000000") "1")
                "0")))
(define (bs)
  (let ((y "200000000000000000000000000000000") (lo "1000000000") (hi "4000000000") (it 0) (mid "0") (s "0"))
    (begin
      (while (< it 48)
        (begin
          (set! mid (u128/div (u128/add lo hi) "2"))
          (set! s (sumq mid))
          (if (u128/lt s y)
              (set! lo (u128/add mid "1"))
              (set! hi mid))
          (set! it (+ it 1))))
      hi)))
(export "run" bs)
"#;
    // y baked as a literal: oracle answer is INTERIOR (2621250000) — a
    // frozen loop leaves hi=4000000000, so the assert discriminates.
    let out = run_method("bs", src, "run", "{}").expect("run");
    assert!(
        out.contains("2621250000"),
        "STALE PARSE CACHE: loop did not converge (expected hi=2621250000, got: {out})"
    );
}

/// Negative control (green before AND after): identical loop family but
/// the body never set!s a value that a later u128 operand reads — the
/// cond reads a limb local directly — must stay green (the fix must not
/// perturb behavior that was already correct).
#[test]
fn cond_only_loop_unchanged() {
    let src = r#"
(define (f)
  (let ((lo "1000000000") (it 0))
    (begin
      (while (u128/lt lo "1000000006")
        (begin (set! lo (u128/add lo "1")) (set! it (+ it 1))))
      lo)))
(export "run" f)
"#;
    let out = run_method("co", src, "run", "{}").expect("run");
    assert!(out.contains("1000000006"), "got: {out}");
}

/// TS `for` lowers to the same while machinery — but `for` sugar gets its
/// own emitter arm, so exercise it through the TS frontend (attach makes
/// the answer interior: sumq crosses y inside (G0, G2)).
#[test]
fn ts_for_loop_bsearch_must_converge() {
    let src = r#"
const Q0 = "100300902708124373119359";
const Q1 = "60180541624874623871615";
function sumq(p: string): string {
  if (u128Lt("2000000000", p)) {
    return u128Add(u128MulDiv(Q0, u128Sub(p, "1000000000"), "1"), u128MulDiv(Q1, u128Sub(p, "2000000000"), "1"));
  }
  return u128MulDiv(Q0, u128Sub(p, "1000000000"), "1");
}
export function run(): string {
  const y = "200000000000000000000000000000000";
  let lo = "1000000000";
  let hi = "4000000000";
  for (let it = 0; it < 48; it++) {
    const mid = u128Div(u128Add(lo, hi), "2");
    const s = sumq(mid);
    if (u128Lt(s, y)) {
      lo = u128Add(mid, "1");
    } else {
      hi = mid;
    }
  }
  return hi;
}
"#;
    let out = run_method_ts("for", src, "run", "{}").expect("run");
    assert!(
        out.contains("2621250000"),
        "STALE PARSE CACHE (ts for): got: {out}"
    );
}

/// HOF pipeline params are rebound every iteration (map/filter/reduce
/// emitters) — the param's parse-cache memo must be invalidated at each
/// rebind, or every element after the first re-serves element 0's limbs
/// (filter returned count 0 / map returned element-0-derived values).
/// Repro was RED before the param-rebind invalidation: single .filter with
/// a u128 predicate on a string array returned 0 instead of 2.
#[test]
fn hof_pipeline_param_rebind_must_not_stale() {
    let src = r#"
(define (run) :: -> str (let ((xs (array "1" "2" "3"))) (let ((a (filter (lambda (x) (u128/gt x "1")) xs))) (str (to-string (vec-length a))))))
(export "run" run #f)
"#;
    let out = run_method("hof", src, "run", "{}").expect("run");
    assert!(out.contains("2"), "STALE HOF PARAM: got: {out}");
}
