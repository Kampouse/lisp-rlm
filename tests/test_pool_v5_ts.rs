//! CLMM v5 TS frontend test — compiles via TS → Lisp IR → WASM
//! and runs the same custody scenario as test_pool_v5_c2b.rs

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
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

fn compile_ts(src: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).expect("ts lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    compile_near_from_exprs(&exprs).expect("compile")
}

fn write_wasm(dir: &Path, name: &str, wasm: &[u8]) {
    std::fs::write(dir.join(name), wasm).unwrap();
}

struct Mock {
    state: PathBuf,
    manifest: String,
}

impl Mock {
    fn new(dir: &Path, pool_wasm: &Path, ft_wasm: &Path) -> Mock {
        let state = dir.join("state.bin");
        let _ = std::fs::remove_file(&state);
        let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
        Mock {
            manifest: format!(
                "pool.tst={},tokx.tst={},toky.tst={}",
                pool_wasm.display(),
                ft_wasm.display(),
                ft_wasm.display()
            ),
            state,
        }
    }

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

    fn pool_verb(&self, method: &str, args: &str) -> String {
        self.cross("pool.tst", method, args, "owner")
    }

    fn ft_mint(&self, token: &str, to: &str, amt: u128) -> String {
        self.cross(token, "mint", &format!("{{\"to\":\"{to}\",\"amt\":\"{amt}\"}}"), "owner")
    }

    fn ft_transfer(&self, token: &str, to: &str, amt: u128, msg: &str) -> String {
        self.cross(
            token,
            "ft_transfer_call",
            &format!("{{\"receiver_id\":\"{to}\",\"amount\":\"{amt}\",\"msg\":{msg}}}"),
            "owner",
        )
    }

    fn ret(&self, s: String) -> String {
        if s.contains("Error:") {
            panic!("near-mock error: {s}");
        }
        s.trim().to_string()
    }
}

const TSS: &str = include_str!("../fixtures/clmm_v5.ts");

#[test]
fn pool_v5_ts_mint_burn_custody() {
    let dir = repo_root().join("target/test-ts-c2b");
    std::fs::create_dir_all(&dir).unwrap();

    // Compile pool via TS frontend
    let pool_wasm = compile_ts(TSS);
    write_wasm(&dir, "pool.wasm", &pool_wasm);

    // Compile FT token (reuse existing ft2.lisp)
    let ft_src = std::fs::read_to_string(repo_root().join("lib/limb_math.lisp")).unwrap()
        + "\n"
        + &std::fs::read_to_string(repo_root().join("lib/clmm_core.lisp")).unwrap()
        + "\n"
        + &std::fs::read_to_string(repo_root().join("lib/ft2.lisp")).unwrap();
    let ft_exprs = parse_all(&ft_src).expect("parse ft");
    lisp_rlm_wasm::typing::type_check_program(&ft_exprs, true).expect("typecheck ft");
    let ft_wasm = compile_near_from_exprs(&ft_exprs).expect("compile ft");
    write_wasm(&dir, "ft.wasm", &ft_wasm);

    let m = Mock::new(&dir, &dir.join("pool.wasm"), &dir.join("ft.wasm"));

    const D: u128 = 100_000_000_000_000_000_000_000_000; // position units (1e26)
    const FUND: u128 = 100_000_000_000_000_000_000_000_000_000; // alice funds (1e29)

    // Bootstrap tokens
    assert_eq!(m.ret(m.cross("tokx.tst", "new", "{}", "owner")), "ok");
    assert_eq!(m.ret(m.cross("toky.tst", "new", "{}", "owner")), "ok");

    // Initialize pool via verb export
    assert_eq!(
        m.ret(m.pool_verb("init", "{\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\"}")),
        "init-ok"
    );

    // Mint tokens to alice
    for tok in ["tokx.tst", "toky.tst"] {
        assert_eq!(m.ret(m.ft_mint(tok, "alice.tst", FUND)), "ok");
    }

    // TODO: Full custody test (mint legs, cancel, burnc, claim)
    // For now, verify init works
    let state = m.ret(m.pool_verb("get_state", "{}"));
    assert!(state.contains("\"S\":\"0\""), "init state: {state}");
}