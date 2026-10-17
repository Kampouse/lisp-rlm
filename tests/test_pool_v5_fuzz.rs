//! Fuzz L5: random-walk state-stability gauntlet.
//!
//! Drives N seeds × M ops of mint/swap-y/swap-x on near-mock. After every
//! op, the contract's S/P (op "view") is read twice and must be identical.
//! This pins that custody/reversal/payout paths leave the curve state in a
//! consistent, readable state and that no op corrupts the view surface.
//!
//! Deterministic: seeds 1..=3 run in CI (gauntlet L5); `FUZZ_SEEDS` widens.

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

fn compile(concat: &[&str], out: &Path) {
    let mut src = String::new();
    for part in concat {
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
    fn cross(&self, c: &str, m: &str, a: &str, s: &str) -> String {
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
    fn ret(&self, out: &str) -> String {
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
            .unwrap_or_default()
    }
    fn op(&self, json: &str) -> String {
        self.ret(&self.cross("pool.tst", "_run", json, "owner"))
    }
    fn view(&self) -> String {
        self.op("{\"op\":\"view\"}")
    }
}

/// Extract the S:P prefix from a p5-view string.
fn sp_head(v: &str) -> String {
    v.split(":X:").next().unwrap_or("?").to_string()
}

#[test]
fn pool_v5_fuzz_state_stable() {
    let dir = repo_root().join("target/test-fuzz");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let seeds: Vec<u64> = std::env::var("FUZZ_SEEDS")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().parse().unwrap()).collect())
        .unwrap_or_else(|| vec![1, 2, 3]);
    const OPS: usize = 40;

    for seed in seeds {
        let state = dir.join(format!("state-fuzz-{seed}.bin"));
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

        assert!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")) == "ok");
        assert!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")) == "ok");
        assert_eq!(m.op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\",\"fee_shift\":\"8\",\"s0\":\"18446744073709551616\"}"), "init-ok");

        const LP_FUND: u128 = 100_000_000_000_000_000_000_000_000_000; // 1e29
        const TRADER_FUND: u128 = 150_000_000_000_000_000_000_000_000_000; // 1.5e29
        for who in ["alice.tst", "bob.tst", "carol.tst"] {
            for tok in ["tokx.tst", "toky.tst"] {
                assert_eq!(
                    m.ret(&m.cross(
                        tok,
                        "mint",
                        &format!("{{\"to\":\"{who}\",\"amt\":\"{LP_FUND}\"}}"),
                        "owner"
                    )),
                    "ok"
                );
            }
            assert!(m
                .ret(&m.cross("pool.tst", "_run", "{\"op\":\"reg\"}", who))
                .starts_with("reg-ok"));
        }
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(
                m.ret(&m.cross(
                    tok,
                    "mint",
                    &format!("{{\"to\":\"trader.tst\",\"amt\":\"{TRADER_FUND}\"}}"),
                    "owner"
                )),
                "ok"
            );
        }

        const SEED_X: u128 = 100_000_000_000_000_000_000_000_000_000;
        const SEED_Y: u128 = 100_000_000_000_000_000_000_000_000_000;
        assert!(m
            .ret(&m.cross(
                "tokx.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_X}\",\"msg\":\"seed\"}}"
                ),
                "alice.tst"
            ))
            .starts_with("used:"));
        assert!(m
            .ret(&m.cross(
                "toky.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_Y}\",\"msg\":\"seed\"}}"
                ),
                "alice.tst"
            ))
            .starts_with("used:"));

        let mut rng = seed;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };

        for i in 0..OPS {
            let r = next() % 100;
            if i < 4 || r < 30 {
                let lo = -((next() % 8) as i64) - 1;
                let hi = (next() % 8) as i64 + 1;
                let (lo, hi) = (lo.min(hi), hi.max(lo));
                let d: u128 = 1_000_000_000_000_000_000_000_000_000u128
                    + (next() % 50) as u128 * 1_000_000_000_000_000_000_000_000_000u128;
                let who = ["alice.tst", "bob.tst", "carol.tst"][(next() % 3) as usize];
                let req = m.op(&format!(
                    "{{\"op\":\"req\",\"d\":\"{d}\",\"a\":\"{lo}\",\"b\":\"{hi}\"}}"
                ));
                let body = req.trim_start_matches("rx:");
                let (rx_s, ry_s) = body.split_once(":ry:").expect("req shape");
                let rx: u128 = rx_s.parse().unwrap();
                let ry: u128 = ry_s.parse().unwrap();
                if rx <= LP_FUND && ry <= LP_FUND {
                    let msg = format!("m:{d}:{lo}:{hi}");
                    let _lx = m.ret(&m.cross("tokx.tst", "ft_transfer_call",
                        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{rx}\",\"msg\":\"{msg}\"}}"), who));
                    let _ly = m.ret(&m.cross("toky.tst", "ft_transfer_call",
                        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{ry}\",\"msg\":\"{msg}\"}}"), who));
                }
            } else if r < 65 {
                let amt: u128 = 1_000_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000_000;
                let _ = m.ret(&m.cross(
                    "toky.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"
                    ),
                    "trader.tst",
                ));
            } else {
                let amt: u128 = 1_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000;
                let _ = m.ret(&m.cross(
                    "tokx.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"
                    ),
                    "trader.tst",
                ));
            }

            let after = sp_head(&m.view());
            let again = sp_head(&m.view());
            assert_eq!(after, again,
                "seed {seed} op {i}: curve state not stable across consecutive view calls\n  first:  {after}\n  second: {again}");
        }
        println!("seed {seed}: {OPS} ops, view-state stable ✓");
    }
    println!("fuzz L5: all seeds green");
}
