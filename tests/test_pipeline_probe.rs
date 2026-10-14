//! Pipeline divergence probe (2026-09-28): the CLI (`lisp-rlm build`) traps
//! on pool.ts quote views while the exprs path passes. This test compiles
//! BOTH ways in-process and probes quote_buy via near-mock, splitting the
//! problem: library string-path (desugar/resolve_modules) vs binary env.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::wasm_emit::{compile_near, compile_near_from_exprs};
use lisp_rlm_wasm::{parse_all, typing::type_check_program};
use std::process::Command;

#[test]
fn exprs_path_rejects_double_consume_without_source() {
    // The exprs-only pipeline (compile_near_from_exprs) has no source text;
    // the promise single-use lint must STILL reject there (labels "line ?").
    use lisp_rlm_wasm::parse_all;
    let src = r#"(define (main)
  (let* ((p (near/promise_batch_create "recv.test.near")))
    (let* ((a (near/promise_then p (near/current_account_id) "cb1" "{}" "0" 10000000000000)))
      (near/promise_then p (near/current_account_id) "cb2" "{}" "0" 10000000000000))))
"#;
    let exprs = parse_all(src).unwrap();
    let err = lisp_rlm_wasm::wasm_emit::compile_near_from_exprs(&exprs)
        .expect_err("double promise_then must be rejected on the exprs path");
    assert!(
        err.contains("consumed 2 times"),
        "exprs-path error must name the double consumption, got: {err}"
    );
}

#[test]
fn pipeline_divergence_probe() {
    let src = std::fs::read_to_string("projects/launchpad/pool.ts").unwrap();
    let ir = ts_to_lisp_source(&src).unwrap();

    // path A: exprs (test/cargo pipeline — known good)
    let exprs = parse_all(&ir).unwrap();
    type_check_program(&exprs, true).unwrap();
    let wasm_a = compile_near_from_exprs(&exprs).unwrap();

    // path B: string (CLI pipeline — compile_near re-parses + desugars)
    let wasm_b = compile_near(&ir).unwrap();

    println!("wasm_a (exprs) = {} bytes", wasm_a.len());
    println!("wasm_b (string) = {} bytes", wasm_b.len());
    println!("identical bytes: {}", wasm_a == wasm_b);
    // PIPELINE EQUIVALENCE INVARIANT (2026-09-28): compile_near (string,
    // re-parse path used by lisp-rlm build / near-compile) and
    // compile_near_from_exprs (pre-parsed path used by every cargo test)
    // MUST emit byte-identical wasm. The pre-merge emitter violated this
    // (u128 operand seam — GAPS.md); fixed by the storage-iter merge
    // (762106c). If this breaks, the two pipelines have diverged again and
    // cargo-green does NOT certify the deploy artifact.
    assert!(
        wasm_a == wasm_b,
        "pipeline divergence: string path and exprs path emit DIFFERENT wasm          (see GAPS.md u128 operand seam)"
    );
    // cheap stable hash for cross-process comparison (CLI build output)
    let fnv = |b: &[u8]| -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for x in b {
            h ^= *x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    };
    println!("fnv_a = {:016x}", fnv(&wasm_a));
    println!("fnv_b = {:016x}", fnv(&wasm_b));

    let dir = std::env::temp_dir();
    let pa = dir.join("pipeA_fixed.wasm");
    let pb = dir.join("pipeB_fixed.wasm");
    std::fs::write(&pa, &wasm_a).unwrap();
    std::fs::write(&pb, &wasm_b).unwrap();

    let state = dir.join(format!("pipe_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&state);
    let manifest = format!(
        "pool.test.near={},token.test.near=/tmp/tidbg/token.wasm",
        pb.display()
    );

    let run = |contract: &str, method: &str, args: &str, attach: Option<&str>| {
        let mut cmd = Command::new("./target/release/near-mock");
        cmd.env("NEAR_MOCK_QUIET", "1")
            .arg("cross")
            .arg(&state)
            .arg(&manifest)
            .arg(contract)
            .arg(method)
            .arg(args)
            .arg("--signer")
            .arg("factory.test.near");
        if let Some(a) = attach {
            cmd.arg("--attach").arg(a);
        }
        let out = cmd.output().unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    run("pool.test.near", "new", "{}", None);
    run(
        "token.test.near",
        "new",
        r#"{"owner_id":"factory.test.near","total_supply":"1000000000000000000000000000","name":"P","symbol":"p","icon":"","decimals":"18"}"#,
        None,
    );
    run(
        "pool.test.near",
        "seed_pool",
        r#"{"token":"token.test.near","grad_th":"0"}"#,
        Some("1000000000000000000000000"),
    );
    run(
        "token.test.near",
        "ft_transfer_call",
        r#"{"receiver_id":"pool.test.near","amount":"1000000000000000000000000000","msg":"seed"}"#,
        Some("1"),
    );

    let out_b = run(
        "pool.test.near",
        "quote_buy",
        r#"{"token":"token.test.near","near_in":"1000000000000000000000000"}"#,
        None,
    );
    let trap_b = out_b.contains("trap") || out_b.contains("__h_u128_parse");
    println!(
        "string-path wasm quote_buy: {}",
        if trap_b { "TRAP" } else { "OK" }
    );

    // fresh state for path A
    let _ = std::fs::remove_file(&state);
    let manifest_a = format!(
        "pool.test.near={},token.test.near=/tmp/tidbg/token.wasm",
        pa.display()
    );
    let run_a = |contract: &str, method: &str, args: &str, attach: Option<&str>| {
        let mut cmd = Command::new("./target/release/near-mock");
        cmd.env("NEAR_MOCK_QUIET", "1")
            .arg("cross")
            .arg(&state)
            .arg(&manifest_a)
            .arg(contract)
            .arg(method)
            .arg(args)
            .arg("--signer")
            .arg("factory.test.near");
        if let Some(a) = attach {
            cmd.arg("--attach").arg(a);
        }
        let out = cmd.output().unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    run_a("pool.test.near", "new", "{}", None);
    run_a(
        "token.test.near",
        "new",
        r#"{"owner_id":"factory.test.near","total_supply":"1000000000000000000000000000","name":"P","symbol":"p","icon":"","decimals":"18"}"#,
        None,
    );
    run_a(
        "pool.test.near",
        "seed_pool",
        r#"{"token":"token.test.near","grad_th":"0"}"#,
        Some("1000000000000000000000000"),
    );
    run_a(
        "token.test.near",
        "ft_transfer_call",
        r#"{"receiver_id":"pool.test.near","amount":"1000000000000000000000000000","msg":"seed"}"#,
        Some("1"),
    );
    let out_a = run_a(
        "pool.test.near",
        "quote_buy",
        r#"{"token":"token.test.near","near_in":"1000000000000000000000000"}"#,
        None,
    );
    let trap_a = out_a.contains("trap") || out_a.contains("__h_u128_parse");
    println!(
        "exprs-path wasm quote_buy: {}",
        if trap_a { "TRAP" } else { "OK" }
    );

    assert!(
        !trap_a && !trap_b,
        "pipeline probe: quote_buy must run clean on BOTH compiled artifacts          (a regression here means the emitter diverged again — see GAPS.md          u128 operand seam entry)"
    );
}
