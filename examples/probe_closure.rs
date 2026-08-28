use lisp_rlm_wasm::wasm_emit::compile_fuzz;

fn check(name: &str, src: &str) {
    let wasm = match compile_fuzz(src) {
        Ok(w) => w,
        Err(e) => { println!("{name}: COMPILE ERR: {}", e); return; }
    };
    match wasmtime::Module::new(&wasmtime::Engine::default(), &wasm) {
        Ok(_) => println!("{name}: module validates ({})", wasm.len()),
        Err(e) => println!("{name}: MODULE ERR: {}", e),
    }
}

fn main() {
    // n_lambdas = 1
    check("single", "(define (make-adder n) (lambda (x) (+ x n)))\n(define (run) ((make-adder 10) 5))");
    // n_lambdas = 2 — nested else-chain depth
    check("double", "(define (make-adder n) (lambda (x) (+ x n)))\n(define (make-mul n) (lambda (x) (* x n)))\n(define (run) (+ ((make-adder 10) 5) ((make-mul 3) 4)))");
    // n_lambdas = 5 — deep chain
    check("five", "(define f1 (lambda (x) (+ x 1)))\n(define f2 (lambda (x) (+ x 2)))\n(define f3 (lambda (x) (+ x 3)))\n(define f4 (lambda (x) (+ x 4)))\n(define f5 (lambda (x) (+ x 5)))\n(define (run) (+ (+ (+ (f1 0) (f2 0)) (+ (f3 0) (f4 0))) (f5 0)))");
    // set! closure
    check("counter", "(define (make-counter init) (let ((n init)) (lambda () (set! n (+ n 1)) n)))\n(define (run) ((make-counter 5)))");
}
