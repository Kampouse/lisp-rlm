//! View-surface pins: the `escrow` and `state` read-only ops.
//!
//! These ops exist so external auditors (and the gauntlet's chain layer)
//! can reconcile custody WITHOUT poking raw contract storage: `escrow`
//! returns the custody pair in one call (`ESCX:n:ESCY:n`), `state` returns
//! the classic view readout PLUS last-action, custody pair, and the token
//! contracts in charge. Both are ZERO-write. Pinned here on a fresh pool:
//! exact strings + zero-write proof (view unchanged across both ops).
//! Funded-flow parity (escrow op == `get` keys == raw ft_balance_of) is
//! pinned at the end of pool_v5_c2b_mint_burn_custody.
//!
//! Oct 16, 2026 — live-chain probe couldn't answer "who holds what" without
//! raw view_state scraping; the contract now exposes it properly.

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
    let tmp = repo_root().join("target/test-views-src.lisp");
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

#[test]
fn pool_v5_view_ops_fresh_surface() {
    let dir = repo_root().join("target/test-c2b");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );

    let state = dir.join("state-views.bin");
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
    let m = Mock {
        manifest: format!("pool.tst={}", pool_wasm.display()),
        state,
    };

    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );

    // escrow op: exact pair, zeroed on fresh pool
    assert_eq!(m.ret(&m.pool_op("{\"op\":\"escrow\"}")), "ESCX:0:ESCY:0");

    // state op: view readout + LAST + custody + token ids + fee totals
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"state\"}")),
        "S:18446744073709551616:P:0:X:0:Y:0:IMIN:0:IMAX:0:LAST:0:ESCX:0:ESCY:0:TOKX:tokx.tst:TOKY:toky.tst:SHIFT:8:FEEX:0:FEEY:0",
        "state = view + LAST + custody + token ids + fee totals"
    );

    // zero-write proof: view output identical before/after both view ops
    let v0 = m.ret(&m.pool_op("{\"op\":\"view\"}"));
    let _ = m.pool_op("{\"op\":\"escrow\"}");
    let _ = m.pool_op("{\"op\":\"state\"}");
    assert_eq!(
        m.ret(&m.pool_op("{\"op\":\"view\"}")),
        v0,
        "view ops are zero-write"
    );
    assert_eq!(
        v0, "S:18446744073709551616:P:0:X:0:Y:0:IMIN:0:IMAX:0",
        "fresh pool seed state"
    );
}
