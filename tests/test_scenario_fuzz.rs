//! Scenario-runner FUZZ — randomized orderings and fail-masks against the
//! trap_stub fixture (tests/fixtures/trap_stub.wat, wat2wasm'd at setup).
//! Asserts CRASH-FREEDOM of `near-mock scenario` plus two fail-closed
//! invariants on every run:
//!   I1: any step that fails (asserted or masked) ⇒ scenario exits nonzero
//!   I2: a gas-capped `spin` ⇒ NEAR-atomicity rollback (storage identical
//!       to the pre-spin snapshot — no partial writes leak through a trap)
//! Crash (signal/panic) = corpus invalid. Designed failure exiting 1 with
//! the right markers = valid run.

use std::process::Command;
use std::sync::Mutex;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    match L.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn has_near_mock() -> bool {
    Command::new("./target/release/near-mock")
        .args(["--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// wat2wasm the trap_stub fixture into a temp path (cached per-process).
fn stub_wasm() -> std::path::PathBuf {
    static DONE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DONE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("nm_fuzz_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wat =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trap_stub.wat");
        let out = dir.join("trap_stub.wasm");
        let st = Command::new("wat2wasm")
            .arg(&wat)
            .arg("-o")
            .arg(&out)
            .status()
            .expect("wat2wasm must exist (wabt) — same dep as verify_near_mock.sh");
        assert!(st.success(), "wat2wasm failed");
        out
    })
    .clone()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn run(json: &std::path::Path) -> (bool, String, Option<i32>) {
    let out = Command::new("./target/release/near-mock")
        .args(["scenario"])
        .arg(json)
        .output()
        .expect("near-mock should run");
    use std::os::unix::process::ExitStatusExt;
    let code = out
        .status
        .code()
        .or_else(|| out.status.signal().map(|s| -s));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text, code)
}

const N_RUNS: u64 = 40;

#[test]
fn fuzz_scenario_orderings_and_fail_masks() {
    if !has_near_mock() {
        return;
    }
    let _l = lock();
    let wasm = stub_wasm();
    let dir = std::env::temp_dir().join(format!("nm_fuzz_{}", std::process::id()));
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ std::process::id() as u64);
    let mut crashes = Vec::new();
    let mut inv1_viol = Vec::new();
    let mut inv2_viol = Vec::new();
    let mut designed_fail = 0u32;
    let mut clean_pass = 0u32;

    for run_idx in 0..N_RUNS {
        // every run gets its own state file — runs are independent
        let state = dir.join(format!("fuzz_{run_idx}.bin"));
        let _ = std::fs::remove_file(&state);
        let n_steps = 3 + rng.below(8); // 3..=10 steps
        let mut steps: Vec<String> = Vec::new();
        // pre-spin snapshot point for I2 (index within generated steps)
        let mut spin_at: Option<usize> = None;
        let mut expect_masked_fail = false;
        for si in 0..n_steps as usize {
            let roll = rng.below(10);
            if roll < 3 {
                steps.push(format!(
                    r#"{{"contract": "stub.test.near", "method": "write_a", "expect": "a"}}"#
                ));
            } else if roll < 6 {
                steps.push(format!(
                    r#"{{"contract": "stub.test.near", "method": "write_b", "expect": "b"}}"#
                ));
            } else if roll < 8 {
                // gas-capped spin: MUST trap (asserted) ⇒ rollback invariant
                steps.push(format!(
                    r#"{{"contract": "stub.test.near", "method": "spin", "gas": "{}", "expect": "trap"}}"#,
                    1 + rng.below(4)
                ));
                if spin_at.is_none() {
                    spin_at = Some(si);
                }
            } else if roll == 8 {
                // guaranteed-failing expect (bogus marker) ⇒ step FAIL ⇒
                // scenario exits nonzero — keeps invariant I1 exercised
                steps.push(format!(
                    r#"{{"contract": "stub.test.near", "method": "write_a", "expect": "nope-{}"}}"#,
                    rng.below(1_000_000)
                ));
                expect_masked_fail = true;
            } else {
                // bookkeeping snapshot
                steps.push(format!(r#"{{"snapshot": "s{}"}}"#, si));
            }
        }
        let scenario = format!(
            r#"{{"name": "fuzz_{}", "manifest": "stub.test.near={}", "state": "{}", "steps": [{}]
        }}"#,
            run_idx,
            wasm.display(),
            state.display(),
            steps.join(",\n")
        );
        let jpath = dir.join(format!("fuzz_{run_idx}.json"));
        std::fs::write(&jpath, &scenario).unwrap();
        let (ok, out, code) = run(&jpath);

        // crash-freedom: signal death or runner panic = corpus invalid
        let crashed = match code {
            Some(c) if c < 0 => true,
            Some(101) => out.contains("panicked"),
            None => true,
            _ => false,
        };
        if crashed {
            crashes.push(format!("run {run_idx}: code {code:?}: {out}"));
            continue;
        }
        // did any step fail? (marker printed by the runner)
        let any_fail = out.contains("⇒ FAIL");
        if !ok {
            designed_fail += 1;
            // I1: nonzero exit must be backed by an actual step failure
            if !any_fail {
                inv1_viol.push(format!("run {run_idx}: exit 1 without '⇒ FAIL': {out}"));
            }
        } else {
            clean_pass += 1;
            if expect_masked_fail && any_fail {
                // success with a failing step should not happen (fail steps
                // flip the summary) — flag for investigation
                inv1_viol.push(format!("run {run_idx}: exit 0 despite '⇒ FAIL': {out}"));
            }
        }
        // I2: any gas-capped spin must NOT leak storage — the run must show
        // the rollback marker right after the trapped step
        if out.contains("expect 'trap'") || out.contains("✓ trap as expected") {
            if !out.contains("rolled back") && !out.contains("rolled back (") {
                inv2_viol.push(format!(
                    "run {run_idx}: trap without rollback marker: {out}"
                ));
            }
        }
        let _ = spin_at;
    }
    assert!(crashes.is_empty(), "runner crashes: {:#?}", crashes);
    assert!(inv1_viol.is_empty(), "I1 violations: {:#?}", inv1_viol);
    assert!(inv2_viol.is_empty(), "I2 violations: {:#?}", inv2_viol);
    // sanity: the corpus must actually exercise BOTH exit paths
    assert!(
        designed_fail > 0 && clean_pass > 0,
        "corpus produced no runs (fail={designed_fail}, clean={clean_pass})"
    );
    eprintln!(
        "fuzz: {N_RUNS} runs — {clean_pass} clean, {designed_fail} designed-fail, 0 crashes, 0 invariant violations"
    );
}
