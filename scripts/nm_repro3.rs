// verify u128 deposit form renders exact decimal
use lisp_rlm_wasm::compile_near;

fn main() {
    let src = r#"(define (main)
  (let* ((bal (near/attached_deposit_u128)))
    bal))"#;
    let wasm = compile_near(src).expect("compile failed");
    std::fs::write("/tmp/nmdebug/u128dep.wasm", &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("/tmp/nmdebug/u128dep.wasm")
        .arg("_run")
        .arg("{}")
        .arg("--deposit")
        .arg("2000000000000000000")
        .env("NEAR_MOCK_STATE", "/tmp/nmdebug/u128state.bin")
        .output()
        .expect("run");
    println!("{}", String::from_utf8_lossy(&out.stdout));
}
