// reproduce the 4 failing near_mock tier tests, isolated
use lisp_rlm_wasm::compile_near;

fn run(src: &str, tag: &str, deposit: Option<&str>) -> String {
    let wasm = compile_near(src).expect("compile failed");
    let p = format!("/tmp/nmdebug/rt_{tag}.wasm");
    std::fs::write(&p, &wasm).unwrap();
    let state = format!("/tmp/nmdebug/rt_state_{tag}.bin");
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg(&p).arg("_run").arg("{}").env("NEAR_MOCK_STATE", &state);
    if let Some(d) = deposit {
        cmd.arg("--deposit").arg(d);
    }
    let out = cmd.output().expect("run");
    format!(
        "exit={}\n--stdout--\n{}--stderr--\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn main() {
    std::env::set_var("NEAR_MOCK_STATE", "/tmp/nmdebug/state.bin");
    println!("=== nm_deposit ===");
    println!(
        "{}",
        run(
            "(define (main)\n  (let* ((bal (near/attached_deposit)))\n    (to-string bal)))",
            "dep",
            Some("2000000000000000000")
        )
    );
    println!("=== nm_let_shadow ===");
    println!(
        "{}",
        run(
            "(define (main)\n  (let* ((x 1))\n    (let* ((x (+ x 1)))\n      x)))",
            "shadow",
            None
        )
    );
    println!("=== nm_mul_overflow ===");
    println!(
        "{}",
        run("(define (main) (* 1073741824 1073741824))", "mul", None)
    );
}
