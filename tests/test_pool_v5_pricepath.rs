//! Price-path sim: many small alternating swaps through the real NEP-141
//! swap path, printing S (spot) after each — output is charted externally.
//! Run: cargo test --release --test test_pool_v5_pricepath -- --nocapture

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bin(name: &str) -> PathBuf {
    let p = repo_root().join("target/release").join(name);
    assert!(p.exists(), "{name} missing — build with `cargo build --release` first");
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
    let o = Command::new(bin("near-compile")).arg(&tmp).arg(out).output().expect("run near-compile");
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
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
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    }

    fn pool_op(&self, json: &str) -> String {
        self.cross("pool.tst", "_run", json, "owner")
    }

    fn ret(&self, out: &str) -> String {
        out.lines()
            .rev()
            .find(|l| l.contains("📄"))
            .map(|l| {
                l.split('📄').nth(1).unwrap_or("").trim()
                    .split_whitespace().next().unwrap_or("")
                    .replace(',', "")
            })
            .unwrap_or_default()
    }
}

fn key_i128(m: &Mock, k: &str) -> i128 {
    let out = m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}"));
    m.ret(&out).parse().expect("int")
}

#[test]
fn pool_v5_price_path_sim() {
    let dir = repo_root().join("target/test-market");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        ["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"].as_slice(),
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let state = dir.join("state_path.bin");
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
    const FUND: u128 = 250_000_000_000_000_000_000_000_000_000; // 2.5e29
    const LO: i64 = -11;
    const HI: i64 = 11;

    println!("== boot ==");
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

    let req_raw = m.pool_op(&format!(
        "{{\"op\":\"req\",\"d\":\"{D}\",\"a\":\"{LO}\",\"b\":\"{HI}\"}}"
    ));
    eprintln!("REQ_RAW: {}", &req_raw);
    let req_out = m.ret(&req_raw);
    eprintln!("REQ_OUT: {:?}", &req_out); let req_body = req_out.trim_start_matches("rx:");
    let (rx_s, ry_s) = req_body.split_once(":ry:").unwrap();
    let rx: u128 = rx_s.parse().unwrap();
    let ry: u128 = ry_s.parse().unwrap();
    let mint_msg = format!("m:{D}:{LO}:{HI}");
    for (tok, amt) in [("tokx.tst", rx), ("toky.tst", ry)] {
        let r = m.ret(&m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{mint_msg}\"}}"
            ),
            "alice.tst",
        ));
        assert_eq!(r, format!("used:{amt}"), "deposit {tok}");
    }

    let s = || key_i128(&m, "S");
    println!("POINT,0,{},boot,0", s());

    // deterministic pseudo-random walk (reproducible, no RNG dep):
    // LCG for sizes, alternating-ish bias flips every ~7 legs so the
    // price trends up, then down, then mean-reverts.
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut buy_bias = true;
    let n_legs = 80;
    for i in 0..n_legs {
        if i > 0 && i % 20 == 0 {
            buy_bias = !buy_bias; // flip regime: up-trend → down-trend → up → down
        }
        let buy_x = if (i % 7) < 5 { buy_bias } else { !buy_bias }; // mostly trend, some noise
        let amt = (20_000_000 + (next() % 180_000_000)) as u128 * 1_000_000_000_000_000_000; // scale: 2e25–2e26
        let tok = if buy_x { "tokx.tst" } else { "toky.tst" };
        let out = m.cross(
            tok,
            "ft_transfer_call",
            &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"
            ),
            "bob.tst",
        );
        let ret = m.ret(&out);
        assert!(out.contains("used:") || ret.starts_with("used:"), "leg {i}: {out}");
        println!("POINT,{},{},{},{},ret={}", i + 1, s(), if buy_x { "buyX" } else { "sellX" }, amt, &ret);
    }

    println!("== end: S={} (boot 18446744073709551616), P={} ==", s(), key_i128(&m, "P"));
}
