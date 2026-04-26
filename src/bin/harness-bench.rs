//! Harness tick throughput benchmark
//!
//! Run: cargo run --bin harness-bench --release

use lisp_rlm::EvalState;
use lisp_rlm::*;

fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_nanos() as f64 / 1_000_000.0
}

fn main() {
    let harness_code = r#"
        (define (get-default m key default)
            (let ((v (dict/get m key)))
                (if (nil? v) default v)))

        (define (urgency intent)
            (let ((deadline (get-default intent "deadline" nil))
                  (last (get-default intent "last-acted" nil))
                  (t0 (now)))
                (cond
                    ((and deadline (> t0 deadline)) 1.0)
                    ((and deadline (< (- deadline t0) 3600000)) 0.9)
                    ((and last (> (elapsed last) 3600000)) 0.7)
                    (t 0.3))))

        (define (cost-efficiency intent)
            (let ((cost (get-default intent "cost" 1)))
                (cond
                    ((= cost 0) 1.0)
                    ((< cost 10) 0.9)
                    ((< cost 100) 0.6)
                    (t 0.3))))

        (define (score-intention intent)
            (let ((u (urgency intent))
                  (e (cost-efficiency intent))
                  (score (+ (* 0.7 u) (* 0.3 e))))
                (dict/set intent "score" score)))

        (define (rank-intentions intentions)
            (sort (map score-intention intentions)
                  (lambda (a b) (> (get-default a "score" 0) (get-default b "score" 0)))))

        (define (execute-action intent)
            (get-default intent "id" "?"))

        (define (handle-result intent result)
            (let ((itype (get-default intent "type" "one-shot")))
                (cond
                    ((equal? itype "perpetual")
                     (dict/set intent "last-acted" (now)))
                    ((equal? itype "one-shot")
                     (list intent result))
                    (t (dict/set intent "last-run" (now))))))

        (define (scheduler-run intentions)
            (let ((ranked (rank-intentions intentions)))
                (map (lambda (intent)
                    (let ((result (execute-action intent)))
                        (handle-result intent result)))
                    ranked)))

        (define *intentions*
            (list
                (dict "id" "task-1" "type" "completable" "cost" 5 "deadline" 1)
                (dict "id" "task-2" "type" "perpetual" "cost" 50)
                (dict "id" "task-3" "type" "one-shot" "cost" 0)
                (dict "id" "task-4" "type" "recurring" "cost" 10 "last-acted" 1000.0)
                (dict "id" "task-5" "type" "completable" "cost" 200)
                (dict "id" "task-6" "type" "one-shot" "cost" 3 "deadline" 1)
                (dict "id" "task-7" "type" "perpetual" "cost" 15)
                (dict "id" "task-8" "type" "recurring" "cost" 8)
                (dict "id" "task-9" "type" "completable" "cost" 1)
                (dict "id" "task-10" "type" "one-shot" "cost" 100 "deadline" 1)))
    "#;

    // Setup
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = parse_all(harness_code).unwrap();
    for expr in &exprs {
        lisp_eval(expr, &mut env, &mut state).unwrap();
    }

    println!("=== Harness Benchmark ===\n");

    // --- Compilation stats ---
    println!("--- Compilation stats ---");
    for name in &["get-default", "urgency", "cost-efficiency", "score-intention",
                   "handle-result", "execute-action", "rank-intentions", "scheduler-run"] {
        match env.get(name) {
            Some(LispVal::Lambda { compiled: Some(_), .. }) => println!("  {} -> compiled (bytecode)", name),
            Some(LispVal::Lambda { compiled: None, .. }) => println!("  {} -> NOT compiled (tree-walk)", name),
            _ => println!("  {} -> not found", name),
        }
    }
    println!();

    // --- Benchmark 1: score-intention (single intent) ---
    {
        let n = 10000;
        let prog = r#"(score-intention (dict "id" "bench" "type" "completable" "cost" 5 "deadline" 1))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("score-intention: {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 2: get-default in isolation ---
    {
        let n = 100000;
        let prog = r#"(get-default (dict "a" 1 "b" 2 "c" 3) "b" 99)"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("get-default: {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 3: rank-intentions (10 intents: map+sort) ---
    {
        let n = 5000;
        let prog = r#"(rank-intentions *intentions*)"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("rank-intentions (10 intents): {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 4: full scheduler-run (10 intents) ---
    {
        let n = 2000;
        let prog = r#"(scheduler-run *intentions*)"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        let ticks_per_sec = n as f64 / (ms / 1000.0);
        println!("scheduler-run (10 intents): {} ticks in {:.1}ms ({:.0} ticks/sec, {:.2}ms/tick)",
                 n, ms, ticks_per_sec, ms / n as f64);
    }

    // --- Benchmark 5: map HOF fast path (10-elem list) ---
    {
        let n = 10000;
        let prog = r#"(map (lambda (x) (+ x 1)) (list 1 2 3 4 5 6 7 8 9 10))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        let total_elems = n * 10;
        println!("map (10-elem list): {} iterations in {:.1}ms ({:.0} elem/sec)", n, ms, total_elems as f64 / (ms / 1000.0));
    }

    // --- Benchmark 6: filter HOF fast path ---
    {
        let n = 10000;
        let prog = r#"(filter (lambda (x) (> x 5)) (list 1 2 3 4 5 6 7 8 9 10))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("filter (10-elem list): {} iterations in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 7: for-each HOF fast path ---
    {
        let n = 10000;
        let prog = r#"(for-each (lambda (x) (+ x 1)) (list 1 2 3 4 5 6 7 8 9 10))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("for-each (10-elem list): {} iterations in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 8: dict/get raw ---
    {
        let n = 100000;
        let prog = r#"(dict/get (dict "x" 42) "x")"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("dict/get (raw eval): {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 9: urgency alone ---
    {
        let n = 10000;
        let prog = r#"(urgency (dict "id" "bench" "deadline" 1 "cost" 5))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("urgency: {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 10: cost-efficiency alone ---
    {
        let n = 10000;
        let prog = r#"(cost-efficiency (dict "id" "bench" "cost" 5))"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("cost-efficiency: {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 11: handle-result ---
    {
        let n = 10000;
        let prog = r#"(handle-result (dict "id" "bench" "type" "one-shot" "cost" 5) "ok")"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        println!("handle-result: {} calls in {:.1}ms ({:.0} calls/sec)", n, ms, n as f64 / (ms / 1000.0));
    }

    // --- Benchmark 12: 100-intent scheduler-run (scaled up) ---
    {
        // Build 100 intentions
        let mut intent_code = String::from("(list ");
        for i in 0..100 {
            let cost = i % 200;
            let itype = match i % 4 {
                0 => "completable",
                1 => "perpetual",
                2 => "one-shot",
                _ => "recurring",
            };
            intent_code.push_str(&format!(
                "(dict \"id\" \"task-{}\" \"type\" \"{}\" \"cost\" {})",
                i, itype, cost
            ));
        }
        intent_code.push_str(")");

        // Store as *big-intentions*
        let store_code = format!("(define *big-intentions* {})", intent_code);
        let store_exprs = parse_all(&store_code).unwrap();
        for expr in &store_exprs {
            let _ = lisp_eval(expr, &mut env, &mut state);
        }

        let n = 500;
        let prog = r#"(scheduler-run *big-intentions*)"#;
        let exprs = parse_all(prog).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = lisp_eval(&exprs[0], &mut env, &mut state);
        }
        let ms = elapsed_ms(start);
        let ticks_per_sec = n as f64 / (ms / 1000.0);
        println!("scheduler-run (100 intents): {} ticks in {:.1}ms ({:.0} ticks/sec, {:.2}ms/tick)",
                 n, ms, ticks_per_sec, ms / n as f64);
    }

    println!("\n=== Done ===");
}
