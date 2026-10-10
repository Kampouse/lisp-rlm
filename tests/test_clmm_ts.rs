//! CLMM TS twin (examples/clmm.ts) — differential e2e vs the lisp original
//! (examples/clmm.lmm → examples/clmm.lisp).
//!
//! Both contracts compile through the SAME backend (TS → lisp IR → wasm vs
//! lisp → wasm) and run the SAME scenario passes through near-mock. Every
//! return value below is machine-verified Q32 arithmetic (python3 floor
//! semantics: muldiv = ⌊a·b/c⌋). Any divergence is a TS-frontend bug.
//!
//! View-name mapping: the lisp file exports tick_net/tick_gross/sp_at_tick
//! as views; the TS dialect auto-views via the get_ prefix, so the twin
//! exports get_tick_net/get_tick_gross/get_sp_at_tick — mapped here.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::process::Command;

fn has_near_mock() -> bool {
    Command::new("./target/release/near-mock")
        .args(["--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn compile_ts(src: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).expect("ts lowering");
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    compile_near_from_exprs(&exprs).expect("compile")
}

fn compile_lisp(src: &str) -> Vec<u8> {
    let exprs = parse_all(src).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    compile_near_from_exprs(&exprs).expect("compile")
}

/// One scenario pass. `which` names the wasm in the manifest (clmm.wasm).
fn pass(dir: &std::path::Path, name: &str, steps: &str) -> (bool, String) {
    let manifest = format!("clmm.c.test.near={}", dir.join("clmm.wasm").display());
    let scenario = dir.join(format!("sc_{name}.json"));
    std::fs::write(
        &scenario,
        format!(
            r#"{{"name": "{}", "manifest": "{}", "steps": {}}}"#,
            name,
            manifest.replace('\\', "/"),
            steps
        ),
    )
    .unwrap();
    let out = Command::new("./target/release/near-mock")
        .args(["scenario"])
        .arg(&scenario)
        .output()
        .expect("near-mock should run");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// One language variant: ONE dir, ONE wasm — scenario passes chain state
/// through the shared state.bin (twap-harness convention).
struct Runner {
    dir: std::path::PathBuf,
}

impl Runner {
    fn new(name: &str, wasm: Vec<u8>) -> Runner {
        let dir = std::env::temp_dir().join(format!("clmm_ts_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clmm.wasm"), wasm).unwrap();
        std::fs::write(dir.join("state.bin"), b"").unwrap();
        Runner { dir }
    }
    fn pass(&self, name: &str, steps: &str) -> (bool, String) {
        pass(&self.dir, name, steps)
    }
}

const LIS: &str = include_str!("../examples/clmm.lisp");
const TSS: &str = include_str!("../examples/clmm.ts");

// ── shared scenario steps (identical for both languages) ──

const INIT_AND_ADD: &str = r#"[
    {"method": "initialize", "args": {"sp": 4294967296, "tick": 0}},
    {"method": "add_liquidity", "args": {"lower": -100, "upper": 100, "liq": 1000000}},
    {"method": "get_price"},
    {"method": "get_liq"},
    {"method": "get_tick"}
  ]"#;

// swap0 dx=1e12: denom=1000001000000, new_sp=4294, dy=999999 (python-verified)
const SWAP0: &str = r#"[
    {"method": "swap0", "args": {"dx": 1000000000000}},
    {"method": "get_price"}
  ]"#;

// swap1 dy_in=5e5 (from sp=4294): dp=2147483648, new_sp=2147487942,
// dx_out=1000223266888 — every muldiv intermediate stays under the
// tagged ceiling 2^60 (muldiv hard-errors past it, by design; the
// formula's Q32 domain is a documented contract property)
const SWAP1: &str = r#"[
    {"method": "swap1", "args": {"dy_in": 500000}},
    {"method": "get_price"}
  ]"#;

// ── per-language step variants (view-name mapping zone) ──
// lisp exports: tick_net/tick_gross/sp_at_tick ; ts exports the get_* twins.

const REMOVE_LIS: &str = r#"[
    {"method": "remove_liquidity", "args": {"lower": -100, "upper": 100, "liq": 400000}},
    {"method": "get_liq"},
    {"method": "sp_at_tick", "args": {"tick": 0}},
    {"method": "sp_at_tick", "args": {"tick": 1}}
  ]"#;

const REMOVE_TSS: &str = r#"[
    {"method": "remove_liquidity", "args": {"lower": -100, "upper": 100, "liq": 400000}},
    {"method": "get_liq"},
    {"method": "get_sp_at_tick", "args": {"tick": 0}},
    {"method": "get_sp_at_tick", "args": {"tick": 1}}
  ]"#;

