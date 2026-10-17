//! Full LP market cycle through the real NEP-141 promise path:
//!
//!   phase 1: LP (alice) mints position A [-8,8]; trader (bob) sells X hard,
//!            price pins at the lower lip (swaps past-lip refund whole)
//!   phase 2: alice burns A and claims owed X/Y from escrow
//!   phase 3: alice re-mints position B re-centered lower: [-16, 0]
//!   phase 4: bob buys (sells Y) — price recovers inside B's range
//!
//! Prints POINT,phase,leg,S,detail per step. Chart externally.
//! Run: cargo test --release --test test_pool_v5_lp_cycle -- --nocapture

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
        src.push_str(
            &std::fs::read_to_string(repo_root().join("lib").join(part))
                .unwrap_or_else(|e| panic!("read lib/{part}: {e}")),
        );
        src.push('\n');
    }
    let tmp = repo_root().join("target/test-lpcycle-src.lisp");
    std::fs::write(&tmp, src).unwrap();
    let o = Command::new(bin("near-compile")).arg(&tmp).arg(out).output().expect("near-compile");
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
            .arg("cross").arg(&self.state).arg(&self.manifest)
            .arg(contract).arg(method).arg(args)
            .arg("--prepaid").arg("2000")
            .env("NEAR_MOCK_SIGNER", signer)
            .output().expect("near-mock cross");
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    }
    fn pool_op(&self, json: &str) -> String {
        self.cross("pool.tst", "_run", json, "owner")
    }
    fn pool_op_as(&self, json: &str, signer: &str) -> String {
        self.cross("pool.tst", "_run", json, signer)
    }
    fn ret(&self, out: &str) -> String {
        out.lines().rev()
            .find(|l| l.contains("📄"))
            .map(|l| l.split('📄').nth(1).unwrap_or("").trim()
                .split_whitespace().next().unwrap_or("").replace(',', ""))
            .unwrap_or_default()
    }
}

fn key_i128(m: &Mock, k: &str) -> i128 {
    m.ret(&m.pool_op(&format!("{{\"op\":\"get\",\"k\":\"{k}\"}}"))).parse().expect("int")
}

const D: u128 = 100_000_000_000_000_000_000_000_000_000_000; // 1e32 LP units
const FUND: u128 = 250_000_000_000_000_000_000_000_000_000; // 2.5e29

#[test]
fn pool_v5_lp_cycle() {
    let dir = repo_root().join("target/test-lpcycle");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"].as_slice(), &pool_wasm);
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let state = dir.join("state_cycle.bin");
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
    let m = Mock {
        manifest: format!("pool.tst={},tokx.tst={},toky.tst={}", pool_wasm.display(), ft_wasm.display(), ft_wasm.display()),
        state,
    };

    println!("== boot ==");
    assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(&m.pool_op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")), "init-ok");
    for who in ["alice.tst", "bob.tst"] {
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(m.ret(&m.cross(tok, "mint", &format!("{{\"to\":\"{who}\",\"amt\":\"{FUND}\"}}"), "owner")), "ok");
        }
    }

    let s = || key_i128(&m, "S");

    // mint position [a,b] for alice through the real two-leg escrow path
    let mint = |m: &Mock, a: i64, b: i64| {
        let req = m.ret(&m.pool_op(&format!("{{\"op\":\"req\",\"d\":\"{D}\",\"a\":\"{a}\",\"b\":\"{b}\"}}")));
        let body = req.trim_start_matches("rx:");
        let (rx_s, ry_s) = body.split_once(":ry:").unwrap();
        let rx: u128 = rx_s.parse().unwrap();
        let ry: u128 = ry_s.parse().unwrap();
        let msg = format!("m:{D}:{a}:{b}");
        for (tok, amt) in [("tokx.tst", rx), ("toky.tst", ry)] {
            let r = m.ret(&m.cross(tok, "ft_transfer_call", &format!(
                "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{msg}\"}}"), "alice.tst"));
            assert_eq!(r, format!("used:{amt}"), "mint leg {tok} [{a},{b}]");
        }
        println!("MINT,[{a},{b}],rx={rx},ry={ry}");
    };

    // burn + claim (burnc is a pool op; claim is a pool op, one per token)
    let burn = |m: &Mock, a: i64, b: i64| {
        let r = m.ret(&m.pool_op(&format!("{{\"op\":\"burnc\",\"d\":\"{D}\",\"a\":\"{a}\",\"b\":\"{b}\"}}")));
        println!("BURN,[{a},{b}],ret={r}");
        for tok in ["tokx", "toky"] {
            let r = m.ret(&m.pool_op(&format!("{{\"op\":\"claim\",\"t\":\"{tok}.tst\"}}")));
            println!("CLAIM,{tok},ret={r}");
        }
    };

    // bob sells amt of `tok` into the pool; true if fully used
    let swap = |m: &Mock, tok: &str, amt: u128| -> bool {
        let r = m.ret(&m.cross(tok, "ft_transfer_call", &format!(
            "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"), "bob.tst"));
        r == format!("used:{amt}")
    };

    // ---------- phase 1: position A [-8,8]; price boots at the LOWER lip
    // (req is nearly all Y). Buy up to the top lip, then ride the whole
    // range back down.
    mint(&m, -40, 40);
    println!("POINT,1,0,{},mintA", s());
    let s0 = s();
    let mut hit = false;
    for leg in 1..=60 {
        let amt = 500_000_000_000_000_000_000_000_000u128; // 5e26 buyX (send X, price up)
        if !swap(&m, "tokx.tst", amt) {
            println!("POINT,1,{leg},{},LIP-top-buy-refund", s());
            hit = true;
            break;
        }
        println!("POINT,1,{leg},{},buyX", s());
    }
    assert!(hit, "phase 1a never hit the top lip");
    let s_top = s();
    hit = false;
    for leg in 1..=300 {
        let amt = 500_000_000_000_000_000_000_000_000u128; // 5e26 sellX (send Y, price down)
        if !swap(&m, "toky.tst", amt) {
            println!("POINT,1b,{leg},{},LIP-bottom-sell-refund", s());
            hit = true;
            break;
        }
        println!("POINT,1b,{leg},{},sellX", s());
    }
    assert!(hit, "phase 1b never crossed back to the bottom lip");
    let s_low = s();
    assert!(s_low < s_top, "price did not fall: {s_top} -> {s_low}");

    // ---------- phase 2: LP burns A and claims ----------
    burn(&m, -8, 8);

    // ---------- phase 3: re-mint re-centered lower ----------
    mint(&m, -80, 0);
    println!("POINT,3,0,{},mintB", s());

    // ---------- phase 4: buy pressure recovers price ----------
    let mut recovered = false;
    for leg in 1..=300 {
        let amt = 500_000_000_000_000_000_000_000_000u128; // buyX: price up
        if !swap(&m, "tokx.tst", amt) {
            println!("POINT,4,{leg},{},LIP-buy-refund", s());
            break;
        }
        println!("POINT,4,{leg},{},buyX", s());
        if s() > s_low + (s0 - s_low) / 2 {
            recovered = true;
            println!("POINT,4,{leg},{},recovered>50%", s());
            break;
        }
    }
    println!("DONE s0={s0} s_low={s_low} s_end={} recovered={recovered}", s());
}
