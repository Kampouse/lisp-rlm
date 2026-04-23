use lisp_rlm::{lisp_eval, parse_all, Env, LispVal};

fn main() {
    let mut env = Env::new();
    let mut gas = 1_000_000u64;

    // Load stdlib
    for module in &["math", "list", "string"] {
        if let Some(code) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = parse_all(code) {
                for expr in &exprs {
                    let _ = lisp_eval(expr, &mut env, &mut gas);
                }
            }
        }
    }
    gas = 1_000_000;

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
                            match lisp_eval(expr, &mut env, &mut gas) {
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
                gas = 1_000_000;
            }
            Err(_) => break,
        }
    }
}
