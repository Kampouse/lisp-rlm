// quick harness clone to reproduce what the test does, with state isolation
use lisp_rlm_wasm::compile_near;

fn main() {
    std::env::set_var("NEAR_MOCK_STATE", "/tmp/nmdebug/state.bin");
    let src = "(define (main) 42)";
    let wasm = compile_near(src).expect("compile_near failed");
    let tmp = std::path::Path::new("/tmp/nmdebug/nm42.wasm");
    std::fs::write(tmp, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(tmp)
        .arg("_run")
        .arg("{}")
        .env("NEAR_MOCK_STATE", "/tmp/nmdebug/state.bin")
        .output()
        .expect("near-mock should run");
    println!("--- stdout ---\n{}", String::from_utf8_lossy(&out.stdout));
    println!("--- stderr ---\n{}", String::from_utf8_lossy(&out.stderr));
}
