fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| "(define (main) (+ 123456789 987654321))".to_string());
    let wasm = lisp_rlm_wasm::compile_outlayer(&src).expect("compile failed");
    std::fs::write("/tmp/p1_money.wasm", &wasm).unwrap();
    println!("wrote {} bytes", wasm.len());
}
