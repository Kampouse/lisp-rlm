fn main() {
    let mut env = lisp_rlm::Env::new();
    let mut state = lisp_rlm::EvalState::new();

    // Load math + list first
    for module in &["math", "list"] {
        if let Some(code) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = lisp_rlm::parse_all(code) {
                let _ = lisp_rlm::run_program(&exprs, &mut env, &mut state);
            }
        }
    }
    eprintln!("math+list loaded");

    // Now load string
    if let Some(code) = lisp_rlm::get_stdlib_code("string") {
        if let Ok(exprs) = lisp_rlm::parse_all(code) {
            eprintln!("string has {} exprs", exprs.len());
            match lisp_rlm::run_program(&exprs, &mut env, &mut state) {
                Ok(v) => eprintln!("string loaded, last result: {}", v),
                Err(e) => eprintln!("string ERR: {}", e),
            }
        }
    }
    eprintln!("string done");
    println!("OK");
}
