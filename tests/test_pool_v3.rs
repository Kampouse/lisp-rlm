//! pool_v3 — launch gate / decaying flip tax / market-cap-ratio tax /
//! seniority pot / sell-gap fix. Dual coverage:
//!
//! - INTERP: the bytecode interpreter with the mock NEAR env — contract
//!   cores are called directly with explicit string args (json_get /
//!   attached-deposit are wasm-side env reads), promises recorded in
//!   EvalState.
//! - WASM: `near-compile build pool-v3` (+ pool-v3/token) → the
//!   near-mock cross engine with a real NEP-141-style mock token: full
//!   ft_transfer_call → ft_on_transfer → keep-string → refund chains.
//!
//! All expected values computed with python3 (floor division), never by
//! hand. Scene scale is u128-safe: dep*Rt ≤ 5e35 < 2^128 ≈ 3.4e38.

use lisp_rlm_wasm::*;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

// ═══════════════════════════════════════════════════════════════════
// scene constants (python3-verified)
// ═══════════════════════════════════════════════════════════════════

const T0: &str = "1700000000000000000"; // launch ts
const TG: &str = "1700000300000000000"; // gate opens (T0 + 300s)
const T60: &str = "1700000360000000000"; // TG + 60s (flip = 4500 bp)
const TB: &str = "1700001500000000000"; // TG + 1200s (bob buys)
const TS2: &str = "1700002100000000000"; // TB + 600s (flip 0 for bob)

const RN0: &str = "500000000000000000"; // initial NEAR reserve (5e17)
const SEED: &str = "1000000000000000000"; // seed tokens (1e18)
const OUT_A: &str = "500000000000000000"; // alice buy out (symmetric pool)

const R1_N: &str = "1000000000000000000"; // after alice buy
const R1_T: &str = "500000000000000000";

const SELL_GROSS: &str = "500000000000000000"; // alice sells all at T60
const SELL_TAX: &str = "225000000000000000"; // 4500 bp
const SELL_PAYOUT: &str = "275000000000000000";
const POT1: &str = "112500000000000000"; // seniority pot after alice sell
const DEPTH1: &str = "112500000000000000"; // and the same to depth

const R2_N: &str = "725000000000000000"; // after alice sell
const R2_T: &str = "1000000000000000000";

const OUT_B: &str = "408163265306122448"; // bob's buy out at R2
const R3_N: &str = "1225000000000000000";
const R3_T: &str = "591836734693877552";
const WSUM: &str = "1816326"; // wA(1e6) + wB(816326)
const CLAIM_A: &str = "61938220341502571"; // pot1 * wA / wsum

const BOB_S3: &str = "1000000000000000"; // bob min-out sells
const BOB_G3: &str = "2066336190574546"; // gross (eff 0)
const BOB_M: &str = "10000000000000000"; // max_near cap
const BOB_TP: &str = "4871084236163602"; // tokens consumed (ceil)
const BOB_S2: &str = "4871084236164379"; // amount sent (Tp + 777)
const BOB_GA: &str = "10000000000000001"; // actual proceeds at cap
const BOB_AFTER: &str = "403292181069958846"; // OUT_B - Tp

const CAROL_SELL: &str = "400000000000000000"; // carol sells at T60
const CAROL_G: &str = "444444444444444444";
const CAROL_TAX: &str = "199999999999999999";
const CAROL_PAY: &str = "244444444444444445";

// storage keys (pinned — the layout IS the ABI)
const K_ES_FT: &str = "es:ft.tst";
const K_PN_FT: &str = "pn:ft.tst";
const K_PT_FT: &str = "pt:ft.tst";
const K_LB_FT_ALICE: &str = "lb:ft.tst|alice.tst";
const K_RB_FT_ALICE: &str = "rb:ft.tst|alice.tst";
const K_WT_FT_ALICE: &str = "wt:ft.tst|alice.tst";

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

fn contract() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/pool_v3.lisp");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read examples/pool_v3.lisp: {}", e))
}

// ═══════════════════════════════════════════════════════════════════
// INTERP DRIVER
// ═══════════════════════════════════════════════════════════════════

struct Interp {
    env: Env,
    state: EvalState,
}

