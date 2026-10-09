//! TWAP execution contract (examples/twap.lisp) — near-mock scenario e2e.
//!
//! Pins the lazy-cursor scheduler end to end:
//!   create (escrow) → tick before due (ERR_NOT_DUE) → due tick queues the
//!   swap → on_slice receipt credits (fee split, cursor advance) → next
//!   slices → expired slices accrue refund → finalize verdicts.
//!
//! Multi-contract: twap.c.test.near + a tiny 2x mock pool. Scheduling is
//! driven by `near/block_height`, which the mock pins via
//! NEAR_MOCK_BLOCK_HEIGHT per invocation — so the scenario runs as
//! several passes (one per block height) over ONE persistent state file
//! (persisted at the end of every scenario run, reloaded by the next).

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

/// One block-height pass over the shared state. Returns (exit_ok, output).
fn pass(dir: &std::path::Path, name: &str, height: u64, steps: &str) -> (bool, String) {
    let manifest = format!(
        "twap.c.test.near={},pool.c.test.near={}",
        dir.join("twap.wasm").display(),
        dir.join("pool.wasm").display()
    );
    let scenario = dir.join(format!("sc_{name}.json"));
    std::fs::write(
        &scenario,
        format!(
            r#"{{"name": "{}", "manifest": "{}", "steps": {}}}"#,
            name,
            manifest.replace('\\', "/"),
            steps
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
    let dir = std::env::temp_dir().join(format!("twap_{}_{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let twap = std::fs::read_to_string("examples/twap.lisp").unwrap();
    std::fs::write(dir.join("twap.wasm"), compile_one(&twap)).unwrap();
    // 2x mock pool: returns out = 2 * amt as a bare string.
    std::fs::write(
        dir.join("pool.wasm"),
        // Fixed 2x-of-slice output — the swap call args carry only
        // {order,k} (amt rides the CALLBACK args), so the pool can't see
        // the amount; a real AMM would price from its own reserves.
        compile_one(
            r#"(define (swap) (near/return "4000"))
(export "swap" swap)
"#,
        ),
    )
    .unwrap();
    // one shared state file per test (name-keyed, pid-tagged dir)
    std::fs::write(dir.join("state.bin"), b"").unwrap();
    dir
}

#[test]
fn twap_lifecycle_fill_then_finalize() {
    if !has_near_mock() {
        return;
    }
    let dir = setup("life");
    let twap = r#""twap.c.test.near""#;

    // H=1000: create — attach 6000, 3 slices x 2000, interval 100,
    // start 1005, grace 50. Pool swaps at 2x → per filled slice:
    // out 4000, fee 40 (1%), cut 20 (0.5%), net 3940.
    let (ok, out) = pass(
        &dir,
        "create",
        1000,
        r#"[
    {"method": "create", "attach": "6000",
     "args": {"pool": "pool.c.test.near", "token_out": "tok.near",
              "recipient": "seller.test.near", "n_slices": "3",
              "interval_h": "100", "start_h": "1005", "grace_h": "50",
              "min_out_per_slice": "0", "min_avg_out": "0"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:st=open"}
  ]"#,
    );
    assert!(ok, "create pass failed:\n{out}");

    // H=1002: tick before due → ERR_NOT_DUE (scenario exits 1).
    let (ok, out) = pass(
        &dir,
        "early",
        1002,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}}
  ]"#,
    );
    assert!(!ok, "tick before due must abort:\n{out}");
    assert!(out.contains("ERR_NOT_DUE"), "early tick:\n{out}");

    // H=1010: slice 0 due → queued swap + on_slice credit.
    let (ok, out) = pass(
        &dir,
        "slice0",
        1010,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=3940", "contains": "o:1:nx=1"}
  ]"#,
    );
    assert!(ok, "slice0 pass failed:\n{out}");

    // H=1110: slice 1 → fo=7880.
    let (ok, out) = pass(
        &dir,
        "slice1",
        1110,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=7880"}
  ]"#,
    );
    assert!(ok, "slice1 pass failed:\n{out}");

    // H=1160: slice 2 due at 1205 — not yet.
    let (ok, out) = pass(
        &dir,
        "notdue2",
        1160,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}}
  ]"#,
    );
    assert!(!ok, "slice 2 must not be due at 1160:\n{out}");
    assert!(out.contains("ERR_NOT_DUE"), "slice2 early:\n{out}");

    // H=1210: slice 2 → fo=11820, all 3 consumed.
    let (ok, out) = pass(
        &dir,
        "slice2",
        1210,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:fo=11820", "contains": "o:1:nx=3"}
  ]"#,
    );
    assert!(ok, "slice2 pass failed:\n{out}");

    // H=1211: all slices consumed → finalize → filled_ok.
    let (ok, out) = pass(
        &dir,
        "fin",
        1211,
        r#"[
    {"method": "finalize", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:st=filled_ok"}
  ]"#,
    );
    assert!(ok, "finalize pass failed:\n{out}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = twap;
}

