use lisp_rlm_wasm::wasm_emit::{compile_pure, compile_standalone, compile_standalone_opts};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    let standalone = args.iter().any(|a| a == "--standalone" || a == "-s");
    let no_typecheck = args.iter().any(|a| a == "--no-typecheck");
    let clean_args: Vec<String> = args
        .iter()
        .filter(|a| *a != "--standalone" && *a != "-s" && *a != "--no-typecheck")
        .cloned()
        .collect();

    if clean_args.len() < 2 {
        eprintln!("Usage: emit_wasm [--standalone|-s] <source.lisp|.ts> [output.wasm]");
        eprintln!("  --standalone  Emit _start entry point, no host imports (for inlayer/wasip1)");
        std::process::exit(1);
    }

    let src_path = &clean_args[1];
    let raw_source = fs::read_to_string(src_path).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {}", src_path, e);
        std::process::exit(1);
    });

    // TS frontend: lower TypeScript source to lisp before the normal pipeline
    let source = if src_path.ends_with(".ts") || src_path.ends_with(".mts") {
        match lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&raw_source) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("❌ TS lowering error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        raw_source
    };

    let output = if clean_args.len() > 2 {
        clean_args[2].clone()
    } else {
        // Default: replace source extension with .wasm
        if src_path.ends_with(".lisp") {
            src_path.replace(".lisp", ".wasm")
        } else if src_path.ends_with(".ts") {
            src_path.replace(".ts", ".wasm")
        } else if src_path.ends_with(".mts") {
            src_path.replace(".mts", ".wasm")
        } else {
            format!("{}.wasm", src_path)
        }
    };

    if standalone {
        let result = if no_typecheck {
            compile_standalone_opts(&source, false)
        } else {
            compile_standalone(&source)
        };
        match result {
            Ok(wasm_bytes) => {
                fs::write(&output, &wasm_bytes).unwrap();
                println!(
                    "✅ Standalone WASM written to: {} ({} bytes)",
                    output,
                    wasm_bytes.len()
                );
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
                println!(
                    "✅ WASM written to: {} ({} bytes)",
                    output,
                    wasm_bytes.len()
                );
            }
            Err(e) => {
                eprintln!("Compile error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
