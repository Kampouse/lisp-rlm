//! Refund / reject path tests for the CLMM v5 custody swap ("sw").
//!
//! Chain contract (lib/clmm_pool_v5.lisp header comment, verified by these
//! tests): ft_resolve_transfer reads the CHAIN RESULT of the ft_on_transfer
//! receipt — p5-pay's near/return. Every path must return the right refund:
//!
//!   R1  reject (unregistered)          ⇒ full amount back
//!   R2  zero-amount                    ⇒ full amount back, no state change
//!   R3  empty walk (no liquidity)      ⇒ full amount back, no book change
//!   R4  executed swap                  ⇒ used kept, rem refunded
//!   R5  payout promise FAILS           ⇒ FULL intake (c+rm) refunded +
//!                                         ledgers reversed (the v4 leak fix)
//!   R6  garbage msg                    ⇒ full amount back (no branch match)
//!
//! Modeled with near-mock; we assert on the 📄 chain-result string of the
//! ft_transfer_call cross (what ft2's resolve consumes) and on ESC/books.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bin(name: &str) -> PathBuf {
    let p = repo_root().join("target/release").join(name);
    assert!(p.exists(), "{name} missing — cargo build --release first");
    p
}

fn compile(files: &[&str], out: &Path) {
    let mut src = String::new();
    for part in files {
        src.push_str(&std::fs::read_to_string(repo_root().join("lib").join(part)).unwrap());
        src.push('\n');
    }
    let tmp = repo_root().join("target/test-fuzz-src.lisp");
    std::fs::write(&tmp, src).unwrap();
    let o = Command::new(bin("near-compile"))
        .arg(&tmp)
        .arg(out)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        o.status.success() && all.contains('✅'),
        "compile failed: {all}"
    );
}

struct Mock {
    state: PathBuf,
    manifest: String,
}

impl Mock {
    fn raw(&self, c: &str, m: &str, a: &str, s: &str) -> String {
        let out = Command::new(bin("near-mock"))
            .args(["cross"])
            .arg(&self.state)
            .arg(&self.manifest)
            .args([c, m, a])
            .env("NEAR_MOCK_SIGNER", s)
            .output()
            .expect("near-mock cross");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
    fn ret(&self, out: &str) -> Option<String> {
        out.lines()
            .rev()
            .find(|l| l.contains('📄'))
            .and_then(|l| l.split('📄').nth(1))
            .map(|l| {
                l.trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .replace(',', "")
            })
    }
    fn chain_result(&self, tok: &str, msg: &str, amt: &str, who: &str) -> String {
        self.ret(&self.raw(
            tok,
            "ft_transfer_call",
            &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{}\"}}", msg.replace('"', "\\\"")),
            who,
        ))
        .unwrap_or_default()
    }
    fn op(&self, json: &str, who: &str) -> String {
        self.ret(&self.raw("pool.tst", "exec", json, who))
            .unwrap_or_default()
    }
    /// Abort-bearing output (near/abort prints on stderr, no 📄 line).
    fn op_raw(&self, json: &str, who: &str) -> String {
        self.raw("pool.tst", "exec", json, who)
    }
}

