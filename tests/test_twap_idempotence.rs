//! TWAP — receipt idempotence probe (red-on-bad / green-on-good).
//!
//! Fence: on_slice only settles when the callback's k equals the stored
//! cursor nx (the cursor advances on settle, so each slice settles
//! exactly once — no pending-key storage needed). This file pins BOTH
//! directions, against BOTH compiles of the contract (lisp original and
//! TS twin — the fence can't drift between arms):
//!   red-on-bad — replaying a STALE receipt (k behind the cursor) must
//!   NOT move money: fo/nx unchanged, SLICE_STALE logged.
//!   green-on-good — the live k==nx receipt settles (fo accrues, nx
//!   advances).

use std::process::Command;

fn has_near_mock() -> bool {
    Command::new("./target/release/near-mock")
        .args(["--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn compile_one(src: &str) -> Vec<u8> {
    let exprs = lisp_rlm_wasm::parse_all(src).expect("parse");
    lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("compile")
}

fn compile_twap_ts() -> Vec<u8> {
    let src = std::fs::read_to_string("examples/twap.ts").unwrap();
    let ir = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src).expect("ts lowering");
    let exprs = lisp_rlm_wasm::parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    lisp_rlm_wasm::compile_near_from_exprs(&exprs).expect("compile")
}

// pool with a per-(order,k) debit counter and 2x-of-debit-count pricing:
// swap(1,0) returns 4000 the first time, 8000 the second — so any
// double-debit would visibly change the money math.
const POOL: &str = r#"(define (sget k d) (default (near/storage_get k) d))
(define (swap)
  (let ((order (near/json_get_str "order")))
    (let ((k (near/json_get_str "k")))
      (let ((key (str-cat "p:" (str-cat order (str-cat ":" k)))))
        (let ((times (sget key "0")))
          (let ((n (u128/add times "1")))
            (near/storage_set key n)
            (near/return (u128/mul n "4000"))))))))
(export "swap" swap)
"#;

/// One block-height pass over the shared state. Returns (exit_ok, output).
fn run_scenario(dir: &std::path::Path, name: &str, height: u64, steps: &str) -> (bool, String) {
    let manifest = format!(
        "twap.c.test.near={},pool.c.test.near={}",
        dir.join("twap.wasm").display(),
        dir.join("pool.wasm").display()
    );
    let scenario = dir.join(format!("sc_{name}.json"));
    std::fs::write(
        &scenario,
        format!(
            r#"{{"name": "{name}", "manifest": "{manifest}", "steps": {steps}}}"#
        ),
    )
    .unwrap();
    let out = Command::new("./target/release/near-mock")
        .args(["scenario"])
        .arg(&scenario)
        .env("NEAR_MOCK_BLOCK_HEIGHT", height.to_string())
        .output()
        .expect("near-mock should run");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn setup(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("twap_idem_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("twap.wasm"), compile_one(&std::fs::read_to_string("examples/twap.lisp").unwrap())).unwrap();
    std::fs::write(dir.join("pool.wasm"), compile_one(POOL)).unwrap();
    std::fs::write(dir.join("state.bin"), b"").unwrap();
    dir
}

/// Full idempotence story for one contract arm. Heights: create@1000,
/// slice0 window [1005,1055] (start 1005, grace 50).
fn run_arm(dir: &std::path::Path, tag: &str) {
    // create: 2 slices x 1000, attach 2000
    let (ok, out) = run_scenario(
        dir,
        &format!("{tag}_create"),
        1000,
        r#"[
    {"method": "create", "attach": "2000",
     "args": {"pool": "pool.c.test.near", "token_out": "tok.near",
              "recipient": "seller.test.near", "n_slices": "2",
              "interval_h": "100", "start_h": "1005", "grace_h": "50",
              "min_out_per_slice": "0", "min_avg_out": "0"}}
  ]"#,
    );
    assert!(ok, "[{tag}] create failed:\n{out}");

    // tick at 1010 → slice 0 receipt settles: pool debits (1,0) once →
    // out 4000 → fo 3940, nx 0→1.
    let (ok, out) = run_scenario(
        dir,
        &format!("{tag}_tick0"),
        1010,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=3940", "contains": "o:1:nx=1"}
  ]"#,
    );
    assert!(ok, "[{tag}] tick0 failed:\n{out}");

    // THE PROBE — a stale receipt: bare on_slice with k=0 while nx=1.
    // This is the receipt-level attack surface. (In-mock receipts settle
    // between steps, so a same-window double-tick can't fire two
    // promises here; on mainnet the second tick's receipt arrives with
    // the cursor already advanced — i.e. exactly this stale shape.) The
    // fence must TRAP with ERR_STALE, leaving every money key untouched.
    let (ok, out) = run_scenario(
        dir,
        &format!("{tag}_stale"),
        1011,
        r#"[
    {"method": "on_slice",
     "args": {"order": "1", "k": "0", "amt": "4000"}, "expect": "trap"},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=3940", "contains": "o:1:nx=1"}
  ]"#,
    );
    assert!(ok, "[{tag}] stale-receipt scenario broke:\n{out}");
    assert!(
        out.contains("ERR_STALE"),
        "[{tag}] stale receipt NOT fenced (no ERR_STALE trap):\n{out}"
    );
    assert_eq!(
        out.matches("filled:0").count(),
        0,
        "[{tag}] stale receipt SETTLED (filled:0 logged in stale pass):\n{out}"
    );

    // green-on-good: the live slice still settles normally afterwards —
    // tick slice 1 at 1110 → pool (1,1) debited once → out 4000 → fo 7880.
    let (ok, out) = run_scenario(
        dir,
        &format!("{tag}_tick1"),
        1110,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=7880", "contains": "o:1:nx=2"}
  ]"#,
    );
    assert!(ok, "[{tag}] tick1 failed:\n{out}");
}

#[test]
fn twap_receipt_replay_is_fenced_both_arms() {
    if !has_near_mock() {
        return;
    }
    // ── lisp arm ──
    let dir = setup("lisp");
    run_arm(&dir, "lisp");
    let _ = std::fs::remove_dir_all(&dir);

    // ── TS arm — fresh dir so state/ids restart clean ──
    let dir = setup("ts");
    std::fs::write(dir.join("twap.wasm"), compile_twap_ts()).unwrap();
    run_arm(&dir, "ts");
    let _ = std::fs::remove_dir_all(&dir);
}
