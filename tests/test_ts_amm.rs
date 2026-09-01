//! AMM — constant product x·y=k, LP shares, 0.3% swap fee (protocol #14).
//!   1. init (owner) mints internal wallets: alice 2000/2000, bob 500/500
//!   2. alice adds 1000+1000 → first LP anchors ts=1000 (min)
//!   3. bob adds 500+500 (exact ratio) → ts=1500, proportional 500
//!   4. alice swaps 200 A → 176 B out (fee-adjusted constant product);
//!      k rises 2,250,000 → 2,250,800 (never decreases)
//!   5. bob removes 500/1500 shares → 566 A + 441 B (floor)
//!   6. CONSERVATION: A 1134+800+566 = 2500, B 883+1176+441 = 2500 —
//!      every minted token is in the pool or a wallet, always
//!   7. guards: zero swap / over-withdraw / ratio-breaking add all abort
//! Type discipline exercised: u128 ops are STRING-typed at runtime;
//! mixed member⊕toStr-local arithmetic must route through u128Add/Sub
//! (generic +/- is i64-only and the checker rejects str operands).

use std::sync::{Mutex, OnceLock};
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{parse_all, compile_near_from_exprs};

const AMM: &str = include_str!("../fixtures/amm.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

struct Call<'a> { acct: &'a str, method: &'a str, args: &'a str, signer: &'a str, view: bool }

fn run(state: &str, c: Call) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(AMM).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("amm_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let manifest = format!("pool.p.test.near={}", p.display());
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross").arg(state).arg(&manifest).arg(c.acct).arg(c.method).arg(c.args)
        .env("NEAR_MOCK_SIGNER", c.signer)
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000")
        .env("NEAR_MOCK_BLOCK_HEIGHT", "100")
        .env("NEAR_MOCK_ATTACH", "0");
    if c.view { cmd.arg("--view"); }
    let out = cmd.output().expect("near-mock");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn amm_constant_product_full_suite() {
    let st = "/tmp/amm-t.bin";
    let _ = std::fs::remove_file(st);
    let A = "pool.p.test.near";
    let call = |m: &str, a: &str, s: &str, v: bool| run(st, Call { acct: A, method: m, args: a, signer: s, view: v });

    // 1. init
    let i = call("init", "{}", "owner.test.near", false);
    assert!(i.contains("minted 2500/2500"), "init: {i}");
    // init is one-shot
    let i2 = call("init", "{}", "owner.test.near", false);
    assert!(i2.contains("already initialized"), "re-init: {i2}");

    // 2. first LP anchors shares = min(1000, 1000)
    let a1 = call("addLiquidity", r#"{"amtA":"1000","amtB":"1000"}"#, "alice.test.near", false);
    assert!(a1.contains(r#""ra":"1000""#) && a1.contains(r#""ts":"1000""#), "alice first LP: {a1}");

    // 3. bob's proportional add: 500/500 at 1:1 → 500 shares
    let b1 = call("addLiquidity", r#"{"amtA":"500","amtB":"500"}"#, "bob.test.near", false);
    assert!(b1.contains(r#""ts":"1500""#), "bob proportional: {b1}");
    let lp = call("lpOf", r#"{"who":"bob.test.near"}"#, "anyone.test.near", true);
    assert!(lp.contains("📄 500"), "bob lp: {lp}");

    // 4. swap 200 A → 176 B; k strictly increases
    let k0 = call("k", "{}", "anyone.test.near", true);
    assert!(k0.contains("📄 2250000"), "k0: {k0}");
    let s1 = call("swapExactIn", r#"{"dir":"a","amtIn":"200"}"#, "alice.test.near", false);
    assert!(s1.contains(r#""ra":"1700""#) && s1.contains(r#""rb":"1324""#), "swap moved price: {s1}");
    let k1 = call("k", "{}", "anyone.test.near", true);
    assert!(k1.contains("📄 2250800"), "k rose: {k1}");
    let wa = call("walletOf", r#"{"who":"alice.test.near"}"#, "anyone.test.near", true);
    assert!(wa.contains(r#""a":"800""#) && wa.contains(r#""b":"1176""#), "alice wallet: {wa}");

    // 5. bob removes 500/1500 → floor(1700*500/1500)=566, floor(1324*500/1500)=441
    let r1 = call("removeLiquidity", r#"{"shares":"500"}"#, "bob.test.near", false);
    assert!(r1.contains(r#""ra":"1134""#) && r1.contains(r#""rb":"883""#) && r1.contains(r#""ts":"1000""#), "bob exit: {r1}");
    let wb = call("walletOf", r#"{"who":"bob.test.near"}"#, "anyone.test.near", true);
    assert!(wb.contains(r#""a":"566""#) && wb.contains(r#""b":"441""#), "bob wallet: {wb}");

    // 6. conservation: pool + wallets == total minted (2500 each side)
    let fin = call("reserves", "{}", "anyone.test.near", true);
    assert!(fin.contains(r#""ra":"1134""#) && fin.contains(r#""rb":"883""#), "final pool: {fin}");
    // 1134 + 800 + 566 = 2500 A; 883 + 1176 + 441 = 2500 B — machine-checked
    let sum_a = 1134 + 800 + 566;
    let sum_b = 883 + 1176 + 441;
    assert_eq!((sum_a, sum_b), (2500, 2500), "token conservation");

    // 7. guards
    let g1 = call("swapExactIn", r#"{"dir":"a","amtIn":"0"}"#, "alice.test.near", false);
    assert!(g1.contains("zero swap"), "zero: {g1}");
    let g2 = call("removeLiquidity", r#"{"shares":"999"}"#, "bob.test.near", false);
    assert!(g2.contains("over-withdraw"), "over: {g2}");
    let g3 = call("addLiquidity", r#"{"amtA":"100","amtB":"200"}"#, "alice.test.near", false);
    assert!(g3.contains("ratio"), "ratio: {g3}");
    // bob's remaining LP: 0 after full exit
    let lp2 = call("lpOf", r#"{"who":"bob.test.near"}"#, "anyone.test.near", true);
    assert!(lp2.contains("📄 0"), "bob lp drained: {lp2}");
}
