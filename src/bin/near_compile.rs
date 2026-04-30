use std::fs;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: near-compile <input.lisp>");
        std::process::exit(1);
    }
    let src = fs::read_to_string(&args[1]).expect("read input");
    let wasm_bytes = lisp_rlm_wasm::wasm_emit::compile_near(&src).expect("compile to WASM");
    
    let out = if args.len() > 2 { args[2].clone() } else { args[1].replace(".lisp", ".wasm") };
    
    fs::write(&out, &wasm_bytes).expect("write WASM");
    println!("✅ {} ({} bytes)", out, wasm_bytes.len());
}