impl Interp {
    fn new() -> Self {
        let mut env = Env::new();
        let mut state = EvalState::new();
        let _ = program::run_program(&parse_all(&contract()).unwrap(), &mut env, &mut state);
        Interp { env, state }
    }
    fn eval(&mut self, src: &str) -> Result<LispVal, String> {
        program::run_program(&parse_all(src).unwrap(), &mut self.env, &mut self.state)
    }
    fn s(&mut self, src: &str) -> String {
        match self.eval(src) {
            Ok(LispVal::Str(s)) => s,
            Ok(other) => panic!("expected Str from {}, got {:?}", src, other),
            Err(e) => panic!("interp failed: {}\n  {}", e, src),
        }
    }
    fn run(&mut self, src: &str) {
        if let Err(e) = self.eval(src) {
            panic!("interp failed: {}\n  {}", e, src);
        }
    }
    fn err(&mut self, src: &str) -> String {
        match self.eval(src) {
            Err(e) => e,
            Ok(v) => panic!("expected Err from {}, got {:?}", src, v),
        }
    }
    fn kv(&self, key: &str) -> String {
        match self.state.near_storage.get(key) {
            Some(LispVal::Str(s)) => s.clone(),
            other => panic!("no string at {}: {:?}", key, other),
        }
    }
    fn kv_opt(&self, key: &str) -> Option<String> {
        self.state
            .near_storage
            .get(key)
            .map(|v| v.to_string_repr_or_empty())
    }
    /// last recorded promise field (near_promises)
    fn last_promise(&self) -> &LispVal {
        self.state
            .near_promises
            .last()
            .expect("no promise recorded")
    }
    fn promise_str(m: &LispVal, k: &str) -> String {
        match m {
            LispVal::Map(h) => match h.get(k) {
                Some(LispVal::Str(s)) => s.clone(),
                other => panic!("promise field {} not str: {:?}", k, other),
            },
            other => panic!("promise not a map: {:?}", other),
        }
    }
    fn last_transfer_amount(&self) -> String {
        for p in self.state.near_promises.iter().rev() {
            if let LispVal::Map(h) = p {
                if h.get("type") == Some(&LispVal::Str("transfer".into())) {
                    return match h.get("amount") {
                        Some(LispVal::Str(s)) => s.clone(),
                        other => panic!("transfer amount not str: {:?}", other),
                    };
                }
            }
        }
        panic!("no transfer promise recorded");
    }
    fn last_transfer_target(&self) -> String {
        for p in self.state.near_promises.iter().rev() {
            if let LispVal::Map(h) = p {
                if h.get("type") == Some(&LispVal::Str("transfer".into())) {
                    return Self::promise_str(p, "target");
                }
            }
        }
        panic!("no transfer promise recorded");
    }
    fn batch_actions_len(&self) -> usize {
        self.state.near_batch_actions.len()
    }
    fn batch_action(&self, i: usize) -> &LispVal {
        &self.state.near_batch_actions[i]
    }
}

trait ValExt {
    fn to_string_repr_or_empty(&self) -> String;
}
impl ValExt for LispVal {
    fn to_string_repr_or_empty(&self) -> String {
        match self {
            LispVal::Str(s) => s.clone(),
            other => format!("{:?}", other),
        }
    }
}

// standard launcher flow: seed escrow → launch a seniority-ON pool at
// RN0/T0 with SEED tokens credited (5e17 near | 1e18 tokens)
fn launch_std(it: &mut Interp, tok: &str) {
    assert_eq!(
        it.s(&format!(
            "(sell-core \"{}\" \"jp.tst\" \"{}\" \"seed\" \"0\" \"0\" \"{}\")",
            tok, SEED, T0
        )),
        SEED
    );
    let r = it.s(&format!(
        "(launch-core \"{}\" \"1\" \"300000\" \"{}\" \"{}\")",
        tok, RN0, T0
    ));
    assert_eq!(r, "1");
    assert_eq!(it.kv(&format!("pt:{}", tok)), SEED);
}

// ═══════════════════════════════════════════════════════════════════
// INTERP TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn interp_quote_buy_exactness_and_rounding() {
    let _g = lock();
    let mut it = Interp::new();

    // launch_std runs the launcher flow: escrow → launch
    launch_std(&mut it, "ft.tst");
    // escrow credited to token reserve at launch
    assert_eq!(it.kv(K_PT_FT), SEED);
    assert_eq!(it.kv(K_PN_FT), RN0);

    // quote: 5e17 into (5e17 near | 1e18 tokens) → 5e17 exact
    assert_eq!(it.s("(calc-buy-out \"ft.tst\" \"500000000000000000\")"), OUT_A);
    // rounding-toward-pool: 1e17 → floor(5e34/1.1e18) = 45454545454545454
    assert_eq!(it.s("(calc-buy-out \"ft.tst\" \"100000000000000000\")"), "166666666666666666");
    // no pool → "0"
    assert_eq!(it.s("(calc-buy-out \"nope.tst\" \"100000000000000000\")"), "0");

    // buy at the gate boundary (ts == gate_until → allowed)
    let n_before = it.state.near_promises.len();
    assert_eq!(
        it.s(&format!(
            "(buy-core \"ft.tst\" \"alice.tst\" \"{}\" \"0\" \"{}\")",
            OUT_A, TG
        )),
        OUT_A
    );
    assert_eq!(it.kv(K_PN_FT), R1_N);
    assert_eq!(it.kv(K_PT_FT), R1_T);
    assert_eq!(it.kv(K_LB_FT_ALICE), TG);
    assert_eq!(it.kv(K_RB_FT_ALICE), R1_N);
    assert_eq!(it.kv(K_WT_FT_ALICE), "1000000"); // 1e24 / 1e18

    // the token transfer promise: near/call ft_transfer on the token account
    assert_eq!(it.state.near_promises.len(), n_before + 1);
    let act = it.last_promise().clone();
    assert_eq!(Interp::promise_str(&act, "type"), "call");
    assert_eq!(Interp::promise_str(&act, "target"), "ft.tst");
    assert_eq!(Interp::promise_str(&act, "method"), "ft_transfer");
    let args = Interp::promise_str(&act, "args");
    assert!(args.contains("\"receiver_id\":\"alice.tst\""), "args={}", args);
    assert!(args.contains(&format!("\"amount\":\"{}\"", OUT_A)), "args={}", args);

    // buy rounding pinned again through the state change
    // (1e17 more: out 45454545454545454)
    assert_eq!(
        it.s(&format!(
            "(buy-core \"ft.tst\" \"alice.tst\" \"100000000000000000\" \"0\" \"{}\")",
            TG
        )),
        "45454545454545454"
    );
}

