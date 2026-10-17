//! Pool PARAMS + client view ops.
//!
//! Init gains `fee_shift` + `s0` (defaults: 8, 2^64 — legacy pools keep
//! the ÷256 skim and price-1.0 boot exactly; pre-param storage keys absent
//! ⇒ defaults via p5-shf/init fallback). Views: op "owed" {who} →
//! `owed:<x>:<y>`; op "positions" {who} → `pos:<a>:<b>:<d>;…` from the
//! 6|PO: index (double scan, ≤129·65 key reads, documented gas bound).
//!
//! Pinned here:
//!   P1  init accepts fee_shift/s0; init-once rejected; keys land (S, SHIFT)
//!   P2  shift-10 skim: FEEY = intake >> 10 EXACT (≠ default >> 8 pin)
//!   P3  owed view == raw OWX/OWY keys (zero pre-burn, funded post-burnc)
//!   P4  positions view: multi-entry, d after mint, d=0 after burnc
//!   P5  owed/positions are ZERO-write (view identical before/after)
//!   P6  legacy init (no params): S = 2^64, SHIFT = "" (→ skim ÷256 via p5-shf)
//!
//! Market goldens (tests/test_pool_v5_market.rs) already pin the DEFAULT
//! skim at ÷256 — legacy parity is regression-guarded there.

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
    let tmp = repo_root().join("target/test-params-src.lisp");
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

    fn key(&self, k: &str) -> String {
        self.ret(&self.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}")))
    }

    fn key_i128(&self, k: &str) -> i128 {
        self.key(k).parse().unwrap_or(0)
    }
}

/// positions view → Vec<(a, b, d)> (decoupled from decimal formatting)
fn parse_positions(s: &str) -> Vec<(i128, i128, i128)> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(';')
        .map(|e| {
            let t: Vec<&str> = e.trim_start_matches("pos:").split(':').collect();
            assert_eq!(t.len(), 3, "entry shape: {e}");
            (
                t[0].parse().unwrap_or_else(|_| panic!("a parse: {e}")),
                t[1].parse().unwrap_or_else(|_| panic!("b parse: {e}")),
                t[2].parse().unwrap_or_else(|_| panic!("d parse: {e}")),
            )
        })
        .collect()
}

