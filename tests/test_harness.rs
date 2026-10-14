//! Test: boot the harness and run one scheduler tick

use lisp_rlm_wasm::*;

mod common;
use common::runtime_lock;

fn eval(code: &str, env: &mut Env, state: &mut EvalState) -> LispVal {
    let exprs = parse_all(code).unwrap();
    lisp_rlm_wasm::program::run_program(&exprs, env, state).unwrap()
}

#[test]
fn test_harness_boot() {
    // flock serializes test threads AND other cargo-test processes; held
    // for the whole body (see tests/common/mod.rs).
    let _flock = runtime_lock();
    // Nuke state at entry, under the lock: no test may depend on another
    // test's cleanup (boot() LOADS whatever runtime/state holds).
    let _ = std::fs::remove_dir_all("runtime/state");
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval("(load-file \"runtime/harness.lisp\")", &mut env, &mut state);
    let boot_result = eval("(boot)", &mut env, &mut state);

    assert!(
        matches!(boot_result, LispVal::Sym(ref s) if s == "booted"),
        "Expected 'booted, got {:?}",
        boot_result
    );
}

#[test]
fn test_register_and_tick() {
    let _flock = runtime_lock();
    // Entry nuke (boot() loads existing state — see test_harness_boot).
    let _ = std::fs::remove_dir_all("runtime/state");
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval("(load-file \"runtime/harness.lisp\")", &mut env, &mut state);
    eval("(boot)", &mut env, &mut state);

    // Register a one-shot intention with a lambda action
    eval(
        r#"(register-intention 
            (dict "id" "test-1" 
                  "type" "one-shot"
                  "action" (lambda () (println "hello from test-1"))
                  "cost" 1))"#,
        &mut env,
        &mut state,
    );

    // Tick
    let sleep_secs = eval("(tick)", &mut env, &mut state);
    assert!(
        matches!(sleep_secs, LispVal::Num(60)),
        "Expected 60, got {:?}",
        sleep_secs
    );

    // After one-shot, intention should be removed
    let remaining = eval("(len *intentions*)", &mut env, &mut state);
    assert!(
        matches!(remaining, LispVal::Num(0)),
        "Expected 0 intentions after one-shot, got {:?}",
        remaining
    );
}

#[test]
fn test_persistence() {
    let _flock = runtime_lock();
    // Entry nuke (boot() loads existing state — see test_harness_boot).
    let _ = std::fs::remove_dir_all("runtime/state");
    let mut env = Env::new();
    let mut state = EvalState::new();

    eval("(load-file \"runtime/harness.lisp\")", &mut env, &mut state);
    eval("(boot)", &mut env, &mut state);

    // Register an intention (no lambda — lambdas don't survive JSON)
    eval(
        r#"(register-intention 
            (dict "id" "persist-test" 
                  "type" "perpetual"
                  "cost" 5))"#,
        &mut env,
        &mut state,
    );

    // Checkpoint
    eval("(checkpoint)", &mut env, &mut state);

    // Verify file was created
    assert!(std::path::Path::new("runtime/state/intentions.json").exists());

    // Load it back
    let loaded = eval(
        "(load-state \"runtime/state/intentions.json\")",
        &mut env,
        &mut state,
    );
    assert!(
        matches!(loaded, LispVal::List(ref l) if l.len() == 1),
        "Expected list of 1, got {:?}",
        loaded
    );
}