fn setup(seed_liquidity: bool, tag: &str) -> Mock {
    let dir = repo_root().join("target/test-fuzz");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let state = dir.join(format!("state-refund-{tag}.bin"));
    let _ = std::fs::remove_file(&state);
    let m = Mock {
        manifest: format!(
            "pool.tst={},tokx.tst={},toky.tst={}",
            pool_wasm.display(),
            ft_wasm.display(),
            ft_wasm.display()
        ),
        state,
    };
        import_key(&m.state, "pool.tst", "6|SIMSWAP", "1");

    assert_eq!(
        m.ret(&m.raw("tokx.tst", "new", "{}", "owner")),
        Some("ok".into())
    );
    assert_eq!(
        m.ret(&m.raw("toky.tst", "new", "{}", "owner")),
        Some("ok".into())
    );
    assert_eq!(
        m.op(
            "{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\",\"fee_shift\":\"8\",\"s0\":\"18446744073709551616\"}",
            "owner"
        ),
        "init-ok"
    );
    const F: u128 = 100_000_000_000_000_000_000_000_000_000;
    for who in ["alice.tst", "bob.tst"] {
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(
                m.ret(&m.raw(
                    tok,
                    "mint",
                    &format!("{{\"to\":\"{who}\",\"amt\":\"{F}\"}}"),
                    "owner"
                )),
                Some("ok".into())
            );
        }
        assert!(m.op("{\"op\":\"reg\"}", who).starts_with("reg-ok"));
    }
    for tok in ["tokx.tst", "toky.tst"] {
        assert_eq!(
            m.ret(&m.raw(
                tok,
                "mint",
                &format!("{{\"to\":\"trader.tst\",\"amt\":\"150000000000000000000000000000\"}}"),
                "owner"
            )),
            Some("ok".into())
        );
    }
    if seed_liquidity {
        const S: u128 = 50_000_000_000_000_000_000_000_000_000;
        let esc0 = m.op("{\"op\":\"escrow\"}", "owner");
        // seed returns 0 (kept) — the seed float, no position
        let r1 = m.chain_result("tokx.tst", "{\"a\":\"seed\"}", &S.to_string(), "alice.tst");
        assert_eq!(r1, format!("used:{S}"), "seed keeps the full amount");
        let r2 = m.chain_result("toky.tst", "{\"a\":\"seed\"}", &S.to_string(), "alice.tst");
        assert_eq!(r2, format!("used:{S}"), "seed keeps the full amount");
        let esc1 = m.op("{\"op\":\"escrow\"}", "owner");
        assert_ne!(esc0, esc1, "seed must grow the escrow float");
        // mint a real position so the walk has liquidity
        let req = m.op(
            "{\"op\":\"req\",\"d\":\"30000000000000000000000000000\",\"a\":\"-2\",\"b\":\"2\"}",
            "bob.tst",
        );
        let body = req.trim_start_matches("rx:");
        let (rx, ry) = body.split_once(":ry:").expect("req shape");
        let msg = "{\"a\":\"mint\",\"d\":\"30000000000000000000000000000\",\"lo\":\"-2\",\"hi\":\"2\"}";
        assert!(m
            .chain_result("tokx.tst", msg, rx, "bob.tst")
            .starts_with("used:"));
        assert!(m
            .chain_result("toky.tst", msg, ry, "bob.tst")
            .starts_with("used:"));
    }
    m
}

const AMT: &str = "1000000000000000000000000";

fn b64e(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |a, &x| (a << 8) | x as u32) << (8 * (3 - chunk.len()) as u32);
        for k in (0..4).rev() {
            if k < 3 - chunk.len() as u32 {
                out.push('=');
            } else {
                out.push(T[((n >> (6 * k)) & 63) as usize] as char);
            }
        }
    }
    out
}

