//! Standalone Lisp-RLM evaluator (native, no WASM)
//! Usage: rlm [file.lisp]
//! Without args: interactive REPL

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        // Run file
        let filename = &args[1];
        let code = fs::read_to_string(filename)
            .unwrap_or_else(|e| { eprintln!("Error reading {}: {}", filename, e); std::process::exit(1) });
        run_file(&code);
    } else {
        // Interactive REPL
        println!("lisp-rlm evaluator. Type (exit) to quit.");
        let stdin = io::stdin();
        let mut env = lisp_rlm_wasm::Env::new();
        let mut state = lisp_rlm_wasm::EvalState::new();

        loop {
            print!("lisp> ");
            io::stdout().flush().unwrap();
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed == "(exit)" || trimmed == "(quit)" { break; }
                    if trimmed.is_empty() { continue; }
                    match lisp_rlm_wasm::parse_all(&line) {
                        Ok(exprs) => {
                            match lisp_rlm_wasm::run_program(&exprs, &mut env, &mut state) {
                                Ok(val) => println!("{}", val),
                                Err(e) => eprintln!("Error: {}", e),
                            }
                        }
                        Err(e) => eprintln!("Parse error: {}", e),
                    }
                }
                Err(_) => break,
            }
        }
    }
}

fn run_file(code: &str) {
    let exprs = match lisp_rlm_wasm::parse_all(code) {
        Ok(e) => e,
        Err(e) => { eprintln!("Parse error: {}", e); std::process::exit(1); }
    };

    let mut env = lisp_rlm_wasm::Env::new();
    let mut state = lisp_rlm_wasm::EvalState::new();

    // Run each expression individually (like REPL) to support define
    let mut last = lisp_rlm_wasm::LispVal::Nil;
    for expr in &exprs {
        match lisp_rlm_wasm::run_program(std::slice::from_ref(expr), &mut env, &mut state) {
            Ok(val) => last = val,
            Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
        }
    }
    // Print final result (skip nil from defines)
    if !matches!(last, lisp_rlm_wasm::LispVal::Nil) {
        println!("{}", last);
    }
}