#[test]
fn interp_gate() {
    let _g = lock();
    let mut it = Interp::new();
    launch_std(&mut it, "ft.tst");

    // one ns before the boundary reverts
    let e = it.err(&format!(
        "(buy-core \"ft.tst\" \"alice.tst\" \"{}\" \"0\" \"{}\")",
        OUT_A, T0
    ));
    assert!(e.contains("ERR_EARLY"), "{}", e);

    // exactly at the boundary passes (covered in the exactness test too)
    assert_eq!(
        it.s(&format!(
            "(buy-core \"ft.tst\" \"alice.tst\" \"{}\" \"0\" \"{}\")",
            OUT_A, TG
        )),
        OUT_A
    );

    // sells are NEVER gated — a sell right after launch succeeds
    let keep = it.s(&format!(
        "(sell-core \"ft.tst\" \"alice.tst\" \"{}\" \"\" \"0\" \"0\" \"{}\")",
        OUT_A, T0
    ));
    assert_eq!(keep, OUT_A);
}

#[test]
fn interp_flip_tax_brackets() {
    let _g = lock();
    let mut it = Interp::new();
    launch_std(&mut it, "ft.tst");
    assert_eq!(
        it.s(&format!(
            "(buy-core \"ft.tst\" \"alice.tst\" \"{}\" \"0\" \"{}\")",
            OUT_A, TG
        )),
        OUT_A
    );

    // pure brackets via eff-bp-of (mcap ratio 1 → 0, so eff == flip)
    // t=0 → 5000
    assert_eq!(it.s(&format!("(eff-bp-of \"ft.tst\" \"alice.tst\" \"{}\")", TG)), "5000");
    // t=300s → 2500
    assert_eq!(it.s("(eff-bp-of \"ft.tst\" \"alice.tst\" \"1700000600000000000\")"), "2500");
    // t=600s → 0 ; t=900s → 0
    assert_eq!(it.s("(eff-bp-of \"ft.tst\" \"alice.tst\" \"1700000900000000000\")"), "0");
    assert_eq!(it.s("(eff-bp-of \"ft.tst\" \"alice.tst\" \"1700001200000000000\")"), "0");
    // no buy record → MAX bracket
    assert_eq!(it.s(&format!("(eff-bp-of \"ft.tst\" \"nobody.tst\" \"{}\")", TG)), "5000");

    // t=60s full sell: 4500 bp — exact split pinned
    let keep = it.s(&format!(
        "(sell-core \"ft.tst\" \"alice.tst\" \"{}\" \"\" \"0\" \"0\" \"{}\")",
        OUT_A, T60
    ));
    assert_eq!(keep, OUT_A);
    assert_eq!(it.kv(K_PN_FT), R2_N);
    assert_eq!(it.kv(K_PT_FT), R2_T);
    assert_eq!(it.kv("po:ft.tst"), POT1);
    // payout NEAR promise to the seller
    assert_eq!(it.last_transfer_amount(), SELL_PAYOUT);
    assert_eq!(it.last_transfer_target(), "alice.tst");
    // Σ-invariant: payout + tax == gross; tax == pot + depth
    // (payout=275e15, tax=225e15, pot=112.5e15, depth=112.5e15 — constants)
    // reserve math: Rn_after = Rn_before - payout (tax stays in depth)
}

#[test]
fn interp_mcap_tiers() {
    let _g = lock();
    let mut it = Interp::new();
    launch_std(&mut it, "ft.tst");
    // pin reserve-at-buy directly (the mcap ratio input)
    it.run("(sput (k-ac \"rb:\" \"ft.tst\" \"bob.tst\") \"1000000000000000000\")");
    let cases = [
        ("1900000000000000000", "0"),    // 1.9x → 0
        ("2000000000000000000", "1000"), // 2x → 1000
        ("4900000000000000000", "1000"), // 4.9x → 1000
        ("5000000000000000000", "2500"), // 5x → 2500
        ("9900000000000000000", "2500"), // 9.9x → 2500
        ("10000000000000000000", "4000"),// 10x → 4000
        ("34000000000000000000", "4000"),// way past → 4000
    ];
    for (rn, want) in cases {
        it.run(&format!("(sput (k-pn \"ft.tst\") \"{}\")", rn));
        assert_eq!(it.s(&format!("(mcap-bp-of \"\" \"{}\")", rn)), "4000", "no-record default");
        assert_eq!(
            it.s(&format!("(mcap-bp-of \"1000000000000000000\" \"{}\")", rn)),
            want,
            "rn={}",
            rn
        );
    }
}

