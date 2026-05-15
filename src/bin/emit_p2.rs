use lisp_rlm_wasm::p2_component::build_p2_component;
use lisp_rlm_wasm::wasm_emit::compile_pure;
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: emit_p2 <source.lisp> [output.wasm]");
        std::process::exit(1);
    }

    let source = fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {}", args[1], e);
        std::process::exit(1);
    });

    let output = if args.len() > 2 {
        args[2].clone()
    } else {
        "/tmp/emitted_p2.wasm".into()
    };

    // Step 1: Compile Lisp → core WASM (P1)
    let core_bytes = match compile_pure(&source) {
        Ok(wasm) => wasm,
        Err(e) => {
            eprintln!("Compile error: {}", e);
            std::process::exit(1);
        }
    };
    eprintln!("Core WASM: {} bytes", core_bytes.len());

    // Step 2: Wrap into P2 component
    let p2_bytes = match build_p2_component(&core_bytes) {
        Ok(wasm) => wasm,
        Err(e) => {
            eprintln!("P2 wrap error: {}", e);
            std::process::exit(1);
        }
    };

    fs::write(&output, &p2_bytes).unwrap();
    println!(
        "✅ P2 component written to: {} ({} bytes)",
        output,
        p2_bytes.len()
    );
}
