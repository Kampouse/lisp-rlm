import { readFile } from 'fs/promises';

const buf = await readFile('target/wasm32-unknown-unknown/release/lisp_rlm_wasm.wasm');
const { instance } = await WebAssembly.instantiate(buf, {});
const mem = instance.exports.memory;
const BASE = 1_000_000;

function ev(code) {
  const b = new TextEncoder().encode(code);
  new Uint8Array(mem.buffer).set(b, BASE);
  const p = instance.exports.eval_lisp(BASE, b.length, BASE + b.length + 16);
  const len = new DataView(mem.buffer).getUint32(BASE + b.length + 16, true);
  return new TextDecoder().decode(new Uint8Array(mem.buffer).slice(p, p + len));
}

function bench(name, code, iterations = 1) {
  // warmup
  ev(code);
  const start = performance.now();
  for (let i = 0; i < iterations; i++) ev(code);
  const ms = performance.now() - start;
  const per = (ms / iterations).toFixed(3);
  console.log(`${name}: ${per}ms/op (${iterations} iters, total ${ms.toFixed(1)}ms)`);
  return ms / iterations;
}

console.log("=== lisp-rlm-wasm BENCHMARKS ===\n");

// 1. Simple arithmetic
bench("add(1+2)", "(+ 1 2)", 10000);
bench("mul(6*7)", "(* 6 7)", 10000);

// 2. Parse-heavy
bench("parse-only", "(quote (a b c d e f g h i j))", 10000);

// 3. Closures
bench("closure(make-adder)", 
  `(begin (define make-adder (lambda (n) (lambda (x) (+ n x)))) ((make-adder 3) 7))`, 10000);

// 4. Recursive fib
bench("fib(10)", 
  `(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 10))`, 5000);
bench("fib(20)", 
  `(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 20))`, 1000);
bench("fib(25)", 
  `(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 25))`, 100);

// 5. Factorial
bench("fact(20)", 
  `(begin (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1)))))) (fact 20))`, 5000);

// 6. Higher-order (map)
bench("map(+1) x100", 
  `(map (lambda (x) (+ x 1)) (quote ${(new Array(100).fill(0).map((_,i)=>i).join(' '))}))`, 5000);

// 7. Higher-order (filter)
bench("filter(>50) x100", 
  `(filter (lambda (x) (> x 50)) (quote ${(new Array(100).fill(0).map((_,i)=>i).join(' '))}))`, 5000);

// 8. Reduce
bench("reduce(+) x100", 
  `(reduce + 0 (quote ${(new Array(100).fill(0).map((_,i)=>i).join(' '))}))`, 5000);

// 9. List operations
bench("reverse x100", 
  `(reverse (quote ${(new Array(100).fill(0).map((_,i)=>i).join(' '))}))`, 5000);
bench("append x50+x50", 
  `(append (quote ${(new Array(50).fill(0).map((_,i)=>i).join(' '))}) (quote ${(new Array(50).fill(0).map((_,i)=>i+50).join(' '))}))`, 5000);

// 10. Let nesting
bench("nested-let x5", 
  `(let ((a 1)) (let ((b (+ a 1))) (let ((c (+ b 1))) (let ((d (+ c 1))) (let ((e (+ d 1))) (+ a b c d e))))))`, 10000);

// 11. String ops
bench("str-concat x10", 
  `(str-concat "hello" " " "world" "!" " " "test" " " "string" " " "ops" "!")`, 10000);

// 12. Deep recursion (tail call test)
bench("countdown 100", 
  `(begin (define countdown (lambda (n) (if (= n 0) 0 (countdown (- n 1))))) (countdown 100))`, 5000);
bench("countdown 1000", 
  `(begin (define countdown (lambda (n) (if (= n 0) 0 (countdown (- n 1))))) (countdown 1000))`, 1000);

// 13. Mandelbrot-style number crunching
bench("num-crunch (* 100 ops)", 
  `(begin ${Array.from({length:100}, (_,i) => `(* ${i} ${i+1})`).join(' ')})`, 1000);

console.log("\n=== DONE ===");