#[test]
fn interp_eff_is_max_composition() {
    let _g = lock();
    let mut it = Interp::new();
    launch_std(&mut it, "ft.tst");
    // bob bought at reserve 5e17, current 2.4e18 (4.8x → 1000); his last
    // buy was 60s ago (flip 4500) → eff = 4500
    it.run("(sput (k-ac \"lb:\" \"ft.tst\" \"bob.tst\") \"1700000300000000000\")");
    it.run("(sput (k-ac \"rb:\" \"ft.tst\" \"bob.tst\") \"500000000000000000\")");
    it.run("(sput (k-pn \"ft.tst\") \"2400000000000000000\")");
    assert_eq!(
        it.s(&format!("(eff-bp-of \"ft.tst\" \"bob.tst\" \"{}\")", T60)),
        "4500"
    );
    // now flip 0 (t=600s), mcap 4.8x → eff = 1000
    it.run(&format!("(sput (k-ac \"lb:\" \"ft.tst\" \"bob.tst\") \"{}\")", TB));
    assert_eq!(
        it.s(&format!("(eff-bp-of \"ft.tst\" \"bob.tst\" \"{}\")", TS2)),
        "1000"
    );
    // flip 0, mcap 12x → 4000
    it.run("(sput (k-pn \"ft.tst\") \"6000000000000000000\")");
    assert_eq!(
        it.s(&format!("(eff-bp-of \"ft.tst\" \"bob.tst\" \"{}\")", TS2)),
        "4000"
    );
}

#[test]
fn interp_seniority_weights_and_claims() {
    let _g = lock();
    let mut it = Interp::new();

    // scene C: Rn0=5e15, seed 1e18, seniority ON
    it.s("(sell-core \"ct.tst\" \"jp.tst\" \"1000000000000000000\" \"seed\" \"0\" \"0\" \"1700000000000000000\")");
    it.s("(launch-core \"ct.tst\" \"1\" \"300000\" \"5000000000000000\" \"1700000000000000000\")");
    // alice buys 5e15 at gate → reserve 1e16, wA = 1e24/1e16 = 1e8
    it.s("(buy-core \"ct.tst\" \"alice.tst\" \"5000000000000000\" \"0\" \"1700000300000000000\")");
    assert_eq!(it.kv("wt:ct.tst|alice.tst"), "100000000");
    // bob buys 5e15 → reserve 1.5e16, wB = 1e24/1.5e16 = 66666666
    it.s("(buy-core \"ct.tst\" \"bob.tst\" \"5000000000000000\" \"0\" \"1700000300000000000\")");
    assert_eq!(it.kv("wt:ct.tst|bob.tst"), "66666666");
    assert_eq!(it.kv("ws:ct.tst"), "166666666");
    // weight ratio ≈ reserve ratio (100000000/66666666 = 1.500000015 ≈ 1.5)

    // alice flips (sells her full out at t=0): eff 5000
    let keep = it.s("(sell-core \"ct.tst\" \"alice.tst\" \"500000000000000000\" \"\" \"0\" \"0\" \"1700000300000000000\")");
    assert_eq!(keep, "500000000000000000");
    assert_eq!(it.kv("po:ct.tst"), "2249999999999999");
    assert_eq!(it.last_transfer_amount(), "4500000000000000");

    // claim: shareA = pot * wA / wsum (exact)
    let sh = it.s("(claim-core \"ct.tst\" \"alice.tst\")");
    assert_eq!(sh, "1350000005399999");
    assert_eq!(it.kv("po:ct.tst"), "899999994600000");
    assert_eq!(it.kv("wt:ct.tst|alice.tst"), "0");
    assert_eq!(it.kv("ws:ct.tst"), "66666666");
    assert_eq!(it.last_transfer_amount(), "1350000005399999");
    assert_eq!(it.last_transfer_target(), "alice.tst");

    // second claim pays 0 (weight zeroed)
    assert_eq!(it.s("(claim-core \"ct.tst\" \"alice.tst\")"), "0");

    // bob claims the rest
    assert_eq!(it.s("(claim-core \"ct.tst\" \"bob.tst\")"), "899999994600000");
    assert_eq!(it.kv("po:ct.tst"), "0");
    assert_eq!(it.kv("ws:ct.tst"), "0");
}

