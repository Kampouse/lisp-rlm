//! Pool-v5 custody (arc C2a): the real NEP-141 swap chain over near-mock.
//!
//! tokx/toky are real ft2 contracts; the pool takes deposits via
//! ft_on_transfer (msg "seed" float / msg "sw" swap), pays the out-leg via
//! a promise batch, and p5-pay owns reversal when that payout receipt
//! fails (--fail-receipt). Asserts exact balances, books, and escrow
//! ledgers on both the happy path and the forced-failure path.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is the package root (/ = repo root here)
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
    let tmp = repo_root().join("target/test-custody-src.lisp");
    std::fs::write(&tmp, src).unwrap();
    let o = Command::new(bin("near-compile"))
        .arg(&tmp)
        .arg(out)
        .output()
        .expect("run near-compile");
    let all = format!(
        "{}{}",
        normalize_num(&String::from_utf8_lossy(&o.stdout)),
        normalize_num(&String::from_utf8_lossy(&o.stderr))
    );
    assert!(o.status.success() && all.contains("✅"), "compile failed: {all}");
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
            normalize_num(&String::from_utf8_lossy(&out.stdout)),
            normalize_num(&String::from_utf8_lossy(&out.stderr))
        )
    }

    /// like `cross` but forces receipt N to fail (payout-fault testing)
    fn cross_fail(&self, contract: &str, method: &str, args: &str, signer: &str, n: usize) -> String {
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
            normalize_num(&String::from_utf8_lossy(&out.stdout)),
            normalize_num(&String::from_utf8_lossy(&out.stderr))
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
    let out = m.cross(tok, "ft_balance_of", &format!("{{\"account_id\":\"{who}\"}}"), "owner");
    m.ret(&out).replace(',', "").parse().expect("balance int")
}

fn escrow(m: &Mock, k: &str) -> u128 {
    let out = m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}"));
    m.ret(&out).replace(',', "").parse().expect("escrow int")
}

#[test]
fn pool_v5_custody_swap_and_reversal() {
    let dir = repo_root().join("target/test-custody");
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

    const L: u128 = 500_000_000_000_000_000_000_000_000; // position d, ±2 cells
    const FLOAT: u128 = 50_000_000_000_000_000_000_000_000_000; // pool float per side
    const SWAP: u128 = 10_000_000_000_000_000; // alice's intake (1e16)

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
                &format!("{{\"to\":\"pool.tst\",\"amt\":\"{FLOAT}\"}}"),
                "owner"
            )),
            "ok"
        );
    }
    assert_eq!(
        m.ret(&m.cross(
            "tokx.tst",
            "mint",
            &format!("{{\"to\":\"alice.tst\",\"amt\":\"{}\"}}", FLOAT * 2),
            "owner"
        )),
        "ok"
    );
    assert_eq!(
        m.ret(&m.pool_op(&format!(
            "{{\"op\":\"mint\",\"d\":\"{L}\",\"a\":\"-2\",\"b\":\"2\"}}"
        ))),
        format!("mint-ok:{L}")
    );

    // seed the float through the real NEP-141 chain
    for tok in ["tokx.tst", "toky.tst"] {
        let used = m.ret(&m.cross(
            tok,
            "ft_transfer_call",
            &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{FLOAT}\",\"msg\":\"seed\"}}"),
            "pool.tst",
        ));
        assert_eq!(used, format!("used:{FLOAT}"), "seed {tok} fully consumed");
    }
    assert_eq!(escrow(&m, "ESCX"), FLOAT);
    assert_eq!(escrow(&m, "ESCY"), FLOAT);

    // fee-gen: the custody walker runs on v_eff = intake − skim (F =
    // intake >> 8 stays escrowed, LP-claimable). Parity quote stays GROSS,
    // so the differential oracle for the fee walk is quote(v_eff).
    let f: u128 = SWAP / 256;
    let q = m.ret(&m.pool_op(&format!("{{\"op\":\"quote\",\"v\":\"{}\"}}", SWAP - f)));
    let qbody = q.trim_start_matches("q:");
    let (qout_s, qrm_s) = qbody
        .split_once(":rm:")
        .unwrap_or_else(|| panic!("quote shape: {q}"));
    let qout: u128 = qout_s.parse().expect("quote out");
    let qrm: u128 = qrm_s.parse().expect("quote rm");
    assert!(qout > 0, "quote must be nonzero at test scale: {q}");

    // ── happy path: swap through the promise chain ──
    let used = m.ret(&m.cross(
        "tokx.tst",
        "ft_transfer_call",
        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SWAP}\",\"msg\":\"sw\"}}"),
        "alice.tst",
    ));
    let consumed: u128 = used.trim_start_matches("used:").parse().expect("used int");
    // trader pays SWAP - rm (walker remainder), receives out
    assert_eq!(consumed, SWAP - qrm, "consumed == intake - remainder");

    assert_eq!(bal(&m, "toky.tst", "alice.tst"), qout, "payout == quoted out");
    assert_eq!(bal(&m, "tokx.tst", "alice.tst"), FLOAT * 2 - consumed);
    assert_eq!(escrow(&m, "ESCX"), FLOAT + qrm, "ESCX +rm (resolve refund)");
    assert_eq!(escrow(&m, "ESCY"), FLOAT - qout, "ESCY drained by out");

    let view_after = m.ret(&m.pool_op("{\"op\":\"view\"}"));

    // ── fault path: force the payout receipt to fail ──
    let used2 = m.ret(&m.cross_fail(
        "toky.tst",
        "ft_transfer_call",
        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SWAP}\",\"msg\":\"sw\"}}"),
        "alice.tst",
        0,
    ));
    assert_eq!(used2, "used:0", "failed payout ⇒ full refund");

    // state byte-identical: reversal composed with the failed receipt
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"view\"}")),
        view_after,
        "books/P/S reverted on payout failure"
    );
    assert_eq!(escrow(&m, "ESCX"), FLOAT + (SWAP - consumed));
    assert_eq!(escrow(&m, "ESCY"), FLOAT - qout);
    assert_eq!(bal(&m, "toky.tst", "alice.tst"), qout);
}

/// The near-mock's \u{1f4c4} printer comma-groups digit runs (human_num,
/// 7d2b3cb1). Strip the grouping so asserts written against raw digits keep
/// working. Commas inside non-digit contexts are preserved.
fn normalize_num(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b',' {
            let before = out.as_bytes().last().map_or(false, |c| c.is_ascii_digit());
            let after = bytes.get(i + 1).map_or(false, |c| c.is_ascii_digit());
            if before && after {
                i += 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
