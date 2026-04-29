fn main() {
    eprintln!("START");
    let mut env = lisp_rlm::Env::new();
    let mut state = lisp_rlm::EvalState::new();

    eprintln!("loading math...");
    if let Some(code) = lisp_rlm::get_stdlib_code("math") {
        if let Ok(exprs) = lisp_rlm::parse_all(code) {
            let _ = lisp_rlm::run_program(&exprs, &mut env, &mut state);
        }
    }
    eprintln!("math ok");

    eprintln!("loading list...");
    if let Some(code) = lisp_rlm::get_stdlib_code("list") {
        if let Ok(exprs) = lisp_rlm::parse_all(code) {
            let _ = lisp_rlm::run_program(&exprs, &mut env, &mut state);
        }
    }
    eprintln!("list ok");

    eprintln!("loading string...");
    if let Some(code) = lisp_rlm::get_stdlib_code("string") {
        if let Ok(exprs) = lisp_rlm::parse_all(code) {
            let _ = lisp_rlm::run_program(&exprs, &mut env, &mut state);
        }
    }
    eprintln!("string ok");

    println!("STDLIB LOADED");
}