#[test]
fn interp_sell_gap_fix() {
    let _g = lock();
    let mut it = Interp::new();
    launch_std(&mut it, "ft.tst");
    it.s(&format!(
        "(buy-core \"ft.tst\" \"alice.tst\" \"{}\" \"0\" \"{}\")",
        OUT_A, TG
    ));
    // alice exits fully at T60 (4500 bp) → main-scene R2 reserves
    it.s(&format!(
        "(sell-core \"ft.tst\" \"alice.tst\" \"{}\" \"\" \"0\" \"0\" \"{}\")",
        OUT_A, T60
    ));
    assert_eq!(it.kv(K_PN_FT), R2_N);
    assert_eq!(it.kv(K_PT_FT), R2_T);

    // ── dust sell at the R2 state (Rn 7.25e17 < Rt 1e18): 1 token →
    //     gross floor = 0 → keep "0", reserves untouched
    let keep = it.s("(sell-core \"ft.tst\" \"alice.tst\" \"1\" \"\" \"0\" \"0\" \"1700000360000000000\")");
    assert_eq!(keep, "0");
    assert_eq!(it.kv(K_PN_FT), R2_N, "dust must not move reserves");
    assert_eq!(it.kv(K_PT_FT), R2_T);

    it.s(&format!(
        "(buy-core \"ft.tst\" \"bob.tst\" \"{}\" \"0\" \"{}\")",
        OUT_A, TB
    ));
    assert_eq!(it.kv(K_PN_FT), R3_N);
    assert_eq!(it.kv(K_PT_FT), R3_T);

    // ── min-out revert: gross3 = 2066336190574546 (eff 0 for bob at TS2),
    //    demand one more → keep "0", NO state change, NO payout promise
    let promises_before = it.state.near_promises.len();
    let keep = it.s(&format!(
        "(sell-core \"ft.tst\" \"bob.tst\" \"{}\" \"\" \"{}\" \"0\" \"{}\")",
        BOB_S3, "2066336190574547", TS2
    ));
    assert_eq!(keep, "0");
    assert_eq!(it.kv(K_PN_FT), R3_N, "min-out revert must not move reserves");
    assert_eq!(it.kv(K_PT_FT), R3_T);
    assert_eq!(
        it.state.near_promises.len(),
        promises_before,
        "no payout on a reverted sell"
    );
    // ── partial-keep: max_near cap consumes only Tp of the sent amount
    //    (still at the R3 state — the reverted min-out changed nothing)
    let keep = it.s(&format!(
        "(sell-core \"ft.tst\" \"bob.tst\" \"{}\" \"\" \"0\" \"{}\" \"{}\")",
        BOB_S2, BOB_M, TS2
    ));
    assert_eq!(keep, BOB_TP, "only the consumed tokens are kept");
    assert_eq!(it.last_transfer_amount(), BOB_GA);
    // token reserve grew by exactly the keep
    let pt_expected: u128 = R3_T.parse::<u128>().unwrap()
        + BOB_TP.parse::<u128>().unwrap();
    assert_eq!(it.kv(K_PT_FT), pt_expected.to_string());

    // ── min_out EXACTLY met passes — at the post-cap state the same
    //    BOB_S3 sells for 2032765778729439 (eff 0), demand exactly that
    let keep = it.s(&format!(
        "(sell-core \"ft.tst\" \"bob.tst\" \"{}\" \"\" \"2032765778729439\" \"0\" \"{}\")",
        BOB_S3, TS2
    ));
    assert_eq!(keep, BOB_S3);
    assert_eq!(it.last_transfer_amount(), "2032765778729439");
}

#[test]
fn interp_error_paths_and_seed_paths() {
    let _g = lock();
    let mut it = Interp::new();

    // zero deposit
    let e = it.err("(buy-core \"ft.tst\" \"a.tst\" \"0\" \"0\" \"1700000300000000000\")");
    assert!(e.contains("ERR_ZERO"), "{}", e);
    // unknown pool
    let e = it.err("(buy-core \"ft.tst\" \"a.tst\" \"1\" \"0\" \"1700000300000000000\")");
    assert!(e.contains("ERR_NO_POOL"), "{}", e);
    // zero sell
    let e = it.err("(sell-core \"ft.tst\" \"a.tst\" \"0\" \"\" \"0\" \"0\" \"1700000300000000000\")");
    assert!(e.contains("ERR_ZERO"), "{}", e);
    // sell on unknown pool
    let e = it.err("(sell-core \"ft.tst\" \"a.tst\" \"1\" \"\" \"0\" \"0\" \"1700000300000000000\")");
    assert!(e.contains("ERR_NO_POOL"), "{}", e);
    // claim on seniority-off pool
    it.s("(launch-core \"off.tst\" \"0\" \"300000\" \"500000000000000000\" \"1700000000000000000\")");
    let e = it.err("(claim-core \"off.tst\" \"a.tst\")");
    assert!(e.contains("ERR_SENIORITY"), "{}", e);
    // claim with no buys → 0 (no error)
    it.s("(launch-core \"non.tst\" \"1\" \"300000\" \"500000000000000000\" \"1700000000000000000\")");
    assert_eq!(it.s("(claim-core \"non.tst\" \"a.tst\")"), "0");
    // relaunch → ERR_EXISTS
    let e = it.err("(launch-core \"non.tst\" \"1\" \"300000\" \"1\" \"1700000000000000000\")");
    assert!(e.contains("ERR_EXISTS"), "{}", e);
    // slip: min_out above the computed out
    it.s("(sell-core \"non.tst\" \"s.tst\" \"1000000000000000000\" \"seed\" \"0\" \"0\" \"1700000000000000000\")");
    let e = it.err("(buy-core \"non.tst\" \"a.tst\" \"1\" \"999\" \"1700000300000000000\")");
    assert!(e.contains("ERR_SLIP"), "{}", e);
    // seed AFTER launch credits the token reserve directly
    assert_eq!(it.kv("pt:non.tst"), "1000000000000000000");
    assert_eq!(it.kv("es:non.tst"), "0");
    // dust buy (out would be 0) → ERR_ZERO
    it.run("(sput (k-pt \"tiny.tst\") \"1\")");
    it.run("(sput (k-pn \"tiny.tst\") \"500000000000000000\")");
    let e = it.err("(buy-core \"tiny.tst\" \"a.tst\" \"1\" \"0\" \"1700000300000000000\")");
    assert!(e.contains("ERR_ZERO"), "{}", e);
}

