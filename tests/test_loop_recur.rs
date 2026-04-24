use lisp_rlm::*;

fn run_program(code: &str, env: &mut Env) -> Result<String, String> {
    let exprs = parse_all(code)?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(&expr, env)?;
    }
    Ok(result.to_string())
}

fn eval(code: &str) -> String {
    let mut env = Env::new();
    run_program(code, &mut env).unwrap_or_else(|e| format!("ERROR: {}", e))
}

#[test]
fn test_loop_basic() {
    assert_eq!(eval("(loop (i 0) (if (= i 5) i (recur (+ i 1))))"), "5");
}

#[test]
fn test_loop_fibonacci() {
    assert_eq!(
        eval("(loop (a 0 b 1 cnt 10) (if (= cnt 0) a (recur b (+ a b) (- cnt 1))))"),
        "55"
    );
}

#[test]
fn test_loop_fibonacci_50() {
    // fib(50) — within i64 range
    let result = eval("(loop (a 0 b 1 cnt 50) (if (= cnt 0) a (recur b (+ a b) (- cnt 1))))");
    assert!(!result.starts_with("ERROR"), "got: {}", result);
    assert_eq!(result, "12586269025");
}

#[test]
fn test_loop_sum_range() {
    // Sum 1..100 using loop/recur
    assert_eq!(
        eval("(loop (i 1 sum 0) (if (> i 100) sum (recur (+ i 1) (+ sum i))))"),
        "5050"
    );
}

#[test]
fn test_loop_factorial() {
    assert_eq!(
        eval("(loop (n 10 acc 1) (if (= n 0) acc (recur (- n 1) (* acc n))))"),
        "3628800"
    );
}

#[test]
fn test_recur_outside_loop_errors() {
    let result = eval("(recur 1 2 3)");
    assert!(
        result.contains("recur") || result.starts_with("ERROR"),
        "got: {}",
        result
    );
}

#[test]
fn test_loop_wrong_arity() {
    assert!(eval("(loop (a 0 b 1) (recur 42))").starts_with("ERROR"));
}

#[test]
fn test_loop_with_closures() {
    assert_eq!(
        eval(
            r#"
        (define power
          (lambda (base exp)
            (loop (result 1 e exp)
              (if (= e 0)
                result
                (recur (* result base) (- e 1))))))
        (power 2 10)
    "#
        ),
        "1024"
    );
}

#[test]
fn test_loop_reverse_list() {
    // Use nil? check instead of len on potentially-empty list
    assert_eq!(
        eval(
            r#"
        (define reverse-list
          (lambda (lst)
            (loop (remaining lst acc (list))
              (if (nil? remaining)
                acc
                (recur (cdr remaining) (cons (nth remaining 0) acc))))))
        (reverse-list (list 1 2 3 4 5))
    "#
        ),
        "(5 4 3 2 1)"
    );
}

#[test]
fn test_loop_pair_bindings() {
    assert_eq!(
        eval("(loop ((a 0) (b 1)) (if (= a 5) b (recur (+ a 1) (+ b a))))"),
        "11"
    );
}

#[test]
fn test_loop_countdown() {
    assert_eq!(
        eval("(loop (n 10) (if (= n 0) (quote done) (recur (- n 1))))"),
        "done"
    );
}
