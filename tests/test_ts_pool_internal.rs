//! v3 internal balances — NEP-611 gas-key trading (2026-09-28).
//!
//! A scoped gas key can NEVER attach a deposit (nearcore
//! verify_function_call_permission rejects deposit > 0 for any restricted
//! key — source-verified). Gas-key trading therefore runs on contract-side
//! ledgers in prod pool.ts:
//!   deposit()        — fund the global NEAR pad (nb:<trader>)
//!   buy() attach=0   — debit pad + near_in, credit internal token ledger
//!   sell_internal()  — ledger tokens → xyk NEAR → credited to the pad
//!   withdraw()       — 1-yoctoNEAR gate (restricted keys can't attach it)
//!
//! Compiles the PROD pair (projects/launchpad/pool.ts + token.ts) — first
//! suite that does — and exercises the classic path as a regression guard.
//! `--signer` stands in for key types: gaskey.test.near always sends
//! attach=0 (what a gas key is forced to do); no NEP-611 enforcement exists
//! in near-mock, so the protocol-level proof lives in /tmp/gaskey-probe
//! (live testnet E2E, 2026-09-28).

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
    let p = std::env::temp_dir().join(format!("ti_{name}_{}.wasm", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

const POOL: &str = "pool.test.near";
const TOKEN: &str = "token.test.near";
const FACTORY: &str = "factory.test.near"; // calls new_ + seed_pool (pool owner)
const TRADER: &str = "trader.test.near"; // classic path (full key)
const GASKEY: &str = "gaskey.test.near"; // gas-key path (attach=0 always)

const ONE: u128 = 1_000_000_000_000_000_000_000_000; // 1 NEAR (1e24)
const TSUP: &str = "1000000000000000000000000000"; // 1e27
const SEED_TOKENS: &str = "500000000000000000000000000"; // 0.5e27

fn state() -> String {
    format!("/tmp/ti_state_{}.bin", std::process::id())
}

fn mock(
    manifest: &str,
    contract: &str,
    method: &str,
    args: &str,
    attach: Option<u128>,
    signer: &str,
) -> String {
    let mut cmd = Command::new("./target/release/near-mock");
    cmd.env("NEAR_MOCK_QUIET", "1")
        .arg("cross")
        .arg(state())
        .arg(manifest)
        .arg(contract)
        .arg(method)
        .arg(args)
        .arg("--signer")
        .arg(signer);
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
    assert!(!out.contains("PANIC:"), "{label}: trapped — {out}");
    assert!(
        !out.contains("AccountDoesNotExist"),
        "{label}: dead promise receipt — {out}"
    );
}

#[test]
fn pool_internal_balances_gaskey_trading() {
    let pool = write_wasm("pool", &lower_and_compile("projects/launchpad/pool.ts"));
    let token = write_wasm("token", &lower_and_compile("projects/launchpad/token.ts"));
    let manifest = format!("{POOL}={pool},{TOKEN}={token}");
    let _ = std::fs::remove_file(state());

    // ── setup: init both contracts, mint to TRADER, seed the pool ──
    let out = mock(
        &manifest, POOL, "new", "{}", None,
        FACTORY, // predecessor → pool owner; seed_pool's isOwner gate needs this
    );
    out_clean(&out, "pool new_");

    let out = mock(
        &manifest,
        TOKEN,
        "new",
        &format!(
            r#"{{"owner_id":"{TRADER}","total_supply":"{TSUP}","name":"Internal Test","symbol":"INT","icon":"","decimals":"24"}}"#
        ),
        None,
        FACTORY,
    );
    out_clean(&out, "token new_");

    // seed_pool FIRST (ft_on_transfer aborts ERR_NO_POOL on an unseeded
    // pool — the real launchpad chains seed_pool → ft_transfer_call too)
    let out = mock(
        &manifest,
        POOL,
        "seed_pool",
        r#"{"token":"token.test.near","grad_th":"0"}"#,
        Some(ONE),
        FACTORY,
    );
    out_clean(&out, "seed_pool");

    // then TRADER pushes the token reserve in: ft_transfer_call msg="seed"
    let out = mock(
        &manifest,
        TOKEN,
        "ft_transfer_call",
        &format!(r#"{{"receiver_id":"{POOL}","amount":"{SEED_TOKENS}","msg":"seed"}}"#),
        Some(1),
        TRADER,
    );
    out_clean(&out, "seed ft_transfer_call");

    // ── quote views: bigDiv parse seam probe (CLI-built wasms trapped
    // here on some layouts — see GAPS note; this pins the cargo path) ──
    let out = mock(
        &manifest,
        POOL,
        "quote_buy",
        r#"{"token":"token.test.near","near_in":"1000000000000000000000000"}"#,
        None,
        FACTORY,
    );
    out_clean(&out, "quote_buy");
    // pt=5e26, pn=1e24, in=1e24 -> gross 2.5e26, fee 1%, net 2.475e26
    assert!(
        out.contains("247500000000000000000000000"),
        "quote_buy: {out}"
    );
    let out = mock(
        &manifest,
        POOL,
        "quote_sell",
        r#"{"token":"token.test.near","tokens_in":"100000000000000000000000000"}"#,
        None,
        FACTORY,
    );
    out_clean(&out, "quote_sell");
    // pt=5e26, pn=1e24, in=1e26 -> gross 1.666..e23, fee 1%, net 1.65e23
    assert!(
        out.contains("165000000000000000000000"),
        "quote_sell: {out}"
    );

    // ── classic path regression guard: attach-buy still works ──
    let out = mock(
        &manifest,
        POOL,
        "buy",
        r#"{"token":"token.test.near"}"#,
        Some(ONE),
        TRADER,
    );
    out_clean(&out, "classic buy");
    assert!(out.contains("✅ Success"), "classic buy: {out}");
    // TRADER: 1e27 - 0.5e27 (seed) + net(gross=2.5e26, fee 1%) = 0.7475e27
    let out = mock(
        &manifest,
        TOKEN,
        "ftBalanceOf",
        &format!(r#"{{"account_id":"{TRADER}"}}"#),
        None,
        TRADER,
    );
    out_clean(&out, "ft after classic buy");
    assert!(
        out.contains("747500000000000000000000000"),
        "classic buy net landed on FT ledger: {out}"
    );

    // ── v3: fund the gas-key trader's pad ──
    let out = mock(&manifest, POOL, "deposit", "{}", Some(2 * ONE), GASKEY);
    out_clean(&out, "deposit");
    let out = mock(
        &manifest,
        POOL,
        "get_balance",
        &format!(r#"{{"account":"{GASKEY}"}}"#),
        None,
        GASKEY,
    );
    out_clean(&out, "get_balance after deposit");
    assert!(
        out.contains("2000000000000000000000000"),
        "pad = 2 NEAR after deposit: {out}"
    );

    // ── gas-key buy: attach = 0, near_in from the pad ──
    // pt=2.5e26, pn=2e24 (classic buy moved the curve), in=0.5e24
    // → gross = 2.5e26*5e23/2.5e24 = 5e25, fee 1% → net 4.95e25
    let out = mock(
        &manifest,
        POOL,
        "buy",
        r#"{"token":"token.test.near","near_in":"500000000000000000000000"}"#,
        None, // attach = 0 — the only thing a gas key CAN do
        GASKEY,
    );
    out_clean(&out, "gas buy");
    assert!(out.contains("✅ Success"), "gas buy: {out}");
    assert!(
        out.contains("49500000000000000000000000"),
        "gas buy net in return value: {out}"
    );
    let out = mock(
        &manifest,
        POOL,
        "get_balance",
        &format!(r#"{{"account":"{GASKEY}","token":"token.test.near"}}"#),
        None,
        GASKEY,
    );
    out_clean(&out, "get_balance after gas buy");
    assert!(
        out.contains("1500000000000000000000000"),
        "pad debited to 1.5 NEAR: {out}"
    );
    assert!(
        out.contains("49500000000000000000000000"),
        "internal token ledger credited: {out}"
    );

    // ── lane volley: 3 more pad buys (each call = one lane's tx) ──
    for _ in 0..3 {
        let out = mock(
            &manifest,
            POOL,
            "buy",
            r#"{"token":"token.test.near","near_in":"100000000000000000000000"}"#,
            None,
            GASKEY,
        );
        out_clean(&out, "lane buy");
        assert!(out.contains("✅ Success"), "lane buy: {out}");
    }

    // ── sell_internal: whole ledger back to NEAR in the pad ──
    let out = mock(
        &manifest,
        POOL,
        "get_balance",
        &format!(r#"{{"account":"{GASKEY}","token":"token.test.near"}}"#),
        None,
        GASKEY,
    );
    out_clean(&out, "get_balance before sell");
    // extract the ledger amount from the view. near-mock wraps the return
    // value escaped inside a result field (\"tokens\":\"<amt>\"), so
    // split on the escaped key and read up to the next backslash.
    let tokens = out
        .split("\\\"tokens\\\":\\\"")
        .nth(1)
        .and_then(|s| s.split('\\').next())
        .unwrap_or("")
        .to_string();
    assert!(
        !tokens.is_empty() && tokens != "0",
        "ledger before sell: {out}"
    );
    let out = mock(
        &manifest,
        POOL,
        "sell_internal",
        &format!(r#"{{"token":"token.test.near","tokens_in":"{tokens}"}}"#),
        None,
        GASKEY,
    );
    out_clean(&out, "sell_internal");
    assert!(out.contains("✅ Success"), "sell_internal: {out}");
    let out = mock(
        &manifest,
        POOL,
        "get_balance",
        &format!(r#"{{"account":"{GASKEY}","token":"token.test.near"}}"#),
        None,
        GASKEY,
    );
    out_clean(&out, "get_balance after sell");
    // near-mock prints the return value escaped: \"tokens\":\"0\"
    assert!(
        out.contains(r#"\"tokens\":\"0\""#),
        "ledger fully sold: {out}"
    );
    assert!(
        !out.contains(r#"\"near\":\"0\""#),
        "pad grew from the sell: {out}"
    );

    // ── withdraw: yocto-gate + drain + re-entry guard ──
    let out = mock(&manifest, POOL, "withdraw", "{}", None, GASKEY);
    assert!(out.contains("ERR_YOCTO"), "withdraw without yocto: {out}");
    let out = mock(&manifest, POOL, "withdraw", "{}", Some(1), GASKEY);
    out_clean(&out, "withdraw with yocto");
    assert!(out.contains("✅ Success"), "withdraw: {out}");
    let out = mock(
        &manifest,
        POOL,
        "get_balance",
        &format!(r#"{{"account":"{GASKEY}"}}"#),
        None,
        GASKEY,
    );
    assert!(
        out.contains(r#"\"near\":\"0\""#),
        "pad drained by withdraw: {out}"
    );
    let out = mock(&manifest, POOL, "withdraw", "{}", Some(1), GASKEY);
    assert!(out.contains("ERR_EMPTY_PAD"), "re-withdraw: {out}");
}
