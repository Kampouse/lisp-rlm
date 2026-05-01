use std::env;
use std::fs;
use std::process;

use clojure_rlm::parser::CljParser;
use clojure_rlm::desugar::desugar;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: clj-compile <file.clj> [-o output.wasm]");
        process::exit(1);
    }

    let filename = &args[1];
    let code = fs::read_to_string(filename)
        .unwrap_or_else(|e| { eprintln!("Error reading {}: {}", filename, e); process::exit(1) });

    let clj_exprs = match CljParser::parse_all(&code) {
        Ok(e) => e,
        Err(e) => { eprintln!("Parse error: {}", e); process::exit(1) }
    };

    let lisp_exprs: Vec<lisp_rlm_wasm::LispVal> = clj_exprs.iter().map(desugar).collect();

    // Check for --wat flag
    let wat_mode = args.iter().any(|a| a == "--wat");

    if wat_mode {
        match lisp_rlm_wasm::compile_near_to_wat_from_exprs(&lisp_exprs) {
            Ok(wat) => println!("{}", wat),
            Err(e) => { eprintln!("Compile error: {}", e); process::exit(1) }
        }
    } else {
        match lisp_rlm_wasm::compile_near_from_exprs(&lisp_exprs) {
            Ok(wasm_bytes) => {
                let out_path = args.iter().position(|a| a == "-o")
                    .and_then(|i| args.get(i + 1))
                    .map(|s| s.as_str())
                    .unwrap_or("out.wasm");
                fs::write(out_path, &wasm_bytes)
                    .unwrap_or_else(|e| { eprintln!("Error writing {}: {}", out_path, e); process::exit(1) });
                eprintln!("Compiled {} bytes → {}", wasm_bytes.len(), out_path);
            }
            Err(e) => { eprintln!("Compile error: {}", e); process::exit(1) }
        }
    }
}