// ── the differential: same passes, same expectations, both languages ──

#[test]
fn clmm_ts_differential_lifecycle() {
    if !has_near_mock() {
        return;
    }
    let l = Runner::new("life_l", compile_lisp(LIS));
    let t = Runner::new("life_t", compile_ts(TSS));

    // Pass 1 — initialize + add_liquidity: liq=1000000, price=Q32, tick=0.
    let (ok, out) = l.pass("init", INIT_AND_ADD);
    assert!(ok, "lisp init failed:\n{out}");
    let (ok, out_t) = t.pass("init", INIT_AND_ADD);
    assert!(ok, "ts init failed:\n{out_t}");
    for expect in ["1000000", "4294967296"] {
        assert!(out.contains(expect), "lisp init missing {expect}:\n{out}");
        assert!(out_t.contains(expect), "ts init missing {expect}:\n{out_t}");
    }

    // Pass 2 — swap0: dy=999999, new price=4294.
    let (ok, out) = l.pass("swap0", SWAP0);
    assert!(ok, "lisp swap0 failed:\n{out}");
    let (ok, out_t) = t.pass("swap0", SWAP0);
    assert!(ok, "ts swap0 failed:\n{out_t}");
    for expect in ["999999", "4294"] {
        assert!(out.contains(expect), "lisp swap0 missing {expect}:\n{out}");
        assert!(out_t.contains(expect), "ts swap0 missing {expect}:\n{out_t}");
    }

    // Pass 3 — swap1: dx_out=1000225266864, new price=214748364804294.
    let (ok, out) = l.pass("swap1", SWAP1);
    assert!(ok, "lisp swap1 failed:\n{out}");
    let (ok, out_t) = t.pass("swap1", SWAP1);
    assert!(ok, "ts swap1 failed:\n{out_t}");
    for expect in ["1000223266888", "2147487942"] {
        assert!(out.contains(expect), "lisp swap1 missing {expect}:\n{out}");
        assert!(out_t.contains(expect), "ts swap1 missing {expect}:\n{out_t}");
    }

    // Pass 4 — remove + views: liq=600000, sp_at_tick(0)=4294967296,
    // sp_at_tick(1)=4295163904 (python-verified isqrt/pow32 chain).
    let (ok, out) = l.pass("remove", REMOVE_LIS);
    assert!(ok, "lisp remove failed:\n{out}");
    let (ok, out_t) = t.pass("remove", REMOVE_TSS);
    assert!(ok, "ts remove failed:\n{out_t}");
    for expect in ["600000", "4294967296", "4295163904"] {
        assert!(out.contains(expect), "lisp remove missing {expect}:\n{out}");
        assert!(out_t.contains(expect), "ts remove missing {expect}:\n{out_t}");
    }
}

#[test]
fn clmm_ts_tick_nets_differential() {
    if !has_near_mock() {
        return;
    }
    let l = Runner::new("nets_l", compile_lisp(LIS));
    let t = Runner::new("nets_t", compile_ts(TSS));

    // add liquidity, then read boundary tick nets: -1000000 / +1000000.
    let lis_steps = r#"[
    {"method": "initialize", "args": {"sp": 4294967296, "tick": 0}},
    {"method": "add_liquidity", "args": {"lower": -100, "upper": 100, "liq": 1000000}},
    {"method": "tick_net", "args": {"tick": -100}},
    {"method": "tick_net", "args": {"tick": 100}},
    {"method": "tick_gross", "args": {"tick": -100}}
  ]"#;
    let ts_steps = r#"[
    {"method": "initialize", "args": {"sp": 4294967296, "tick": 0}},
    {"method": "add_liquidity", "args": {"lower": -100, "upper": 100, "liq": 1000000}},
    {"method": "get_tick_net", "args": {"tick": -100}},
    {"method": "get_tick_net", "args": {"tick": 100}},
    {"method": "get_tick_gross", "args": {"tick": -100}}
  ]"#;
    let (ok, out) = l.pass("nets", lis_steps);
    assert!(ok, "lisp nets failed:\n{out}");
    let (ok, out_t) = t.pass("nets", ts_steps);
    assert!(ok, "ts nets failed:\n{out_t}");
    // both must contain the negative net AND the positive one; the gross
    // read at -100 = +1000000 (also asserted by the lisp/TS equality).
    for expect in ["-1000000", "1000000"] {
        assert!(out.contains(expect), "lisp nets missing {expect}:\n{out}");
        assert!(out_t.contains(expect), "ts nets missing {expect}:\n{out_t}");
    }
}
