use lisp_rlm::{parse_all, lisp_eval, Env, LispVal};

fn main() {
    // Deep recursion - should not stack overflow thanks to stacker
    let code = "(define f (lambda (n) (if (<= n 0) 0 (+ 1 (f (- n 1)))))) (f 100000)";
    let exprs = parse_all(code).unwrap();
    let mut env = Env::new();
    let mut result = LispVal::Nil;
    for expr in &exprs {
        match lisp_eval(expr, &mut env) {
            Ok(v) => result = v,
            Err(e) => { println!("ERROR: {}", e); return; }
        }
    }
    println!("Result: {}", result);
}
