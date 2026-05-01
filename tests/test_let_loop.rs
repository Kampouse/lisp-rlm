use lisp_rlm_wasm::*;

fn eval_str(code: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parser::parse_all(code)?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = program::run_program(&[expr.clone()], &mut env, &mut state)?;
    }
    Ok(result)
}

#[test]
fn test_let_simple() {
    let r = eval_str("(let ((x 10)) x)").unwrap();
    assert_eq!(r, LispVal::Num(10));
}

#[test]
fn test_let_in_loop_no_shadow() {
    // let inside loop body, new variable (no shadowing)
    let r = eval_str("(loop ((i 0) (sum 0)) (if (>= i 5) sum (let ((x (* i 2))) (recur (+ i 1) (+ sum x)))))").unwrap();
    // sum = 0 + 2 + 4 + 6 + 8 = 20
    assert_eq!(r, LispVal::Num(20), "let inside loop should accumulate correctly");
}

#[test]
fn test_let_shadow_loop_var() {
    // let shadows a loop var — restore after let
    let r = eval_str("(loop ((i 0) (sum 0)) (if (>= i 5) sum (let ((i 99)) (recur (+ i 1) (+ sum i)))))");
    match r {
        Ok(v) => {
            // If shadow works correctly: i inside let is 99, but recur gets +i 1 where i is still the loop var
            // Actually: after the let restores, the loop var i should be the original value
            // Inside the let body, i = 99. recur receives (+ i 1) = 100, (+ sum i) = sum + 99
            // So: sum = 0+99 + 99+99 + 99+99 + 99+99 + 99+99 ... 
            // Wait no: recur args are evaluated INSIDE the let body where i=99
            // So (recur (+ 99 1) (+ sum 99)) → i=100, which is >=5, returns sum+99
            // First iter: sum = 0+99 = 99, then i=100 >= 5, return 99
            println!("let-shadow result: {:?}", v);
        }
        Err(e) => panic!("let-shadow-loop-var ERR: {}", e),
    }
}

#[test]
fn test_nested_let_in_loop() {
    let r = eval_str("(loop ((i 0)) (if (>= i 3) i (let ((a (+ i 10))) (let ((b (+ a 1))) (recur (+ i 1))))))").unwrap();
    assert_eq!(r, LispVal::Num(3));
}
