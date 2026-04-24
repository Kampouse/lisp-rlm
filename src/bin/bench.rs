use lisp_rlm::*;

fn main() {
    let mut env = Env::new();

    // Bench: 100k iterations
    let prog = "(loop ((i 0) (sum 0)) (if (>= i 100000) sum (recur (+ i 1) (+ sum i))))";
    let exprs = parse_all(prog).unwrap();
    let start = std::time::Instant::now();
    let result = lisp_eval(&exprs[0], &mut env);
    println!(
        "100k loop: {:?} ({}µs)",
        result,
        start.elapsed().as_micros()
    );

    // Correctness: sum 0..10
    let prog2 = "(loop ((i 0) (sum 0)) (if (>= i 10) sum (recur (+ i 1) (+ sum i))))";
    let e2 = parse_all(prog2).unwrap();
    println!("sum 0..10 = {:?}", lisp_eval(&e2[0], &mut env));

    // Correctness: fib(20)
    let prog3 = "(loop ((n 20) (a 0) (b 1)) (if (<= n 0) a (recur (- n 1) b (+ a b))))";
    let e3 = parse_all(prog3).unwrap();
    println!("fib(20) = {:?}", lisp_eval(&e3[0], &mut env));

    // Bench: 1M iterations
    let prog4 = "(loop ((i 0) (sum 0)) (if (>= i 1000000) sum (recur (+ i 1) (+ sum i))))";
    let e4 = parse_all(prog4).unwrap();
    let start4 = std::time::Instant::now();
    let r4 = lisp_eval(&e4[0], &mut env);
    println!("1M loop: {:?} ({}ms)", r4, start4.elapsed().as_millis());

    // Bench: 10M iterations (stress test)
    let prog5 = "(loop ((i 0) (sum 0)) (if (>= i 10000000) sum (recur (+ i 1) (+ sum i))))";
    let e5 = parse_all(prog5).unwrap();
    let start5 = std::time::Instant::now();
    let r5 = lisp_eval(&e5[0], &mut env);
    println!("10M loop: {:?} ({}ms)", r5, start5.elapsed().as_millis());
}