// ═══════════════════════════════════════════════════════════════════
// WASM DRIVER (near-compile build + near-mock cross engine)
// ═══════════════════════════════════════════════════════════════════

fn bin(name: &str) -> PathBuf {
    let release = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/release")
        .join(name);
    if release.exists() {
        return release;
    }
    let debug = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(name);
    if debug.exists() {
        return debug;
    }
    panic!("{} not built (run cargo build --release first)", name);
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build_wasm(project: &str) {
    let out = Command::new(bin("near-compile"))
        .current_dir(repo_root())
        .args(["build", project])
        .output()
        .expect("run near-compile");
    let txt = format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "near-compile build {} failed:\n{}", project, txt);
}

/// The pool-v3 root project is a LOCAL-ONLY spike: `.gitignore` line 11
/// (`near.json`) eats its manifest and the root sources were never
/// committed (only pool-v3/token/src is tracked, commit 1edca58). A fresh
/// clone can never run the WASM section — skip loudly instead of failing;
/// machines that DO have the spike still exercise it for real.
fn pool_v3_present() -> bool {
    let ok = repo_root().join("pool-v3/near.json").exists();
    if !ok {
        eprintln!(
            "skip: pool-v3/near.json absent — local-only spike (.gitignore `near.json` \
             rule keeps manifests out of git; root sources never committed)"
        );
    }
    ok
}

struct Wasm {
    state: PathBuf,
}

impl Wasm {
    fn init(state_rel: &str, contracts: &[(&str, &str)]) -> Wasm {
        let state = repo_root().join(state_rel);
        let _ = std::fs::remove_file(&state);
        let _ = std::fs::remove_file(format!("{}.manifest.json", state.display()));
        let mut cmd = Command::new(bin("near-mock"));
        cmd.current_dir(repo_root())
            .args(["init", state_rel])
            .env("NEAR_MOCK_SIGNER", "jp.tst");
        for (acct, wasm) in contracts {
            cmd.arg("--contract").arg(format!("{}={}", acct, wasm));
        }
        let out = cmd.output().expect("run near-mock init");
        assert!(
            out.status.success(),
            "near-mock init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Wasm { state }
    }

    fn call(
        &self,
        acct: &str,
        method: &str,
        args: &str,
        signer: &str,
        attach: &str,
        ts: &str,
        view: bool,
    ) -> String {
        let state_rel = self
            .state
            .strip_prefix(repo_root())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut cmd = Command::new(bin("near-mock"));
        cmd.current_dir(repo_root())
            .args(["call", &state_rel, acct, method, args])
            .arg("--signer").arg(signer)
            .arg("--attach").arg(attach)
            .arg("--block-ts").arg(ts)
            .env("NEAR_MOCK_SIGNER", signer)
            .env("NEAR_MOCK_BLOCK_TS", ts);
        if view {
            cmd.arg("--view");
        }
        let out = cmd.output().expect("run near-mock call");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn ret(&self, acct: &str, method: &str, args: &str, signer: &str, ts: &str) -> String {
        let out = self.call(acct, method, args, signer, "0", ts, true);
        extract_ret(&out).unwrap_or_else(|| panic!("no 📄 return in output:\n{}", out))
    }
}

fn extract_ret(out: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(rest) = line.trim().strip_prefix("📄") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn last_transfer(out: &str) -> Option<String> {
    // receipt lines look like: ↗ transfer 275000000000000000 yocto → alice.tst
    for line in out.lines().rev() {
        let t = line.trim();
        if t.starts_with("↗ transfer") || t.contains("TRANSFER") {
            return Some(t.to_string());
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════════
// WASM TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn wasm_size_budget() {
    let _g = lock();
    if !pool_v3_present() {
        return;
    }
    build_wasm("pool-v3");
    build_wasm("pool-v3/token");
    let pool = std::fs::read(repo_root().join("target/pool_v3.wasm")).unwrap();
    assert!(pool.len() < 40960, "pool_v3.wasm {} bytes ≥ 40KB", pool.len());
    let _tok = std::fs::read(repo_root().join("target/pool_v3_token.wasm")).unwrap();
}

#[test]
fn wasm_full_lifecycle() {
    let _g = lock();
    if !pool_v3_present() {
        return;
    }
    build_wasm("pool-v3");
    build_wasm("pool-v3/token");

    let w = Wasm::init(
        "pool-v3/tests/mock-state.json",
        &[
            ("pool.tst", "target/pool_v3.wasm"),
            ("ft.tst", "target/pool_v3_token.wasm"),
            ("ft2.tst", "target/pool_v3_token.wasm"),
        ],
    );

    // faucet: jp mints seed tokens on both token contracts
    let mint = format!("{{\"account_id\":\"jp.tst\",\"amount\":\"{}\"}}", SEED);
    assert!(w.call("ft.tst", "mint_to", &mint, "jp.tst", "0", T0, false).contains("📄"));
    assert!(w.call("ft2.tst", "mint_to", &mint, "jp.tst", "0", T0, false).contains("📄"));

    // ── seed BEFORE launch → escrow; get_pool still null ──
    let seed_call = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"seed\"}}",
        SEED
    );
    let out = w.call("ft.tst", "ft_transfer_call", &seed_call, "jp.tst", "1", T0, false);
    assert!(out.contains("✅"), "seed escrow failed:\n{}", out);
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft.tst\"}", "jp.tst", T0),
        "null"
    );

    // ── launch with the escrowed credit ──
    let out = w.call(
        "pool.tst",
        "launch",
        "{\"token\":\"ft.tst\",\"seniority\":1,\"gate_ms\":300000}",
        "jp.tst",
        RN0,
        T0,
        false,
    );
    assert!(out.contains("📄 1"), "launch failed:\n{}", out);
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft.tst\"}", "jp.tst", T0),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", RN0, SEED)
    );