#[test]
fn twap_expired_slice_refunds_and_skips() {
    if !has_near_mock() {
        return;
    }
    let dir = setup("exp");

    // 2 slices x 1000 (attach 2000), start 1005, grace 50.
    let (ok, out) = pass(
        &dir,
        "create",
        1000,
        r#"[
    {"method": "create", "attach": "2000",
     "args": {"pool": "pool.c.test.near", "token_out": "tok.near",
              "recipient": "seller.test.near", "n_slices": "2",
              "interval_h": "100", "start_h": "1005", "grace_h": "50",
              "min_out_per_slice": "0", "min_avg_out": "0"}}
  ]"#,
    );
    assert!(ok, "create failed:\n{out}");

    // H=1200: slice 0 (due 1005) expired at 1055 < 1200 → refund accrues.
    let (ok, out) = pass(
        &dir,
        "exp0",
        1200,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:rx=1000", "contains": "o:1:xp=1"}
  ]"#,
    );
    assert!(ok, "expire0 failed:\n{out}");

    // H=1210: slice 1 (due 1105) also expired → rx=2000, nx=2=xp.
    let (ok, out) = pass(
        &dir,
        "exp1",
        1210,
        r#"[
    {"method": "tick", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:rx=2000", "contains": "o:1:xp=2"}
  ]"#,
    );
    assert!(ok, "expire1 failed:\n{out}");

    // Finalize: filled=0 → avg_skip verdict.
    let (ok, out) = pass(
        &dir,
        "fin",
        1211,
        r#"[
    {"method": "finalize", "args": {"order_id": "1"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:st=avg_skip"}
  ]"#,
    );
    assert!(ok, "finalize failed:\n{out}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn twap_cancel_before_start_refunds() {
    if !has_near_mock() {
        return;
    }
    let dir = setup("cxl");

    let (ok, out) = pass(
        &dir,
        "create",
        1000,
        r#"[
    {"method": "create", "attach": "4000", "as": "seller.test.near",
     "args": {"pool": "pool.c.test.near", "token_out": "tok.near",
              "recipient": "seller.test.near", "n_slices": "4",
              "interval_h": "100", "start_h": "2000", "grace_h": "50",
              "min_out_per_slice": "0", "min_avg_out": "0"}},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:seller=seller.test.near"}
  ]"#,
    );
    assert!(ok, "create failed:\n{out}");

    // H=1005 < start 2000, nx=0. Non-seller cancel → ERR_PERM trap.
    let (ok, out) = pass(
        &dir,
        "perm",
        1005,
        r#"[
    {"method": "cancel", "args": {"order_id": "1"}, "expect": "trap"}
  ]"#,
    );
    // "expect":"trap" → scenario exits 0 (trap was the expected outcome).
    assert!(ok, "perm pass should succeed with expected trap:\n{out}");
    assert!(out.contains("ERR_PERM"), "perm error:\n{out}");

    // Seller cancels → refund 4000, st=cancelled.
    let (ok, out) = pass(
        &dir,
        "cxl",
        1005,
        r#"[
    {"method": "cancel", "args": {"order_id": "1"}, "as": "seller.test.near"},
    {"method": "get_order", "args": {"order_id": "1"}, "view": true,
     "expect": "o:1:st=cancelled"}
  ]"#,
    );
    assert!(ok, "cancel failed:\n{out}");

    let _ = std::fs::remove_dir_all(&dir);
}
