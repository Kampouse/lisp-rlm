use lisp_rlm_wasm::wasm_emit::{compile_pure, compile_standalone};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    let standalone = args.iter().any(|a| a == "--standalone" || a == "-s");
    let clean_args: Vec<String> = args.iter().filter(|a| *a != "--standalone" && *a != "-s").cloned().collect();
    
    if clean_args.len() < 2 {
        eprintln!("Usage: emit_wasm [--standalone|-s] <source.lisp> [output.wasm]");
        eprintln!("  --standalone  Emit _start entry point, no host imports (for inlayer/wasip1)");
        std::process::exit(1);
    }

    let source = fs::read_to_string(&clean_args[1]).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {}", clean_args[1], e);
        std::process::exit(1);
    });

    let output = if clean_args.len() > 2 { clean_args[2].clone() } else { "/tmp/emitted.wasm".into() };

    if standalone {
        match compile_standalone(&source) {
            Ok(wasm_bytes) => {
                fs::write(&output, &wasm_bytes).unwrap();
                println!("✅ Standalone WASM written to: {} ({} bytes)", output, wasm_bytes.len());
            }
            Err(e) => {
                eprintln!("Compile error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        match compile_pure(&source) {
            Ok(wasm_bytes) => {
                fs::write(&output, &wasm_bytes).unwrap();
                println!("✅ WASM written to: {} ({} bytes)", output, wasm_bytes.len());
            }
            Err(e) => {
                eprintln!("Compile error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
