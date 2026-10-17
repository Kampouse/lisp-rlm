//! Pool-v5 custody (arc C2b): two-leg mint escrow + burnc/claim payout over
//! near-mock.
//!
//! Mint = required-amount custody: each leg ft_transfer_calls the pool with
//! msg "m:d:a:b" attaching EXACTLY its side's req (op "req"); leg 1 records
//! 6|PM:<who> = "X:a:b:d:req" and escrows req; leg 2 matches side+a+b+d,
//! escrows its req, completes (POS/LQ/IMIN/IMAX), refunds any excess via
//! used. msg "c" cancels: plain ft_transfer of the recorded slice, cleared
//! only in the pay callback (failure leaves pending+escrow intact).
//! Burn = op "burnc" (authority + op-burn core, owed ledgers from the same
//! closed forms) + op "claim" per token: single payout leg, cx/cy reversal
//! restores owed+escrow when the receipt fails.
//! Asserts exact balances/escrows/owed on: leg1, cancel(+fault,+retry),
//! leg2 excess refund, burnc, claim happy, claim fault, claim retry.

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
    let tmp = repo_root().join("target/test-c2b-src.lisp");
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

    fn cross_fail(
        &self,
        contract: &str,
        method: &str,
        args: &str,
        signer: &str,
        n: usize,
    ) -> String {
        let out = Command::new(bin("near-mock"))
            .arg("cross")
            .arg(&self.state)
            .arg(&self.manifest)
            .arg(contract)
            .arg(method)
            .arg(args)
            .arg("--fail-receipt")
            .arg(n.to_string())
            .env("NEAR_MOCK_SIGNER", signer)
            .output()
            .expect("run near-mock cross --fail-receipt");
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

#[test]
fn pool_v5_c2b_mint_burn_custody() {
    let dir = repo_root().join("target/test-c2b");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
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

    const D: u128 = 100_000_000_000_000_000_000_000_000; // position units (1e26)
    const FUND: u128 = 100_000_000_000_000_000_000_000_000_000; // alice funds (1e29)
    const A: i64 = 0;
    const B: i64 = 1;

    // bootstrap
    assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")), "ok");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );
    for tok in ["tokx.tst", "toky.tst"] {
        assert_eq!(
            m.ret(&m.cross(
                tok,
                "mint",
                &format!("{{\"to\":\"alice.tst\",\"amt\":\"{FUND}\"}}"),
                "owner"
            )),
            "ok"
        );
    }

    // required amounts (S-independent closed form)
    let req_out = m.ret(&m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{D}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"
    )));
    let req_body = req_out.trim_start_matches("rx:");
    let (rx_s, ry_s) = req_body
        .split_once(":ry:")
        .unwrap_or_else(|| panic!("req shape: {req_out}"));
    let rx: u128 = rx_s.parse().expect("rx");
    let ry: u128 = ry_s.parse().expect("ry");
    assert!(rx > 0 && ry > 0, "nonzero reqs at test scale: {req_out}");

    let mint_msg = format!("m:{D}:{A}:{B}");
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

    // ── leg 1: exact attach, record + escrow ──
    assert_eq!(m.ret(&leg("tokx.tst", rx)), format!("used:{rx}"));
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"PM:alice.tst\"}}"))),
        format!("X:{A}:{B}:{D}:{rx}")
    );
    assert_eq!(escrow(&m, "ESCX"), rx);

    // ── cancel: fault leaves pending+escrow intact; clean retry refunds ──
    let cf = m.cross_fail(
        "tokx.tst",
        "ft_transfer_call",
        "{\"receiver_id\":\"pool.tst\",\"amount\":\"0\",\"msg\":\"c\"}",
        "alice.tst",
        0,
    );
    assert_eq!(
        m.ret(&cf),
        "used:0",
        "failed cancel ⇒ full refund of the 0-leg"
    );
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"get\",\"k\":\"PM:alice.tst\"}")),
        format!("X:{A}:{B}:{D}:{rx}"),
        "pending record survives cancel failure"
    );
    assert_eq!(escrow(&m, "ESCX"), rx);
    assert_eq!(bal(&m, "tokx.tst", "alice.tst"), FUND - rx);

    assert_eq!(
        m.ret(&m.cross(
            "tokx.tst",
            "ft_transfer_call",
            "{\"receiver_id\":\"pool.tst\",\"amount\":\"0\",\"msg\":\"c\"}",
            "alice.tst",
        )),
        "used:0"
    );
    assert_eq!(escrow(&m, "ESCX"), 0, "clean cancel drains the slice");
    assert_eq!(bal(&m, "tokx.tst", "alice.tst"), FUND, "alice made whole");

    // ── leg 1 again, then leg 2 with EXCESS (refund via used) ──
    assert_eq!(m.ret(&leg("tokx.tst", rx)), format!("used:{rx}"));
    let ry_excess = ry + 1000;
    assert_eq!(m.ret(&leg("toky.tst", ry_excess)), format!("used:{ry}"));
    assert_eq!(escrow(&m, "ESCX"), rx);
    assert_eq!(escrow(&m, "ESCY"), ry);
    assert_eq!(
        bal(&m, "toky.tst", "alice.tst"),
        FUND - ry,
        "excess refunded"
    );
    assert_eq!(
        m.ret(&m.pool_op(&format!(
            "{{\"op\":\"get\",\"k\":\"POS:alice.tst:{A}:{B}\"}}"
        ))),
        D.to_string(),
        "position minted"
    );
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}"))),
        D.to_string()
    );

    // ── burnc: authority + owed ledgers from the same closed forms ──
    assert_eq!(
        m.ret(&m.cross(
            "pool.tst",
            "_run",
            &format!("{{\"op\":\"burnc\",\"d\":\"{D}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"),
            "alice.tst"
        )),
        format!("burnc-ok:{rx}:{ry}")
    );
    assert_eq!(escrow(&m, "OWX:alice.tst"), rx, "owed X = req");
    assert_eq!(escrow(&m, "OWY:alice.tst"), ry, "owed Y = req");
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}"))),
        "0",
        "liquidity burned"
    );

    // ── claim X: happy path pays the owed slice ──
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"x\"}", "alice.tst")),
        "0",
        "claim consumes the 0-attach leg"
    );
    assert_eq!(escrow(&m, "ESCX"), 0);
    assert_eq!(escrow(&m, "OWX:alice.tst"), 0);
    assert_eq!(bal(&m, "tokx.tst", "alice.tst"), FUND, "alice X whole");

    // ── claim Y: payout receipt forced to fail ⇒ owed+escrow restored ──
    let y_before = bal(&m, "toky.tst", "alice.tst");
    assert_eq!(
        m.ret(&m.cross_fail(
            "pool.tst",
            "_run",
            "{\"op\":\"claim\",\"t\":\"y\"}",
            "alice.tst",
            0
        )),
        "0"
    );
    assert_eq!(escrow(&m, "OWY:alice.tst"), ry, "owed restored on failure");
    assert_eq!(escrow(&m, "ESCY"), ry, "escrow restored on failure");
    assert_eq!(
        bal(&m, "toky.tst", "alice.tst"),
        y_before,
        "no tokens moved"
    );

    // ── regression (live tx 2026-10-17): UPPERCASE "Y" must route to the
    // Y branch. Old code matched (str= tok "y") strictly and silently
    // took the X branch instead (OWX already 0 ⇒ "no-owed", Y stuck). ──
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"Y\"}", "alice.tst")),
        "0"
    );
    assert_eq!(escrow(&m, "ESCY"), 0, "uppercase claim drained Y escrow");
    assert_eq!(escrow(&m, "OWY:alice.tst"), 0, "uppercase claim consumed owed");
    assert_eq!(bal(&m, "toky.tst", "alice.tst"), FUND, "alice Y whole");

    // ── lowercase retry after uppercase: clean no-op (no double-pay) ──
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"y\"}", "alice.tst")),
        "no-owed"
    );
    assert_eq!(bal(&m, "toky.tst", "alice.tst"), FUND, "still whole");

    // ── unknown token rejected (old fall-through paid the X branch) ──
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"z\"}", "alice.tst")),
        "bad-token"
    );
    assert_eq!(bal(&m, "toky.tst", "alice.tst"), FUND, "no dust moved");

    // pool ended where it started (no dust anywhere)
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"view\"}")),
        "S:18446744073709551616:P:0:X:0:Y:0:IMIN:0:IMAX:0",
        "pool back to seed state — no dust"
    );

    // ── view-surface parity: escrow op == get-keys == raw FT balances ──
    // The gauntlet's chain layer reconciles exactly this triple on live
    // chain; pinned here at the funded checkpoints (final: all zero).
    let esc = m.ret(&m.pool_op("{\"op\":\"escrow\"}"));
    assert_eq!(esc, "ESCX:0:ESCY:0", "escrow op matches get-keys at 0");
    assert_eq!(
        bal(&m, "tokx.tst", "pool.tst"),
        escrow(&m, "ESCX") + m.ret(&m.pool_op("{\"op\":\"get\",\"k\":\"X\"}")).parse::<u128>().unwrap(),
        "raw tokx(pool) == ESCX + reserve X"
    );
    assert_eq!(
        bal(&m, "toky.tst", "pool.tst"),
        escrow(&m, "ESCY") + m.ret(&m.pool_op("{\"op\":\"get\",\"k\":\"Y\"}")).parse::<u128>().unwrap(),
        "raw toky(pool) == ESCY + reserve Y"
    );
    let st = m.ret(&m.pool_op("{\"op\":\"state\"}"));
    // fee-gen (Oct 16): state carries parked-fee totals too; this flow does
    // no swaps, so both totals must read 0 (no phantom fee mass).
    assert!(
        st.ends_with("TOKX:tokx.tst:TOKY:toky.tst:FEEX:0:FEEY:0"),
        "state carries token ids + zero fee totals"
    );
}
// Regression pin (8a56ddcf): burnc support-shrink must compare LQ in the
// string domain. `p5-lq-nz?` used to push token-scale LQ through checked
// 60-bit `str->num`, so ANY partial burn leaving residue > 2^60 trapped
// (`unreachable` in p5-shrink-imin) — pool1.clmm5-51fe5a.testnet, Oct 16.
// Every prior burn pin burned fresh pools to zero, so the seam never fired.
#[test]
fn pool_v5_c2b_partial_burn_outsized_lq() {
    let dir = repo_root().join("target/test-c2b-pb");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
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

    const D1: u128 = 1_000_000_000_000_000_000_000_000_000; // outsized mint (1e27 >> 2^60)
    const D2: u128 = 100_000_000_000_000_000_000_000_000; // partial burn (1e26)
    const FUND: u128 = 100_000_000_000_000_000_000_000_000_000; // alice funds (1e29)
    const A: i64 = 0;
    const B: i64 = 1;
    assert!(D1 - D2 > (1u128 << 60), "residue must stay token-scale");

    // bootstrap
    assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")), "ok");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );
    for tok in ["tokx.tst", "toky.tst"] {
        assert_eq!(
            m.ret(&m.cross(
                tok,
                "mint",
                &format!("{{\"to\":\"alice.tst\",\"amt\":\"{FUND}\"}}"),
                "owner"
            )),
            "ok"
        );
    }

    // mint D1 in two legs (exact attach from the req closed form)
    let req_out = m.ret(&m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{D1}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"
    )));
    let req_body = req_out.trim_start_matches("rx:");
    let (rx1_s, ry1_s) = req_body
        .split_once(":ry:")
        .unwrap_or_else(|| panic!("req shape: {req_out}"));
    let rx1: u128 = rx1_s.parse().expect("rx1");
    let ry1: u128 = ry1_s.parse().expect("ry1");
    let leg = |tok: &str, amt: u128| {
        m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"m:{D1}:{A}:{B}\"}}"
            ),
            "alice.tst",
        )
    };
    assert_eq!(m.ret(&leg("tokx.tst", rx1)), format!("used:{rx1}"));
    assert_eq!(m.ret(&leg("toky.tst", ry1)), format!("used:{ry1}"));
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}"))),
        D1.to_string(),
        "LQ cell holds a >2^60 value (the poison state)"
    );

    // -- partial burn D2: shrink probes residue LQ = D1-D2 > 2^60 --
    // Old code: str->num overflow => unreachable trap. Must be burnc-ok.
    let burn_out = m.ret(&m.cross(
        "pool.tst",
        "_run",
        &format!("{{\"op\":\"burnc\",\"d\":\"{D2}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"),
        "alice.tst",
    ));
    let burn_body = burn_out.trim_start_matches("burnc-ok:");
    let (ox1_s, oy1_s) = burn_body
        .split_once(":")
        .unwrap_or_else(|| panic!("burnc shape: {burn_out}"));
    let ox1: u128 = ox1_s.parse().expect("ox1");
    let oy1: u128 = oy1_s.parse().expect("oy1");
    assert!(ox1 > 0 && oy1 > 0, "partial burn pays out");
    assert_eq!(escrow(&m, "OWX:alice.tst"), ox1);
    assert_eq!(escrow(&m, "OWY:alice.tst"), oy1);
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}"))),
        (D1 - D2).to_string(),
        "residue intact - this read used to trap the shrink probe"
    );

    // claims pay the partial-burn slice
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"x\"}", "alice.tst")),
        "0"
    );
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"y\"}", "alice.tst")),
        "0"
    );
    assert_eq!(escrow(&m, "OWX:alice.tst"), 0);
    assert_eq!(escrow(&m, "OWY:alice.tst"), 0);

    // -- full exit of the residue: shrink probes LQ = 9e26 AGAIN --
    let burn2_out = m.ret(&m.cross(
        "pool.tst",
        "_run",
        &format!(
            "{{\"op\":\"burnc\",\"d\":\"{}\",\"a\":\"{A}\",\"b\":\"{B}\"}}",
            D1 - D2
        ),
        "alice.tst",
    ));
    let burn2_body = burn2_out.trim_start_matches("burnc-ok:");
    let (ox2_s, oy2_s) = burn2_body
        .split_once(":")
        .unwrap_or_else(|| panic!("burnc shape: {burn2_out}"));
    let ox2: u128 = ox2_s.parse().expect("ox2");
    let oy2: u128 = oy2_s.parse().expect("oy2");

    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}"))),
        "0",
        "position fully closed"
    );

    // final claims
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"x\"}", "alice.tst")),
        "0"
    );
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"claim\",\"t\":\"y\"}", "alice.tst")),
        "0"
    );

    // Conservation across both burns. burnc owes req(d) PER burn (same
    // closed form as the mint requirement); floor division is not additive
    // across splits, so payouts may trail the single-mint intake by at most
    // burns-1 yocto. The dust stays in the pool float - no burner is ever
    // shorted (each payout is req of their own burn amount), and a single
    // burn of a whole position stays exact (see the test above + pool1/pool2
    // on-chain).
    let dust_x = rx1 - (ox1 + ox2);
    let dust_y = ry1 - (oy1 + oy2);
    assert!(dust_x <= 1, "X dust <= burns-1 yocto, got {dust_x}");
    assert!(dust_y <= 1, "Y dust <= burns-1 yocto, got {dust_y}");
    assert_eq!(bal(&m, "tokx.tst", "alice.tst"), FUND - dust_x, "alice X short by exactly the dust");
    assert_eq!(bal(&m, "toky.tst", "alice.tst"), FUND - dust_y, "alice Y short by exactly the dust");
    let vx = m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"LQ:{A}\"}}")));
    assert_eq!(vx, "0", "liquidity fully closed");
    // The dust is stranded in the escrow slices (custody is off-books from
    // the AMM float), matching the books returning to exact seed state.
    assert_eq!(escrow(&m, "ESCX"), dust_x, "X dust parked in escrow");
    assert_eq!(escrow(&m, "ESCY"), dust_y, "Y dust parked in escrow");
    let view = m.ret(&m.pool_op("{\"op\":\"view\"}"));
    assert_eq!(
        view,
        "S:18446744073709551616:P:0:X:0:Y:0:IMIN:0:IMAX:0",
        "books back to seed state - dust lives in escrow, not the float"
    );
}