    // ── quote_buy exact ──
    assert_eq!(
        w.ret(
            "pool.tst",
            "quote_buy",
            "{\"token\":\"ft.tst\",\"near_in\":\"500000000000000000\"}",
            "jp.tst",
            T0
        ),
        format!(
            "{{\"near_in\":\"500000000000000000\",\"tokens_out\":\"{}\"}}",
            OUT_A
        )
    );
    // quote on an unknown pool → null
    assert_eq!(
        w.ret("pool.tst", "quote_buy", "{\"token\":\"nope.tst\",\"near_in\":\"1\"}", "jp.tst", T0),
        "null"
    );

    // ── gate: buy one ns early reverts ERR_EARLY ──
    let out = w.call(
        "pool.tst",
        "buy",
        "{\"token\":\"ft.tst\",\"min_out\":\"0\"}",
        "alice.tst",
        OUT_A,
        "1700000299999999999",
        false,
    );
    assert!(out.contains("ERR_EARLY"), "gate should revert:\n{}", out);

    // ── alice buys AT the gate ──
    let out = w.call(
        "pool.tst",
        "buy",
        "{\"token\":\"ft.tst\",\"min_out\":\"0\"}",
        "alice.tst",
        OUT_A,
        TG,
        false,
    );
    assert!(out.contains("✅"), "buy failed:\n{}", out);
    // (the 📄 shown is the promise-returned ft_transfer receipt; the real
    //  assertion is the reserve + balance check below)
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft.tst\"}", "jp.tst", TG),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", R1_N, R1_T)
    );
    // her token balance came from the pool's ft_transfer promise
    assert_eq!(
        w.ret("ft.tst", "ft_balance_of", "{\"account_id\":\"alice.tst\"}", "jp.tst", TG),
        OUT_A
    );

    // ── alice sells everything at T60 (flip 4500) ──
    let sell = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"\"}}",
        OUT_A
    );
    let out = w.call("ft.tst", "ft_transfer_call", &sell, "alice.tst", "1", T60, false);
    assert!(out.contains("✅"), "sell failed:\n{}", out);
    // NEAR payout receipt
    let tr = last_transfer(&out).expect("no transfer receipt");
    assert!(tr.contains(SELL_PAYOUT), "payout receipt: {}", tr);
    // alice refunded nothing (keep = full amount)
    assert_eq!(
        w.ret("ft.tst", "ft_balance_of", "{\"account_id\":\"alice.tst\"}", "jp.tst", T60),
        "0"
    );
    // pool reserves: tax stays in depth
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft.tst\"}", "jp.tst", T60),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", R2_N, R2_T)
    );
    // taxes for alice at T60: flip 4500, mcap 0 (ratio 1), pot accrued
    assert_eq!(
        w.ret("pool.tst", "get_taxes", "{\"token\":\"ft.tst\"}", "alice.tst", T60),
        format!(
            "{{\"flip_bp\":4500,\"mcap_bp\":0,\"pot\":\"{}\",\"weights_sum\":\"1000000\"}}",
            POT1
        )
    );

    // ── bob buys at TB ──
    let out = w.call(
        "pool.tst",
        "buy",
        "{\"token\":\"ft.tst\",\"min_out\":\"0\"}",
        "bob.tst",
        OUT_A,
        TB,
        false,
    );
    assert!(out.contains("✅"), "bob buy:\n{}", out);
    assert_eq!(
        w.ret("ft.tst", "ft_balance_of", "{\"account_id\":\"bob.tst\"}", "jp.tst", TB),
        OUT_B
    );

    // ── bob min-out revert: keep 0, full token refund, no payout ──
    let s3 = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"{{\\\"min_out\\\":\\\"2066336190574547\\\"}}\"}}",
        BOB_S3
    );
    let out = w.call("ft.tst", "ft_transfer_call", &s3, "bob.tst", "1", TS2, false);
    assert!(out.contains("✅"), "min-out revert should not fail the tx:\n{}", out);
    assert!(last_transfer(&out).is_none(), "no NEAR payout on revert:\n{}", out);
    assert_eq!(
        w.ret("ft.tst", "ft_balance_of", "{\"account_id\":\"bob.tst\"}", "jp.tst", TS2),
        OUT_B,
        "bob's tokens must be fully refunded"
    );
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft.tst\"}", "jp.tst", TS2),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", R3_N, R3_T),
        "reverted sell must not move reserves"
    );

    // ── bob partial-keep with a max_near cap ──
    let s2 = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"{{\\\"max_near\\\":\\\"{}\\\"}}\"}}",
        BOB_S2, BOB_M
    );
    let out = w.call("ft.tst", "ft_transfer_call", &s2, "bob.tst", "1", TS2, false);
    assert!(out.contains("✅"), "partial keep failed:\n{}", out);
    let tr = last_transfer(&out).expect("no payout receipt");
    assert!(tr.contains(BOB_GA), "capped payout receipt: {}", tr);
    assert_eq!(
        w.ret("ft.tst", "ft_balance_of", "{\"account_id\":\"bob.tst\"}", "jp.tst", TS2),
        BOB_AFTER,
        "only the consumed tokens stay with the pool"
    );

    // ── seniority claim: alice's share, then 0 on a second claim ──
    let out = w.call(
        "pool.tst",
        "claim_seniority",
        "{\"token\":\"ft.tst\"}",
        "alice.tst",
        "0",
        TS2,
        false,
    );
    assert!(out.contains(&format!("📄 {}", CLAIM_A)), "claim:\n{}", out);
    let tr = last_transfer(&out).expect("claim transfer");
    assert!(tr.contains(CLAIM_A), "claim receipt: {}", tr);
    assert_eq!(
        w.ret("pool.tst", "claim_seniority", "{\"token\":\"ft.tst\"}", "alice.tst", TS2),
        "0",
        "second claim pays 0"
    );
    // pot after alice's claim
    let rest: u128 = POT1.parse::<u128>().unwrap() - CLAIM_A.parse::<u128>().unwrap();
    assert_eq!(
        w.ret("pool.tst", "get_taxes", "{\"token\":\"ft.tst\"}", "alice.tst", TS2),
        format!(
            "{{\"flip_bp\":0,\"mcap_bp\":0,\"pot\":\"{}\",\"weights_sum\":\"816326\"}}",
            rest
        )
    );

    // ── ft2 pool: seniority OFF + POST-init seed + dust sell ──
    let out = w.call(
        "pool.tst",
        "launch",
        "{\"token\":\"ft2.tst\",\"seniority\":0,\"gate_ms\":300000}",
        "jp.tst",
        RN0,
        T0,
        false,
    );
    assert!(out.contains("📄 1"), "ft2 launch:\n{}", out);
    let seed2 = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"seed\"}}",
        SEED
    );
    let out = w.call("ft2.tst", "ft_transfer_call", &seed2, "jp.tst", "1", T0, false);
    assert!(out.contains("✅"), "post-init seed:\n{}", out);
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft2.tst\"}", "jp.tst", T0),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", RN0, SEED)
    );
    // carol buys at the gate
    let out = w.call(
        "pool.tst",
        "buy",
        "{\"token\":\"ft2.tst\",\"min_out\":\"0\"}",
        "carol.tst",
        OUT_A,
        TG,
        false,
    );
    assert!(out.contains("✅"), "carol buy:\n{}", out);
    // carol sells 4e17 of it at T60 — tax all to depth, pot stays 0
    let csell = format!(
        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{}\",\"msg\":\"\"}}",
        CAROL_SELL
    );
    let out = w.call("ft2.tst", "ft_transfer_call", &csell, "carol.tst", "1", T60, false);
    assert!(out.contains("✅"), "carol sell:\n{}", out);
    let tr = last_transfer(&out).expect("carol payout");
    assert!(tr.contains(CAROL_PAY), "carol payout: {}", tr);
    assert_eq!(
        w.ret("pool.tst", "get_taxes", "{\"token\":\"ft2.tst\"}", "carol.tst", T60),
        "{\"flip_bp\":4500,\"mcap_bp\":0,\"pot\":\"0\",\"weights_sum\":\"1000000\"}",
        "seniority off → pot never accrues (weights still recorded, unused)"
    );
    // reserve check: Rn 1e18 − payout
    let rn2c: u128 = 1000000000000000000u128 - CAROL_PAY.parse::<u128>().unwrap();
    let rt2c: u128 = 500000000000000000 + CAROL_SELL.parse::<u128>().unwrap();
    assert_eq!(
        w.ret("pool.tst", "get_pool", "{\"token\":\"ft2.tst\"}", "jp.tst", T60),
        format!("{{\"near\":\"{}\",\"tokens\":\"{}\"}}", rn2c, rt2c)
    );
    // dust sell: 1 token → gross 0 → keep 0, full refund
    let dust = "{\"receiver_id\":\"pool.tst\",\"amount\":\"1\",\"msg\":\"\"}";
    let bal_before = w.ret("ft2.tst", "ft_balance_of", "{\"account_id\":\"carol.tst\"}", "jp.tst", T60);
    let out = w.call("ft2.tst", "ft_transfer_call", dust, "carol.tst", "1", T60, false);
    assert!(out.contains("✅"), "dust sell:\n{}", out);
    assert_eq!(
        w.ret("ft2.tst", "ft_balance_of", "{\"account_id\":\"carol.tst\"}", "jp.tst", T60),
        bal_before,
        "dust must be fully refunded"
    );
    // claim on a seniority-off pool traps ERR_SENIORITY
    let out = w.call(
        "pool.tst",
        "claim_seniority",
        "{\"token\":\"ft2.tst\"}",
        "carol.tst",
        "0",
        T60,
        false,
    );
    assert!(out.contains("ERR_SENIORITY"), "claim on off-pool:\n{}", out);
}
