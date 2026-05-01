use lisp_rlm_wasm::*;

#[test]
fn test_let_top_level_trace() {
    let code = "(let ((x 10)) x)";
    let exprs = parser::parse_all(code).unwrap();
    let mut env = Env::new();
    let mut state = EvalState::new();
    
    // Manually replicate what run_program does for Phase 5
    let body = &exprs[0];
    let closed_env = std::sync::Arc::new(std::sync::RwLock::new(env.snapshot()));
    let cl = bytecode::try_compile_lambda(
        &[],
        body,
        &closed_env.read().unwrap().clone().into_iter().collect::<Vec<_>>(),
        &env,
        None,
        None,
    );
    match cl {
        Some(cl) => {
            println!("Compiled! total_slots={}, code={:?}", cl.total_slots, cl.code);
            let result = bytecode::run_compiled_lambda(&cl, &[], &mut env, &mut state);
            println!("Result: {:?}", result);
        }
        None => println!("Compilation returned None!"),
    }
}
