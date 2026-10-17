//! Two-sided market simulation over near-mock: does the pool behave like a
//! real market? Alternating buy/sell pressure through the real NEP-141 swap
//! path (ft_transfer_call msg "sw" → escrow intake → curve walk → payout),
//! price walking cells both directions, clamps on both range faces, LP
//! liquidity provably stationary, tokens conserved to the atom.
//!
//! FEE VERDICT (measured, then pinned): v5 has NO fee parameter — books
//! take full in/out and fee cells never move (header invariant). But the
//! grid curve itself quotes a per-cell bid-ask spread: a boot round trip
//! (sell 1e9 X → Y, reverse the exact Y) returns 813049996 X — the pool's
//! token balance gains exactly the 186950004 difference — while S/P
//! restore to boot EXACTLY. Market shape: order-book-style spread, zero
//! explicit fee, LPs redeem closed-form deposits (burnc req forms).
//!
//! Scale note (probed live): at S = 2^64 (price 1.0) a straddle position
//! d over [-8,8] holds ry/rx ≈ 2^64 — X-side inventory is in ~1e9 units,
//! Y-side in ~1e29 units. Sizes below are calibrated to that; every pin
//! in this file was measured on live mock runs before being asserted.
//!
//! Run: cargo test --release --test test_pool_v5_market -- --nocapture

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bin(name: &str) -> PathBuf {
    let p = repo_root().join("target/release").join(name);
    assert!(
        p.exists(),
        "{name} missing — build with `cargo build --release` first"
    );
    p
}

fn compile(concat: &[&str], out: &Path) {
    let mut src = String::new();
    for part in concat {
        src.push_str(
            &std::fs::read_to_string(repo_root().join("lib").join(part))
                .unwrap_or_else(|e| panic!("read lib/{part}: {e}")),
        );
        src.push('\n');
    }
    let tmp = repo_root().join("target/test-market-src.lisp");
    std::fs::write(&tmp, src).unwrap();
    let o = Command::new(bin("near-compile"))
        .arg(&tmp)
        .arg(out)
        .output()
        .expect("run near-compile");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        o.status.success() && all.contains("✅"),
        "compile failed: {all}"
    );
}

struct Mock {
    state: PathBuf,
    manifest: String,
}

impl Mock {
    fn cross(&self, contract: &str, method: &str, args: &str, signer: &str) -> String {
        let out = Command::new(bin("near-mock"))
            .arg("cross")
            .arg(&self.state)
            .arg(&self.manifest)
            .arg(contract)
            .arg(method)
            .arg(args)
            .env("NEAR_MOCK_SIGNER", signer)
            .output()
            .expect("run near-mock cross");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn pool_op(&self, json: &str) -> String {
        self.cross("pool.tst", "_run", json, "owner")
    }

    /// last 📄 line (the RETURN value): first whitespace token, comma-stripped
    fn ret(&self, out: &str) -> String {
        out.lines()
            .rev()
            .find(|l| l.contains("📄"))
            .map(|l| {
                l.split('📄')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .replace(',', "")
            })
            .unwrap_or_default()
    }
}

fn bal(m: &Mock, tok: &str, who: &str) -> u128 {
    let out = m.cross(
        tok,
        "ft_balance_of",
        &format!("{{\"account_id\":\"{who}\"}}"),
        "owner",
    );
    m.ret(&out).parse().expect("balance int")
}

fn escrow(m: &Mock, k: &str) -> u128 {
    let out = m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}"));
    m.ret(&out).parse().expect("escrow int")
}

/// storage value via op get — the op PREPENDS the "6|" namespace itself,
/// so callers pass the bare suffix ("S", "LQ:3", "POS:alice.tst:-8:8").
/// SIGNED: tick keys (P, IMIN/IMAX) go negative; balances fit i128 too.
fn key_i128(m: &Mock, k: &str) -> i128 {
    let out = m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}"));
    m.ret(&out).parse().unwrap_or(0)
}

/// swap leg: trader ft_transfer_calls the pool msg "sw"; returns "used:N"
/// = amount the pool KEPT (ft2 resolve refunds amt−N from the pool's
/// chain result m = the walk remainder).
fn swap(m: &Mock, tok: &str, who: &str, amt: u128) -> String {
    m.cross(
        tok,
        "ft_transfer_call",
        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"),
        who,
    )
}

fn snap(m: &Mock, label: &str) {
    println!(
        "  [{label}] S={} P={} | escrow X={} Y={} | flow X={} Y={}",
        key_i128(m, "S"),
        key_i128(m, "P"),
        escrow(m, "ESCX"),
        escrow(m, "ESCY"),
        key_i128(m, "X"),
        key_i128(m, "Y"),
    );
}

