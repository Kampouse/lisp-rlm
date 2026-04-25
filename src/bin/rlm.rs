use lisp_rlm::EvalState;
use lisp_rlm::{lisp_eval, parse_all, Env, GenericProvider, LispVal};
use std::env;

fn main() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    // Initialize LLM provider if API key is available
    if let Ok(provider) = GenericProvider::from_env() {
        state.llm_provider = Some(Box::new(provider));
    }

    // Load stdlib (skip list — recursive defs may hang on eval budget)
    for module in &["math", "list", "string"] {
        if let Some(code) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = parse_all(code) {
                for expr in &exprs {
                    let _ = lisp_eval(expr, &mut env, &mut state);
                }
            }
        }
    }

    let args: Vec<String> = env::args().collect();

    // If a file argument is given, run it
    if args.len() > 1 {
        let path = &args[1];
        match std::fs::read_to_string(path) {
            Ok(code) => match parse_all(&code) {
                Ok(exprs) => {
                    for expr in &exprs {
                        match lisp_eval(expr, &mut env, &mut state) {
                            Ok(val) => {
                                if !matches!(val, LispVal::Nil) {
                                    println!("{}", val);
                                }
                            }
                            Err(e) => eprintln!("ERROR: {}", e),
                        }
                    }
                }
                Err(e) => eprintln!("PARSE ERROR: {}", e),
            },
            Err(e) => eprintln!("Failed to read {}: {}", path, e),
        }
        return;
    }

    // Interactive REPL
    println!("lisp-rlm 0.1.0 — Recursive Language Model runtime");
    let mut rl = rustyline::DefaultEditor::new().unwrap();
    loop {
        match rl.readline("rlm> ") {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);
                match parse_all(&line) {
                    Ok(exprs) => {
                        for expr in &exprs {
                            match lisp_eval(expr, &mut env, &mut state) {
                                Ok(val) => {
                                    if !matches!(val, LispVal::Nil) {
                                        println!("{}", val);
                                    }
                                }
                                Err(e) => eprintln!("ERROR: {}", e),
                            }
                        }
                    }
                    Err(e) => eprintln!("PARSE ERROR: {}", e),
                }
            }
            Err(_) => break,
        }
    }
}
