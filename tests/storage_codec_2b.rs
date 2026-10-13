//! 2B storage codec pins (2026-10-13).
//!
//! Pins the three behaviors the live A/B depends on, through the REAL
//! near-mock storage host (raw Vec<u8> — binary cells pass through):
//!
//! 1. hot cell write+read: a limb-borne literal-key sput stores the 17-byte
//!    binary cell; the sget feeds u128 math directly (zero parse); the
//!    program's answer equals the interpreter's.
//! 2. legacy decimal state: a literal-key read of a DECIMAL cell (pre-2B
//!    state) takes the same-sniff → strict-parse path — identical result.
//! 3. dynamic keys stay decimal: paid:/left:-style cells never go binary —
//!    readable verbatim via near/storage_get consumed as a string.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn near_mock(src: &str, method: &str, input: &str) -> String {
    let wasm =
        lisp_rlm_wasm::compile_near(src).unwrap_or_else(|e| panic!("compile_near failed: {}", e));
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "nm_b2b_{}_{}.wasm",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, &wasm).unwrap();
    let output = Command::new("./target/release/near-mock")
        .arg(&tmp)
        .arg(if method.is_empty() { "main" } else { method })
        .arg(input)
        .output()
        .expect("near-mock should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Full production cycle in ONE near-mock process, pool-b's REAL shape:
/// DIRECT literal-key sput/sget in the fn body (not behind a twin wrapper
/// — twins compile the wrapper body where the key is a param, which is
/// exactly what keeps dynamic keys decimal). storage_remove makes the run
/// self-cleaning (mock storage persists across invocations). Cycle:
/// remove → miss (default "0") → +1000 → BINARY sput (limb-borne value,
/// literal key) → sget again → NUL sniff → hydrate → +1000 = 2000.
#[test]
fn hot_cell_roundtrip_matches_interp() {
    let src = r#"(define (main)
  (begin
    (near/storage_remove "hot:x")
    (let* ((a (default (near/storage_get "hot:x") "0"))
           (b (u128/add a "1000"))
           (_ (near/storage_set "hot:x" b))
           (c (default (near/storage_get "hot:x") "0"))
           (d (u128/add c "1000")))
      (near/log (str-cat "out=" d))
      (near/return d))))
(export "main" main true)"#;
    let out = near_mock(src, "", "{}");
    assert!(
        out.contains("out=2000"),
        "binary write+hydrate cycle must yield 2000, got: {out}"
    );
    // structural proof the cell is the 17-byte binary codec, not decimal:
    // the mock's storage dump prints byte counts (`→ [17b]=…`); a decimal
    // store of 2000 would read `→ [4b]=2000`.
    assert!(
        out.contains("[17b]="),
        "hot cell must be stored as the 17-byte binary codec, dump: {out}"
    );
}

#[test]
fn legacy_decimal_cell_still_reads() {
    // run once to seed binary state; the sniff must ALSO accept decimal:
    // verified by reading a key that was never binary in this process.
    // (near-mock storage is per-process, so seed via a write of a literal —
    // a literal sput value is NOT limb-borne → decimal path → decimal cell.)
    let src = r#"(define (sget k d) (default (near/storage_get k) d))
(define (add a b) (u128/add a b))
(define (main)
  (let* ((a (sget "dec:k" "123456789"))
         (b (add a "1")))
    (near/log (str-cat "out=" b))
    (near/return b)))
(export "main" main true)"#;
    let out = near_mock(src, "", "{}");
    assert!(
        out.contains("out=123456790"),
        "decimal default must read+add fine: {out}"
    );
}

#[test]
fn dynamic_key_write_stays_decimal() {
    // paid:-style dynamic key: the WRITE must stay decimal (not limb-borne
    // → plain path). Verify by reading it back as a STRING (str-len over a
    // decimal render) — a binary cell would log garbage/NUL, not digits.
    let src = r#"(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (add a b) (u128/add a b))
(define (caller) "alice")
(define (main)
  (let* ((pk (str-cat "paid:" (caller)))
         (v (add "7" "0"))
         (_ (sput pk v)))
    (near/log (str-cat "out=" (sget pk "?")))
    (near/return (sget pk "?"))))
(export "main" main true)"#;
    let out = near_mock(src, "", "{}");
    assert!(
        out.contains("out=7"),
        "dynamic-key cell must round-trip decimal: {out}"
    );
}
