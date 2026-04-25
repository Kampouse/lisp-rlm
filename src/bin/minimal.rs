fn main() {
    let mut env = lisp_rlm::Env::new();
    let mut state = lisp_rlm::EvalState::new();

    // Load math + list first
    for module in &["math", "list"] {
        if let Some(code) = lisp_rlm::get_stdlib_code(module) {
            if let Ok(exprs) = lisp_rlm::parse_all(code) {
                for expr in &exprs {
                    let _ = lisp_rlm::lisp_eval(expr, &mut env, &mut state);
                }
            }
        }
    }
    eprintln!("math+list loaded");

    // Now load string
    if let Some(code) = lisp_rlm::get_stdlib_code("string") {
        if let Ok(exprs) = lisp_rlm::parse_all(code) {
            eprintln!("string has {} exprs", exprs.len());
            for (i, expr) in exprs.iter().enumerate() {
                eprintln!("  eval [{}]...", i);
                match lisp_rlm::lisp_eval(expr, &mut env, &mut state) {
                    Ok(v) => eprintln!("  [{}] = {}", i, v),
                    Err(e) => {
                        eprintln!("  [{}] ERR: {}", i, e);
                        break;
                    }
                }
            }
        }
    }
    eprintln!("string done");
    println!("OK");
}
