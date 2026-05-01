use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

use clojure_rlm::parser::CljParser;
use clojure_rlm::desugar::desugar;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check for -e flag (inline eval)
    if args.len() >= 3 && (args[1] == "-e" || args[1] == "--eval") {
        inline_eval(&args[2]);
        return;
    }

    if args.len() > 1 {
        let filename = &args[1];
        let code = fs::read_to_string(filename)
            .unwrap_or_else(|e| { eprintln!("Error reading {}: {}", filename, e); std::process::exit(1) });
        run_file(&code);
    } else {
        println!("clj-rlm. Clojure frontend for Lisp-RLM VM. Type (exit) to quit.");
        let stdin = io::stdin();
        let mut env = lisp_rlm_wasm::Env::new();
        let mut state = lisp_rlm_wasm::EvalState::new();

        loop {
            print!("clj=> ");
            io::stdout().flush().unwrap();
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed == "(exit)" || trimmed == "(quit)" { break; }
                    if trimmed.is_empty() { continue; }
                    match CljParser::parse_all(&line) {
                        Ok(clj_exprs) => {
                            let lisp_exprs: Vec<lisp_rlm_wasm::LispVal> = clj_exprs.iter().map(desugar).collect();
                            match lisp_rlm_wasm::run_program(&lisp_exprs, &mut env, &mut state) {
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

fn inline_eval(code: &str) {
    let clj_exprs = match CljParser::parse_all(code) {
        Ok(e) => e,
        Err(e) => { eprintln!("Parse error: {}", e); std::process::exit(1) }
    };
    let lisp_exprs: Vec<lisp_rlm_wasm::LispVal> = clj_exprs.iter().map(desugar).collect();
    let mut env = lisp_rlm_wasm::Env::new();
    let mut state = lisp_rlm_wasm::EvalState::new();
    for expr in &lisp_exprs {
        match lisp_rlm_wasm::run_program(std::slice::from_ref(expr), &mut env, &mut state) {
            Ok(val) => {
                if !matches!(val, lisp_rlm_wasm::LispVal::Nil) {
                    println!("{}", val);
                }
            }
            Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
        }
    }
}

fn run_file(code: &str) {
    let clj_exprs = match CljParser::parse_all(code) {
        Ok(e) => e,
        Err(e) => { eprintln!("Parse error: {}", e); std::process::exit(1) }
    };

    let lisp_exprs: Vec<lisp_rlm_wasm::LispVal> = clj_exprs.iter().map(desugar).collect();
    let mut env = lisp_rlm_wasm::Env::new();
    let mut state = lisp_rlm_wasm::EvalState::new();

    for expr in &lisp_exprs {
        match lisp_rlm_wasm::run_program(std::slice::from_ref(expr), &mut env, &mut state) {
            Ok(val) => {
                if !matches!(val, lisp_rlm_wasm::LispVal::Nil) {
                    println!("{}", val);
                }
            }
            Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
        }
    }
}
