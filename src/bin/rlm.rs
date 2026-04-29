use lisp_rlm::EvalState;
use lisp_rlm::{run_program, parse_all, Env, GenericProvider, LispVal};
use std::env;
use std::time::Instant;

// ANSI color codes — no external deps needed
mod color {
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    #[allow(dead_code)]
    pub const BLUE: &str = "\x1b[34m";
    pub const CYAN: &str = "\x1b[36m";
    pub const DIM: &str = "\x1b[2m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RESET: &str = "\x1b[0m";
}

fn print_error(msg: &str, state: &EvalState) {
    let trace = state.format_trace();
    eprintln!(
        "{}{}ERROR:{} {}\n{}{}Call trace:\n{}{}",
        color::BOLD,
        color::RED,
        color::RESET,
        msg,
        color::DIM,
        color::CYAN,
        trace,
        color::RESET,
    );
}

fn print_value(val: &LispVal) {
    let s = val.to_string();
    // Colorize output: strings in green, numbers in cyan, nil dimmed
    if matches!(val, LispVal::Str(_)) {
        println!("{}{}{}", color::GREEN, s, color::RESET);
    } else if matches!(val, LispVal::Num(_)) {
        println!("{}{}{}", color::CYAN, s, color::RESET);
    } else if matches!(val, LispVal::Nil) {
        println!("{}nil{}", color::DIM, color::RESET);
    } else if matches!(val, LispVal::Bool(true)) {
        println!("{}true{}", color::GREEN, color::RESET);
    } else if matches!(val, LispVal::Bool(false)) {
        println!("{}false{}", color::RED, color::RESET);
    } else if matches!(val, LispVal::Lambda { .. }) {
        println!("{}<lambda>{}", color::YELLOW, color::RESET);
    } else {
        println!("{}", s);
    }
}

fn print_banner() {
    println!(
        "{}{}lisp-rlm{} {}0.2.0{} — Recursive Language Model runtime",
        color::BOLD,
        color::CYAN,
        color::RESET,
        color::DIM,
        color::RESET,
    );
    println!(
        "{}Type {}:help for commands, (:quit) or Ctrl-D to exit{}",
        color::DIM,
        color::YELLOW,
        color::RESET,
    );
}

fn handle_repl_command(line: &str, env: &mut Env, state: &mut EvalState) -> bool {
    let trimmed = line.trim();
    match trimmed {
        ":help" | ":h" => {
            println!("{}Commands:{}", color::BOLD, color::RESET);
            println!(
                "  {}:help{}        Show this help",
                color::YELLOW,
                color::RESET
            );
            println!(
                "  {}:env{}         Show all bindings",
                color::YELLOW,
                color::RESET
            );
            println!(
                "  {}:time EXPR{}   Time an expression",
                color::YELLOW,
                color::RESET
            );
            println!(
                "  {}:reset{}       Reset environment",
                color::YELLOW,
                color::RESET
            );
            println!(
                "  {}:trace{}       Show call trace",
                color::YELLOW,
                color::RESET
            );
            println!("  {}:quit{}        Exit", color::YELLOW, color::RESET);
            println!();
            println!("{}Built-in docs:{}", color::BOLD, color::RESET);
            println!(
                "  {}(:doc \"map\"){} Show help for a builtin",
                color::CYAN,
                color::RESET
            );
            println!(
                "  {}(:doc \"+\"){}   Works with strings or symbols",
                color::CYAN,
                color::RESET
            );
        }
        ":env" => {
            let bindings = env.snapshot();
            if bindings.is_empty() {
                println!("{}(empty environment){}", color::DIM, color::RESET);
            } else {
                let mut keys: Vec<_> = bindings.keys().collect();
                keys.sort();
                for k in keys {
                    let v = bindings.get(k).unwrap();
                    let type_tag = match v {
                        LispVal::Lambda { .. } => "fn",
                        LispVal::Num(_) => "num",
                        LispVal::Str(_) => "str",
                        LispVal::Bool(_) => "bool",
                        LispVal::List(_) => "list",
                        LispVal::Map(_) => "dict",
                        LispVal::Nil => "nil",
                        _ => "?",
                    };
                    println!(
                        "  {}{}:{} {}{}{}",
                        color::CYAN,
                        k,
                        color::DIM,
                        type_tag,
                        color::RESET,
                        if matches!(v, LispVal::Lambda { .. }) {
                            String::new()
                        } else {
                            format!(" = {}", v)
                        }
                    );
                }
            }
        }
        ":reset" => {
            *env = Env::new();
            *state = EvalState::new();
            println!("{}Environment reset.{}", color::GREEN, color::RESET);
        }
        ":trace" => {
            let trace = state.format_trace();
            println!("{}{}{}", color::DIM, trace, color::RESET);
        }
        ":quit" | ":q" => return false,
        _ if trimmed.starts_with(":time ") => {
            let code = &trimmed[6..];
            match parse_all(code) {
                Ok(exprs) => {
                    let start = Instant::now();
                    match run_program(&exprs, env, state) {
                        Ok(val) => {
                            let elapsed = start.elapsed();
                            print_value(&val);
                            println!("{}({:.2?}){}", color::DIM, elapsed, color::RESET);
                        }
                        Err(e) => print_error(&e, state),
                    }
                }
                Err(e) => eprintln!("{}PARSE ERROR:{} {}", color::RED, color::RESET, e),
            }
        }
        _ => return false, // Not a command
    }
    true // Command handled, keep running
}

fn main() {
    // Spawn with 64MB stack to avoid overflow from deeply nested eval
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(real_main)
        .expect("thread spawn failed")
        .join()
        .expect("thread panicked");
}

fn real_main() {
    let mut env = Env::new();
    let mut state = EvalState::new();

    // Initialize LLM provider if API key is available
    if let Ok(provider) = GenericProvider::from_env() {
        state.llm_provider = Some(Box::new(provider));
    }

    // Load stdlib
    for module in &["math", "list", "string"] {
        if let Some(code) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = parse_all(code) {
                if let Err(e) = run_program(&exprs, &mut env, &mut state) {
                    eprintln!("{}STDLOAD ERROR ({}):{} {}", color::RED, module, color::RESET, e);
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
                Ok(exprs) => match run_program(&exprs, &mut env, &mut state) {
                    Ok(val) => {
                        if !matches!(val, LispVal::Nil) {
                            println!("{}", val);
                        }
                    }
                    Err(e) => {
                        print_error(&e, &state);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!(
                        "{}{}PARSE ERROR:{} {}",
                        color::BOLD,
                        color::RED,
                        color::RESET,
                        e
                    );
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("{}Failed to read {}:{}", color::RED, path, e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Interactive REPL
    print_banner();
    let mut rl = rustyline::DefaultEditor::new().unwrap();
    loop {
        match rl.readline("rlm> ") {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);

                // Check for REPL commands
                if handle_repl_command(&line, &mut env, &mut state) {
                    continue;
                }

                match parse_all(&line) {
                    Ok(exprs) => match run_program(&exprs, &mut env, &mut state) {
                        Ok(val) => {
                            if !matches!(val, LispVal::Nil) {
                                print_value(&val);
                            }
                        }
                        Err(e) => print_error(&e, &state),
                    },
                    Err(e) => {
                        eprintln!(
                            "{}{}PARSE ERROR:{} {}",
                            color::BOLD,
                            color::RED,
                            color::RESET,
                            e
                        );
                    }
                }
            }
            Err(_) => break,
        }
    }
}
