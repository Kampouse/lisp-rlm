/// Comparative benchmark: lisp-rlm vs Guile baseline
/// cargo run --release --bin bench-compare
use std::time::Instant;

fn time_it<F: FnMut()>(label: &str, mut thunk: F, iterations: usize) -> f64 {
    thunk(); // warmup
    let start = Instant::now();
    for _ in 0..iterations {
        thunk();
    }
    let elapsed = start.elapsed().as_secs_f64();
    let per_sec = iterations as f64 / elapsed;
    println!(
        "{}: {} calls in {:.1}ms ({:.0} calls/sec)",
        label,
        iterations,
        elapsed * 1000.0,
        per_sec
    );
    per_sec
}

fn main() {
    use lisp_rlm::{lisp_eval, parse_all, Env, EvalState, LispVal};

    fn eval_all(code: &str, env: &mut Env) -> LispVal {
        let parsed = parse_all(code).unwrap();
        let mut state = EvalState::new();
        state.eval_budget = 100_000_000;
        let mut result = LispVal::Nil;
        for form in parsed {
            result = lisp_eval(&form, env, &mut state).unwrap();
        }
        result
    }

    let mut env = Env::new();

    println!("\n=== lisp-rlm Bytecode Benchmarks ===\n");

    eval_all(
        r#"
(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(define (get-default m key default)
  (let ((found (dict/get m key)))
    (if (nil? found) default found)))
(define (score-intention item)
  (let* ((urgency (get-default item "urgency" 0.5))
         (cost (get-default item "cost" 1.0))
         (score (* urgency cost)))
    (dict/set item "score" score)))
(define (map-test lst) (map (lambda (x) (* x x)) lst))
(define (filter-test lst) (filter (lambda (x) (> x 5)) lst))
(define (sort-test lst) (sort lst <))
(define (loop-sum n)
  (loop ((i 0) (sum 0))
    (if (>= i n) sum (recur (+ i 1) (+ sum i)))))
(define (dict-chain n)
  (loop ((i 0) (m (dict)))
    (if (>= i n) m (recur (+ i 1) (dict/set m (to-string i) i)))))
"#,
        &mut env,
    );

    // fib(28) — fast enough to not dominate
    time_it(
        "fib(28)",
        || {
            eval_all("(fib 28)", &mut env);
        },
        3,
    );

    // get-default
    eval_all(
        "(define test-dict (dict \"a\" 1 \"b\" 2 \"c\" 3))",
        &mut env,
    );
    time_it(
        "get-default",
        || {
            eval_all("(get-default test-dict \"b\" 0)", &mut env);
        },
        1_000_000,
    );

    // score-intention
    eval_all(
        "(define test-item (dict \"urgency\" 0.8 \"cost\" 0.5))",
        &mut env,
    );
    time_it(
        "score-intention",
        || {
            eval_all("(score-intention test-item)", &mut env);
        },
        100_000,
    );

    // map(100-elem)
    eval_all("(define test-list (range 0 100))", &mut env);
    time_it(
        "map(100-elem)",
        || {
            eval_all("(map-test test-list)", &mut env);
        },
        10_000,
    );

    // filter(100-elem)
    time_it(
        "filter(100-elem)",
        || {
            eval_all("(filter-test test-list)", &mut env);
        },
        10_000,
    );

    // sort(100-elem)
    eval_all("(define test-sort-list (reverse (range 0 100)))", &mut env);
    time_it(
        "sort(100-elem)",
        || {
            eval_all("(sort-test test-sort-list)", &mut env);
        },
        10_000,
    );

    // loop-sum(1000)
    time_it(
        "loop-sum(1000)",
        || {
            eval_all("(loop-sum 1000)", &mut env);
        },
        10_000,
    );

    // dict-chain(100)
    time_it(
        "dict-chain(100)",
        || {
            eval_all("(dict-chain 100)", &mut env);
        },
        10_000,
    );

    println!("\n=== Done ===");
}
