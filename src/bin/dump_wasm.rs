use wasmtime::{Engine, Module};
use wasmparser::Validator;

fn main() {
    let source = std::env::args().nth(1).unwrap_or_else(|| "(define (run) (if 1 42 99))".to_string());
    let wasm = lisp_rlm_wasm::wasm_emit::compile_fuzz(&source).unwrap();
    let path = "/tmp/test_fuzz.wasm";
    std::fs::write(path, &wasm).unwrap();
    println!("Wrote {} bytes to {}", wasm.len(), path);

    // Validate with wasmtime
    let engine = Engine::default();
    match Module::new(&engine, &wasm) {
        Ok(_) => println!("wasmtime: OK"),
        Err(e) => println!("wasmtime FAILED: {}", e),
    }

    // Detailed validation with wasmparser
    let mut validator = Validator::new();
    match validator.validate_all(&wasm) {
        Ok(_) => println!("wasmparser: OK"),
        Err(e) => println!("wasmparser FAILED: {:?}", e),
    }
}
