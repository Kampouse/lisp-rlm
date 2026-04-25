use lisp_rlm::{parse_all, lisp_eval, Env};

fn eval(s: &str, env: &mut Env) -> String {
    let exprs = parse_all(s).unwrap();
    let mut r = "nil".to_string();
    for e in &exprs { match lisp_eval(e, env) { Ok(v) => r = format!("{:?}", v), Err(e) => r = format!("ERR: {}", e) } }
    r
}

fn main() {
    let mut env = Env::new();

    // 2-arg inline lambda
    println!("2-arg call: {}", eval("((lambda (a b) (+ a b)) 3 4)", &mut env));

    // Y1 (standard, 1-arg eta)
    println!("\n--- Y1 (1-arg eta) ---");
    let y1 = r#"
(define Y1
  (lambda (f)
    ((lambda (x) (f (lambda (v) ((x x) v))))
     (lambda (x) (f (lambda (v) ((x x) v)))))))
"#;
    println!("Y1: {}", eval(y1, &mut env));

    // Y1 factorial (1-arg, works)
    let fact = r#"(define fact (Y1 (lambda (self) (lambda (n) (if (= n 0) 1 (* n (self (- n 1))))))))"#;
    println!("fact: {}", eval(fact, &mut env));
    println!("fact(10): {}", eval("(fact 10)", &mut env));

    // Y1 with 2-arg function (should fail - eta only passes 1 arg)
    let gcd1 = r#"(define gcd1 (Y1 (lambda (self) (lambda (a b) (if (= b 0) a (self b (mod a b)))))))"#;
    println!("\ngcd1 (Y1, 2-arg): {}", eval(gcd1, &mut env));
    println!("gcd1(12,8): {}", eval("(gcd1 12 8)", &mut env));

    // Y2 (2-arg eta)
    println!("\n--- Y2 (2-arg eta) ---");
    let y2 = r#"
(define Y2
  (lambda (f)
    ((lambda (x) (f (lambda (a b) ((x x) a b))))
     (lambda (x) (f (lambda (a b) ((x x) a b)))))))
"#;
    println!("Y2: {}", eval(y2, &mut env));

    let gcd2 = r#"(define gcd2 (Y2 (lambda (self) (lambda (a b) (if (= b 0) a (self b (mod a b)))))))"#;
    println!("gcd2: {}", eval(gcd2, &mut env));
    println!("gcd2(12,8): {}", eval("(gcd2 12 8)", &mut env));
    println!("gcd2(1071,462): {}", eval("(gcd2 1071 462)", &mut env));

    // Y3 (3-arg eta) — for foldl
    println!("\n--- Y3 (3-arg eta) ---");
    let y3 = r#"
(define Y3
  (lambda (f)
    ((lambda (x) (f (lambda (a b c) ((x x) a b c))))
     (lambda (x) (f (lambda (a b c) ((x x) a b c)))))))
"#;
    println!("Y3: {}", eval(y3, &mut env));

    let foldl = r#"(define foldl-y (Y3 (lambda (self) (lambda (f acc lst) (if (nil? lst) acc (self f (f acc (car lst)) (cdr lst)))))))"#;
    println!("foldl-y: {}", eval(foldl, &mut env));
    println!("foldl sum 1..5: {}", eval("(foldl-y + 0 (list 1 2 3 4 5))", &mut env));
    println!("foldl sum 1..100: {}", eval("(foldl-y + 0 (range 1 101))", &mut env));
    println!("foldl * 1..6: {}", eval("(foldl-y * 1 (range 1 7))", &mut env));
}
