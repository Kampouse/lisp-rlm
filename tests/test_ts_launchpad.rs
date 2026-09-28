use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::process::Command;

fn lower_and_compile(path: &str) -> Vec<u8> {
    let src = std::fs::read_to_string(path).unwrap();
    let ir = ts_to_lisp_source(&src).unwrap_or_else(|e| panic!("lowering {path}: {e}"));
    let exprs = parse_all(&ir).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .unwrap_or_else(|e| panic!("typecheck {path}: {e}"));
    compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile {path}: {e}"))
}

fn write_wasm(name: &str, bytes: &[u8]) -> String {
    let p = std::env::temp_dir().join(format!("lp_{name}_{}.wasm", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

const LAUNCHPAD: &str = "owner.test.near";
// Derived by launchpad.ts: tokenAccount() = "<symbol>.<currentAccountId>".
const TOKEN: &str = "JEM.owner.test.near";
// Hardcoded by launchpad.ts batch 2 / graduate as the Ref router.
const REF_ROUTER: &str = "ref-finance.testnet.near";
const ATTACH: u128 = 1_100_000_000_000_000_000_000_000;
// 0.4 NEAR — the pad retains 0.5 NEAR after a funded launch (0.5 to the
// token, 0.1 to Ref), so graduate must attach ≤ 0.4 NEAR to clear the
// outbound-transfer balance check.
const GRAD_NEAR: &str = "400000000000000000000000";
const GRAD_ATTACH: u128 = 400_000_000_000_000_000_000_000;

fn mock(manifest: &str, contract: &str, method: &str, args: &str, attach: Option<u128>) -> String {
    let mut cmd = Command::new("./target/release/near-mock");
    cmd.env("NEAR_MOCK_QUIET", "1")
        .arg("cross")
        .arg(format!("/tmp/lp_state_{}.bin", std::process::id()))
        .arg(manifest)
        .arg(contract)
        .arg(method)
        .arg(args);
    if let Some(a) = attach {
        cmd.arg("--attach").arg(a.to_string());
    }
    let out = cmd.output().expect("near-mock run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn out_clean(out: &str, label: &str) {
    assert!(!out.contains("panicked"), "{label}: {out}");
    assert!(
        !out.contains("AccountDoesNotExist"),
        "{label}: dead promise receipt — {out}"
    );
}

#[test]
fn launchpad_launch_then_graduate() {
    let pad = write_wasm("pad", &lower_and_compile("fixtures/launchpad.ts"));
    let ft = write_wasm("ft", &lower_and_compile("fixtures/launch_ft.ts"));
    let rf = write_wasm("ref", &lower_and_compile("fixtures/launch_ref.ts"));
    let manifest = format!("{LAUNCHPAD}={pad},{TOKEN}={ft},{REF_ROUTER}={rf}");
    let _ = std::fs::remove_file(format!("/tmp/lp_state_{}.bin", std::process::id()));

    // 1. underfunded launch must abort
    let out = mock(
        &manifest,
        LAUNCHPAD,
        "launch_token",
        r#"{"creator":"creator.test.near","symbol":"JEM"}"#,
        Some(500_000_000_000_000_000_000_000),
    );
    assert!(out.contains("insufficient deposit"), "underfunded: {out}");

    // 2. funded launch fires both batches through the DAG — and the
    // receipts must actually land (parent commits even when a receipt
    // fails, so a dead Ref router would otherwise pass silently).
    let out = mock(
        &manifest,
        LAUNCHPAD,
        "launch_token",
        r#"{"creator":"creator.test.near","symbol":"JEM"}"#,
        Some(ATTACH),
    );
    out_clean(&out, "funded launch");
    // Quiet mode suppresses LOG lines; success marker + downstream state
    // (steps 3–4) carry the proof.
    assert!(out.contains("✅ Success"), "funded launch: {out}");

    // 3. spawned token really minted its whole supply to the launchpad
    let out = mock(
        &manifest,
        TOKEN,
        "ft_balance_of",
        r#"{"account_id":"owner.test.near"}"#,
        None,
    );
    out_clean(&out, "ft_balance_of");
    assert!(
        out.contains("1000000000000000000000000000"),
        "mint landed: {out}"
    );

    // 4. Ref pool really created: pool 1 = JEM/wrap, fee 30
    let out = mock(&manifest, REF_ROUTER, "get_pool", r#"{"id":"1"}"#, None);
    out_clean(&out, "get_pool");
    assert!(out.contains("JEM.owner.test.near"), "pool token A: {out}");
    assert!(out.contains("wrap.testnet.near"), "pool token B: {out}");
    assert!(out.contains("30"), "pool fee: {out}");

    // 5. graduate 0.4 NEAR into the pool: stub Ref records the deposit
    let out = mock(
        &manifest,
        LAUNCHPAD,
        "graduate",
        r#"{"token":"JEM.owner.test.near","nearAmount":"400000000000000000000000"}"#,
        Some(GRAD_ATTACH),
    );
    out_clean(&out, "graduate");
    // Quiet mode suppresses LOG lines; the final receipt's return value
    // ("deposited" from stub-Ref) + get_deposit below carry the proof.
    assert!(out.contains("deposited"), "graduate: {out}");
    let out = mock(
        &manifest,
        REF_ROUTER,
        "get_deposit",
        r#"{"account_id":"owner.test.near","token_id":"wrap.testnet.near"}"#,
        None,
    );
    out_clean(&out, "get_deposit");
    assert!(out.contains(GRAD_NEAR), "reserve landed on Ref: {out}");

    // 6. double-graduate aborts
    let out = mock(
        &manifest,
        LAUNCHPAD,
        "graduate",
        r#"{"token":"JEM.owner.test.near","nearAmount":"400000000000000000000000"}"#,
        Some(GRAD_ATTACH),
    );
    assert!(out.contains("already graduated"), "double graduate: {out}");
}
