use lisp_rlm_wasm::wasm_emit::compile_pure;
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: emit_wasm <source.lisp> [output.wasm]");
        std::process::exit(1);
    }

    let source = fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {}", args[1], e);
        std::process::exit(1);
    });

    let output = if args.len() > 2 { args[2].clone() } else { "/tmp/emitted.wasm".into() };

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