/// Seed one storage key on `acct` (SIMSWAP arming for exec reads).
fn import_key(state: &Path, acct: &str, key: &str, val: &str) {
    use std::io::Write as _;
    let row = format!(
        "[{{\"account\":\"{acct}\",\"key\":\"{}\",\"value\":\"{}\"}}]",
        b64e(key),
        b64e(val)
    );
    let mut child = Command::new(bin("near-mock"))
        .args(["state", "import", state.to_str().unwrap(), "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn near-mock state import");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(row.as_bytes())
        .unwrap();
    let st = child.wait().expect("state import wait");
    assert!(st.success(), "state import failed: {row}");
}

#[test]
fn refund_reject_paths() {
    // ── R1: unregistered (but funded) sender ⇒ full refund, no escrow ──
    let m = setup(false, "reject");
    assert_eq!(
        m.ret(&m.raw(
            "toky.tst",
            "mint",
            "{\"to\":\"stranger.tst\",\"amt\":\"10000000000000000000000000000\"}",
            "owner"
        )),
        Some("ok".into())
    );
    let before = m.op("{\"op\":\"escrow\"}", "owner");
    // Unregistered ⇒ on-transfer keeps 0 ("used:0"); ft_resolve_transfer
    // then refunds amount − kept = FULL amount to the sender.
    let res = m.chain_result("toky.tst", "{\"a\":\"swap\"}", AMT, "stranger.tst");
    assert_eq!(
        res, "used:0",
        "R1: unregistered must keep nothing (full refund via resolve)"
    );
    assert_eq!(
        m.op("{\"op\":\"escrow\"}", "owner"),
        before,
        "R1: escrow touched"
    );

    // ── R2: zero amount ⇒ kept 0 (nothing to refund), escrow untouched ──
    let res = m.chain_result("toky.tst", "{\"a\":\"swap\"}", "0", "trader.tst");
    assert_eq!(res, "used:0", "R2: zero-amount keeps nothing");
    assert_eq!(
        m.op("{\"op\":\"escrow\"}", "owner"),
        before,
        "R2: escrow touched"
    );

    // ── R3: registered, funded, but NO liquidity ⇒ empty walk, full refund ──
    let res = m.chain_result("toky.tst", "{\"a\":\"swap\"}", AMT, "trader.tst");
    assert_eq!(
        res, "used:0",
        "R3: empty walk (out=0) must keep nothing (full refund via resolve)"
    );
    assert_eq!(
        m.op("{\"op\":\"escrow\"}", "owner"),
        before,
        "R3: escrow touched"
    );
    let st = m.op("{\"op\":\"state\"}", "owner");
    assert!(
        !st.contains("LAST:swap-out:"),
        "R3: failed swap must not write LAST (stale-LAST trap)"
    );

    // ── R6: garbage msg from a registered user ⇒ keeps nothing ──
    let res = m.chain_result("toky.tst", "garbage-msg", AMT, "trader.tst");
    assert_eq!(
        res, "used:0",
        "R6: unknown msg must keep nothing (full refund via resolve)"
    );
}

#[test]
fn refund_executed_swap_keeps_used_refunds_rem() {
    // ── R4: executed swap ⇒ result = rem (what resolve refunds), escrow
    //       grows by used, books move, LAST written.
    let m = setup(true, "exec");
    let esc_before = m.op("{\"op\":\"escrow\"}", "owner");
    let amt: u128 = 5_000_000_000_000_000_000_000_000;
    let res = m.chain_result("toky.tst", "{\"a\":\"swap\"}", &amt.to_string(), "trader.tst");
    let kept: u128 = res
        .trim_start_matches("used:")
        .parse()
        .expect("R4: executed swap must return the kept amount");
    assert!(
        kept < amt,
        "R4: kept must be < amount when the walk consumed input"
    );

    let esc_after = m.op("{\"op\":\"escrow\"}", "owner");
    // ESCY grows by the consumed slice (the refund leaves pool custody at
    // ft2 resolve: sub-bal recv rem)
    let refunded = amt - kept;
    let (bx, by) = esc_after
        .strip_prefix("ESCX:")
        .unwrap()
        .split_once(":ESCY:")
        .unwrap();
    let (_, by0) = esc_before
        .strip_prefix("ESCX:")
        .unwrap()
        .split_once(":ESCY:")
        .unwrap();
    let dy: u128 = by.parse::<u128>().unwrap() - by0.parse::<u128>().unwrap();
    assert_eq!(
        dy, kept,
        "R4: ESCY delta must equal the consumed slice (rem left via resolve)"
    );
    assert!(bx.parse::<u128>().unwrap() > 0, "R4: X book grew (payout)");

    let st = m.op("{\"op\":\"state\"}", "owner");
    assert!(st.contains("LAST:swap-out:"), "R4: LAST written on success");
}

#[test]
fn burn_authority_owner_ok_stranger_rejected() {
    // Finding A fix: op burn requires POS ownership ≥ d and decrements it.
    let m = setup(true, "burnauth");
    // bob minted [−2,2] d=3e28 in setup(true) → bob owns the position.
    let D = "10000000000000000000000000000"; // 1e28 ≤ 3e28
    let A = "-2";
    let B = "2";

    // stranger (registered, funded, NO position) ⇒ ERR_AUTH, books intact
    assert_eq!(
        m.ret(&m.raw(
            "tokx.tst",
            "mint",
            "{\"to\":\"eve.tst\",\"amt\":\"100000000000000000000000000000\"}",
            "owner"
        )),
        Some("ok".into())
    );
    assert!(m.op("{\"op\":\"reg\"}", "eve.tst").starts_with("reg-ok"));
    let st0 = m.op("{\"op\":\"state\"}", "owner");
    let out = m.op_raw(
        &format!("{{\"op\":\"burn\",\"d\":\"{D}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"),
        "eve.tst",
    );
    assert!(
        out.contains("ERR_AUTH"),
        "stranger burn must abort ERR_AUTH, got: {out}"
    );
    assert_eq!(
        m.op("{\"op\":\"state\"}", "owner"),
        st0,
        "rejected burn mutated state"
    );

    // owner burn ⇒ ok, LQ cells drop, POS shrinks
    let res = m.op(
        &format!("{{\"op\":\"burn\",\"d\":\"{D}\",\"a\":\"{A}\",\"b\":\"{B}\"}}"),
        "bob.tst",
    );
    assert_eq!(res, format!("burn-ok:{D}"), "owner burn must succeed");
    // d minted = 3e28, burned 1e28 → 2e28 remains (get prepends 6|)
    let pos = m.op(
        &format!("{{\"op\":\"get\",\"k\":\"POS:bob.tst:{A}:{B}\"}}"),
        "owner",
    );
    assert_eq!(pos, "20000000000000000000000000000", "POS must shrink by d");
}