#[test]
fn pool_v5_params_and_views() {
    let dir = repo_root().join("target/test-params");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let state = dir.join("state-params.bin");
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

    // ── boot: custom params — fee_shift 10 (÷1024), s0 = 2^64 (price 1.0) ──
    assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")), "ok");
    assert_eq!(
        m.ret(&m.pool_op(
            "{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\",\"fee_shift\":\"10\",\"s0\":\"18446744073709551616\"}"
        )),
        "init-ok",
        "P1: init with custom params"
    );
    assert_eq!(
        m.ret(&m.pool_op(
            "{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}"
        )),
        "already-initialized",
        "P1: init-once holds with params too"
    );
    assert_eq!(m.key("S"), "18446744073709551616", "P1: s0 landed");
    assert_eq!(m.key("SHIFT"), "10", "P1: fee_shift landed");
    assert!(
        m.ret(&m.pool_op("{\"op\":\"state\"}"))
            .ends_with(":SHIFT:10:FEEX:0:FEEY:0"),
        "P1: state exposes SHIFT"
    );

    // ── fund alice, LP position straddling the boot price ──
    const FUND: u128 = 250_000_000_000_000_000_000_000_000_000;
    const D: u128 = 100_000_000_000_000_000_000_000_000_000_000;
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
    let p = m.key_i128("P");
    // straddle the boot price: [p−1, p+1] — both sides deep so the shift-10
    // swap below actually consumes book (exercises the skim for real)
    let lo = p - 1;
    let hi = p + 1;
    let req = m.ret(&m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{D}\",\"a\":\"{lo}\",\"b\":\"{hi}\"}}"
    )));
    let (rx_s, ry_s) = req
        .trim_start_matches("rx:")
        .split_once(":ry:")
        .unwrap_or_else(|| panic!("req shape: {req}"));
    let rx: u128 = rx_s.parse().expect("rx");
    let ry: u128 = ry_s.parse().expect("ry");
    let leg = |tok: &str, amt: u128| {
        m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"m:{D}:{lo}:{hi}\"}}"
            ),
            "alice.tst",
        )
    };
    assert_eq!(m.ret(&leg("tokx.tst", rx)), format!("used:{rx}"), "leg x");
    assert_eq!(m.ret(&leg("toky.tst", ry)), format!("used:{ry}"), "leg y");

    // second position, adjacent cell — multi-entry proof
    let d2: u128 = 40_000_000_000_000_000_000_000_000_000_000;
    let req2 = m.ret(&m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{d2}\",\"a\":\"{lo}\",\"b\":\"{p}\"}}"
    )));
    let (rx2_s, ry2_s) = req2
        .trim_start_matches("rx:")
        .split_once(":ry:")
        .unwrap_or_else(|| panic!("req2 shape: {req2}"));
    let rx2: u128 = rx2_s.parse().expect("rx2");
    let ry2: u128 = ry2_s.parse().expect("ry2");
    let leg2 = |tok: &str, amt: u128| {
        m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"m:{d2}:{lo}:{p}\"}}"
            ),
            "alice.tst",
        )
    };
    assert_eq!(m.ret(&leg2("tokx.tst", rx2)), format!("used:{rx2}"), "leg2 x");
    assert_eq!(m.ret(&leg2("toky.tst", ry2)), format!("used:{ry2}"), "leg2 y");

    // ── P2: shift-10 skim — Y-sell of 1.024e29 ⇒ FEEY = 1e26 EXACT
    //      (default ÷256 would skim 4e26 — this pin fails on default) ──
    let v0 = m.ret(&m.pool_op("{\"op\":\"view\"}"));
    let amt: u128 = 102_400_000_000_000_000_000_000_000_000; // 1.024e29 — big enough to fill (1e32 book)
    let out = m.cross(
        "toky.tst",
        "ft_transfer_call",
        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"),
        "alice.tst",
    );
    assert!(m.ret(&out).starts_with("used:"), "swap leg executed");
    assert_eq!(
        m.key_i128("FEEY"),
        100_000_000_000_000_000_000_000_000,
        "P2: FEEY = intake >> 10 (shift is the pool PARAMETER, not baked 8)"
    );
    // ── P5 zero-write proof for the two view ops (view stable across) ──
    let before = m.ret(&m.pool_op("{\"op\":\"view\"}"));
    let _ = m.ret(&m.pool_op(&format!("{{\"op\":\"owed\",\"who\":\"alice.tst\"}}")));
    let _ = m.ret(&m.pool_op(&format!("{{\"op\":\"positions\",\"who\":\"alice.tst\"}}")));
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"view\"}")),
        before,
        "P5: owed/positions are ZERO-write"
    );

    // ── P3: owed view zero pre-burn ──
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"owed\",\"who\":\"alice.tst\"}}"))),
        "owed:0:0",
        "P3: no owed before burn"
    );

    // ── P4: positions view pre-burn: both entries live ──
    let pos = m.ret(&m.pool_op(&format!("{{\"op\":\"positions\",\"who\":\"alice.tst\"}}")));
    let want_d1: i128 = m.key_i128(&format!("POS:alice.tst:{lo}:{hi}"));
    let want_d2: i128 = m.key_i128(&format!("POS:alice.tst:{lo}:{p}"));
    assert_eq!(
        parse_positions(&pos),
        vec![(lo, p, want_d2), (lo, hi, want_d1)],
        "P4: two live entries, a-then-b order"
    );
    // nobody else: empty
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"positions\",\"who\":\"nobody.tst\"}")),
        "",
        "P4: unknown owner ⇒ empty"
    );

    // ── burn position 1 via burnc (custody burn → owed receipt);
    //    authority: caller must own the position — sign as alice ──
    let bc = m.ret(&m.cross(
        "pool.tst",
        "_run",
        &format!("{{\"op\":\"burnc\",\"d\":\"{D}\",\"a\":\"{lo}\",\"b\":\"{hi}\"}}"),
        "alice.tst",
    ));
    assert!(bc.starts_with("burnc-ok"), "burnc executed: {bc}");

    // ── P3: owed view == raw OW keys post-burn ──
    let owx = m.key_i128(&format!("OWX:alice.tst"));
    let owy = m.key_i128(&format!("OWY:alice.tst"));
    assert!(owx > 0 && owy > 0, "P3: burnc wrote owed receipts");
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"owed\",\"who\":\"alice.tst\"}}"))),
        format!("owed:{owx}:{owy}"),
        "P3: owed view == raw OW keys"
    );

    // ── P4: post-burn: fully-burned band drops out (poskey → "0",
    //    indistinguishable from absent — d≠0 filter); live band remains.
    //    Partial burns would show reduced d. ──
    assert_eq!(
        parse_positions(
            &m.ret(&m.pool_op(&format!("{{\"op\":\"positions\",\"who\":\"alice.tst\"}}")))
        ),
        vec![(lo, p, want_d2)],
        "P4: burned band drops out, live band remains"
    );

    // ── reg gate (claims are gated): unregistered ⇒ refused, owed intact ──
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"claim\",\"t\":\"x\"}")),
        "not-registered",
        "unregistered claimer refused (owner account never registered)"
    );
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"owed\",\"who\":\"alice.tst\"}}"))),
        format!("owed:{owx}:{owy}"),
        "refusal left owed untouched"
    );
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"reg\"}", "alice.tst")),
        "reg-ok:alice.tst",
        "self-registration"
    );
    // double reg is a no-op (idempotent)
    assert_eq!(
        m.ret(&m.cross("pool.tst", "_run", "{\"op\":\"reg\"}", "alice.tst")),
        "reg-ok:alice.tst"
    );

    // ── claim pays out; owed view returns to zero ──
    let cx = m.ret(&m.cross(
        "pool.tst",
        "_run",
        "{\"op\":\"claim\",\"t\":\"x\"}",
        "alice.tst",
    ));
    assert_eq!(cx, "0", "claim x consumed the receipt");
    let cy = m.ret(&m.cross(
        "pool.tst",
        "_run",
        "{\"op\":\"claim\",\"t\":\"Y\"}",
        "alice.tst",
    ));
    assert_eq!(cy, "0", "claim y (uppercase) consumed the receipt");
    assert_eq!(
        m.ret(&m.pool_op(&format!("{{\"op\":\"owed\",\"who\":\"alice.tst\"}}"))),
        "owed:0:0",
        "P3: owed drained after claim receipts"
    );
}

#[test]
fn pool_v5_legacy_defaults_untouched() {
    let dir = repo_root().join("target/test-params");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );

    let state = dir.join("state-legacy.bin");
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
    let m = Mock {
        manifest: format!("pool.tst={}", pool_wasm.display()),
        state,
    };

    // P6: bare init (the pre-param call shape) ⇒ S = 2^64, SHIFT unset
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );
    assert_eq!(
        m.key("S"),
        "18446744073709551616",
        "P6: s0 default = 2^64 (price 1.0)"
    );
    assert_eq!(m.key("SHIFT"), "", "P6: SHIFT unset — p5-shf falls back to 8");
    assert!(
        m.ret(&m.pool_op("{\"op\":\"state\"}"))
            .ends_with(":SHIFT:8:FEEX:0:FEEY:0"),
        "P6: state reports the EFFECTIVE shift (8) for legacy pools"
    );
}
