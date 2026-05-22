//! Quick test: translate a Solidity file to Lisp, then compile to NEAR WASM.
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: sol_compile <input.sol> <output.wasm>");
        std::process::exit(1);
    }
    let sol_src = fs::read_to_string(&args[1]).unwrap();
    let lisp_vals = lisp_rlm_wasm::solidity::translate_solidity(&sol_src).unwrap();
    let lisp_src: String = lisp_vals.iter().map(|v| format!("{}\n", v)).collect();
    println!("Translated Lisp:\n{}", lisp_src);
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near_untyped(&lisp_src).unwrap();
    fs::write(&args[2], &wasm).unwrap();
    println!("✅ {} ({} bytes)", args[2], wasm.len());
}