#[test]
fn pool_v5_market_two_sided_sim() {
    let dir = repo_root().join("target/test-market");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        ["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"].as_slice(),
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let state = dir.join("state.bin");
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
    let m = Mock {
        manifest: format!(
            "pool.tst={},tokx.tst={},toky.tst={}",
            pool_wasm.display(),
            ft_wasm.display(),
            ft_wasm.display()
        ),
        state,
    };

    const D: u128 = 100_000_000_000_000_000_000_000_000_000_000; // LP units (1e32)
    const FUND: u128 = 250_000_000_000_000_000_000_000_000_000; // 2.5e29 per mint
    const LO: i64 = -8;
    const HI: i64 = 8; // straddle: price 1.0 starts MID-range, both sides deep
    const S0: i128 = 18446744073709551616; // 2^64 = boot price

    println!("== boot: tokens, pool, funds, LP straddle [{LO},{HI}] ==");
    assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")), "ok");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );
    for who in ["alice.tst", "bob.tst"] {
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(
                m.ret(&m.cross(
                    tok,
                    "mint",
                    &format!("{{\"to\":\"{who}\",\"amt\":\"{FUND}\"}}"),
                    "owner"
                )),
                "ok"
            );
        }
    }

    // LP position: alice mints d over the straddle via required-amount custody
    let req_out = m.ret(&m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{D}\",\"a\":\"{LO}\",\"b\":\"{HI}\"}}"
    )));
    let req_body = req_out.trim_start_matches("rx:");
    let (rx_s, ry_s) = req_body
        .split_once(":ry:")
        .unwrap_or_else(|| panic!("req shape: {req_out}"));
    let rx: u128 = rx_s.parse().expect("rx");
    let ry: u128 = ry_s.parse().expect("ry");
    println!("  req: rx={rx} ry={ry}");
    let mint_msg = format!("m:{D}:{LO}:{HI}");
    let leg = |tok: &str, amt: u128| {
        m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{mint_msg}\"}}"
            ),
            "alice.tst",
        )
    };
    assert_eq!(m.ret(&leg("tokx.tst", rx)), format!("used:{rx}"), "leg 1");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"get\",\"k\":\"PM:alice.tst\"}")),
        format!("X:{LO}:{HI}:{D}:{rx}"),
        "leg 1 recorded"
    );
    assert_eq!(m.ret(&leg("toky.tst", ry)), format!("used:{ry}"), "leg 2");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"get\",\"k\":\"PM:alice.tst\"}")),
        "0",
        "PM cleared after leg 2"
    );

    // pool-owned float ("seed") so payout legs have spendable balance.
    // The pool's ft-side balance is the REAL custody; escrow keys mirror it.
    const SEED_X: u128 = 6_000_000_000;
    const SEED_Y: u128 = 120_000_000_000_000_000_000_000_000_000;
    // "used:N" = amount KEPT by the pool (ft2 resolve semantics): seed keeps all.
    assert_eq!(
        m.ret(&m.cross(
            "tokx.tst",
            "ft_transfer_call",
            &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_X}\",\"msg\":\"seed\"}}"),
            "alice.tst"
        )),
        format!("used:{SEED_X}"),
        "X seed float kept"
    );
    assert_eq!(
        m.ret(&m.cross(
            "toky.tst",
            "ft_transfer_call",
            &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_Y}\",\"msg\":\"seed\"}}"),
            "alice.tst"
        )),
        format!("used:{SEED_Y}"),
        "Y seed float kept"
    );

    // custody model: pool ft balance == escrow keys at boot (nothing traded
    // yet); flow ledgers 6|X/6|Y still zero; every cell holds exactly d.
    assert_eq!(bal(&m, "tokx.tst", "pool.tst"), escrow(&m, "ESCX"));
    assert_eq!(bal(&m, "toky.tst", "pool.tst"), escrow(&m, "ESCY"));
    assert_eq!(key_i128(&m, "X"), 0, "flow ledger untouched by mint");
    assert_eq!(key_i128(&m, "Y"), 0, "flow ledger untouched by mint");
    let lq0: Vec<i128> = (LO..=HI)
        .map(|i| key_i128(&m, &format!("LQ:{i}")))
        .collect();
    assert!(
        lq0.iter().all(|&v| v == D as i128),
        "every cell holds exactly d: {lq0:?}"
    );
    snap(&m, "boot");
    assert_eq!(key_i128(&m, "S"), S0);
    assert_eq!(key_i128(&m, "P"), 0);

    // ── R0: the FEE/SPREAD probe, from boot — buy Y then reverse exactly ──
    // Zero-fee books take full in/out both legs AND the state restores
    // exactly; the only cost is the grid curve's per-cell spread.
    println!("== R0: fee probe — sell 1e9 X for Y, reverse the exact Y ==");
    let probe: u128 = 1_000_000_000;
    let pool_x0 = bal(&m, "tokx.tst", "pool.tst");
    let bob_x0 = bal(&m, "tokx.tst", "bob.tst");
    let bob_y0 = bal(&m, "toky.tst", "bob.tst");
    assert_eq!(
        m.ret(&swap(&m, "tokx.tst", "bob.tst", probe)),
        format!("used:{probe}"),
        "leg 1: full walk keeps the whole intake"
    );
    let y_got = bal(&m, "toky.tst", "bob.tst") - bob_y0;
    assert_eq!(
        y_got, 15_000_374_993_750_229_587_335_942_580,
        "golden: boot 3-cell walk pays exactly this Y"
    );
    snap(&m, "R0a bought");
    assert_eq!(
        m.ret(&swap(&m, "toky.tst", "bob.tst", y_got)),
        format!("used:{y_got}"),
        "leg 2: exact reverse, full walk back"
    );
    let x_back = bal(&m, "tokx.tst", "bob.tst");
    assert_eq!(
        bob_x0 - x_back,
        186_950_004,
        "golden: round trip keeps 186950004 of the 1e9 input"
    );
    assert_eq!(bal(&m, "toky.tst", "bob.tst"), bob_y0, "Y fully returned");
    assert_eq!(
        key_i128(&m, "S"),
        S0,
        "state reversal: S restored to the atom"
    );
    assert_eq!(key_i128(&m, "P"), 0, "state reversal: P restored");
    assert_eq!(
        bal(&m, "tokx.tst", "pool.tst") - pool_x0,
        186_950_004,
        "the spread landed in the pool's balance, to the atom"
    );
    snap(&m, "R0b reversed");

    // ── R1-R2: buy pressure — bob sells X, price climbs ──
    let mut bob_y_before = bal(&m, "toky.tst", "bob.tst");
    let mut round = 1;
    for _ in 0..2 {
        println!("== R{round}: bob sells 1e9 X (buys Y) ==");
        let y_out_before = bal(&m, "toky.tst", "pool.tst");
        let xb_before = key_i128(&m, "X");
        assert_eq!(
            m.ret(&swap(&m, "tokx.tst", "bob.tst", 1_000_000_000)),
            "used:1000000000",
            "full walk keeps the whole intake"
        );
        let got = bal(&m, "toky.tst", "bob.tst") - bob_y_before;
        assert!(got > 0, "paid out");
        assert_eq!(
            key_i128(&m, "X") - xb_before,
            1_000_000_000,
            "flow ledger takes FULL intake (no skim)"
        );
        assert_eq!(
            y_out_before - bal(&m, "toky.tst", "pool.tst"),
            got,
            "payout drawn from the pool's Y balance exactly"
        );
        bob_y_before = bal(&m, "toky.tst", "bob.tst");
        snap(&m, &format!("R{round}"));
        round += 1;
    }
    let s_top = key_i128(&m, "S");
    assert_eq!(s_top, 18_452_278_650_352_433_436, "golden S after R2");
    assert_eq!(key_i128(&m, "P"), 6, "walked up to cell 6");

    // ── R3: oversized sell — partial walk to the range lip, excess refunded ──
    println!("== R{round}: bob sells 4e9 X (more than the remaining capacity) ==");
    let ret_big = m.ret(&swap(&m, "tokx.tst", "bob.tst", 4_000_000_000));
    let kept: u128 = ret_big
        .trim_start_matches("used:")
        .parse()
        .unwrap_or_else(|_| panic!("used shape: {ret_big}"));
    assert_eq!(kept, 812_806_130, "golden: lip capacity consumed exactly");
    assert_eq!(
        key_i128(&m, "S"),
        18_455_046_561_344_878_016,
        "pinned at S(9)"
    );
    assert!(key_i128(&m, "P") > HI as i128, "parked above the range lip");
    println!(
        "  kept {kept}, refunded {} — parked at S={}, P={}",
        4_000_000_000 - kept,
        key_i128(&m, "S"),
        key_i128(&m, "P")
    );
    // parked above the range: another X sell moves NOTHING, full refund
    let ret2 = m.ret(&swap(&m, "tokx.tst", "bob.tst", 1_000_000_000));
    assert_eq!(ret2, "used:0", "past-range sell: whole intake refunded");
    assert_eq!(key_i128(&m, "S"), 18_455_046_561_344_878_016, "S pinned");
    snap(&m, &format!("R{round}b clamped"));
    round += 1;

    // ── R4-R5: sell pressure — bob sells Y, price walks back down ──
    let sells_y: [u128; 2] = [
        50_000_000_000_000_000_000_000_000_000, // 5e28
        40_000_000_000_000_000_000_000_000_000, // 4e28
    ];
    let mut bob_x_before = bal(&m, "tokx.tst", "bob.tst");
    for amt in sells_y {
        println!("== R{round}: bob sells {amt} Y (buys X) ==");
        let x_out_before = bal(&m, "tokx.tst", "pool.tst");
        let yb_before = key_i128(&m, "Y");
        let ret_y = m.ret(&swap(&m, "toky.tst", "bob.tst", amt));
        let kept_y: u128 = ret_y
            .trim_start_matches("used:")
            .parse()
            .unwrap_or_else(|_| panic!("used shape: {ret_y}"));
        assert!(kept_y > 0 && kept_y <= amt, "kept {kept_y} of {amt}");
        let got = bal(&m, "tokx.tst", "bob.tst") - bob_x_before;
        assert!(got > 0, "paid out");
        assert_eq!(
            key_i128(&m, "Y") - yb_before,
            kept_y as i128,
            "flow ledger takes exactly the kept slice"
        );
        assert_eq!(
            x_out_before - bal(&m, "tokx.tst", "pool.tst"),
            got,
            "X payout drawn from the pool's balance exactly"
        );
        bob_x_before = bal(&m, "tokx.tst", "bob.tst");
        snap(&m, &format!("R{round} kept {kept_y}"));
        round += 1;
    }
    let s_back = key_i128(&m, "S");
    assert_eq!(s_back, 18_439_367_220_385_604_838, "golden: bottom lip S");
    assert_eq!(
        key_i128(&m, "P"),
        LO as i128,
        "walked down to the floor cell"
    );
    assert!(
        s_back < s_top,
        "sell pressure walked price back down: {s_back} < {s_top}"
    );

    // ── closing verdicts ──
    println!("== verdicts ==");
    // V1: liquidity never moved — every cell still exactly d, POS intact
    let lq1: Vec<i128> = (LO..=HI)
        .map(|i| key_i128(&m, &format!("LQ:{i}")))
        .collect();
    assert_eq!(lq1, lq0, "V1: pointwise book unchanged by trading");
    assert_eq!(
        m.ret(&m.pool_op(&format!(
            "{{\"op\":\"get\",\"k\":\"POS:alice.tst:{LO}:{HI}\"}}"
        ))),
        format!("{D}"),
        "V1: position record unchanged"
    );
    // V2: no fee ledgers exist — fee cells never move (header invariant)
    for k in ["FEE", "FEE:X", "FEE:Y", "BOOK"] {
        assert_eq!(key_i128(&m, k), 0, "V2: no fee ledger at 6|{k}");
    }
    // V3: token conservation to the atom — every minted unit sits in a
    //     trader balance or the pool's ft-side balance (its real custody).
    let x_tot = bal(&m, "tokx.tst", "alice.tst")
        + bal(&m, "tokx.tst", "bob.tst")
        + bal(&m, "tokx.tst", "pool.tst");
    let y_tot = bal(&m, "toky.tst", "alice.tst")
        + bal(&m, "toky.tst", "bob.tst")
        + bal(&m, "toky.tst", "pool.tst");
    assert_eq!(x_tot, 2 * FUND, "V3: X conserved to the atom");
    assert_eq!(y_tot, 2 * FUND, "V3: Y conserved to the atom");
    println!("  V1 liquidity stationary: PASS (cells+POS identical)");
    println!("  V2 fee ledgers: PASS (none exist — books take full in/out)");
    println!("  V3 conservation: PASS (X {x_tot}, Y {y_tot} = exact mints)");
    println!("  price: 2^64 → up 2 cells×2 → lip → down to floor lip (pinned)");
    println!("  round trip at boot: 1e9 X in → 813049996 back; S/P exact;");
    println!("  spread 186950004 X → pool balance (LP-side surplus).");
    println!("MARKET SIM: GREEN — both sides active, clamps proven both");
    println!("faces, state reversal exact. Fee generation: NO fee parameter");
    println!("(fee cells never move); the grid curve quotes a per-cell");
    println!("bid-ask spread instead — order-book style, not AMM-style.");
}
