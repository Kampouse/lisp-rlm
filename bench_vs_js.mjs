// Benchmark: lisp-rlm-wasm vs native JS doing the same work

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

function benchJS(name, fn, iters = 10000) {
  fn(); // warmup
  const start = performance.now();
  for (let i = 0; i < iters; i++) fn();
  const ms = performance.now() - start;
  return ms / iters;
}

function benchWasm(name, code, iters = 10000) {
  ev(code); // warmup
  const start = performance.now();
  for (let i = 0; i < iters; i++) ev(code);
  const ms = performance.now() - start;
  return ms / iters;
}

function row(name, wasmMs, jsMs) {
  const ratio = (wasmMs / jsMs).toFixed(1);
  const tag = wasmMs < jsMs ? '🟢' : wasmMs < jsMs * 3 ? '🟡' : '🔴';
  console.log(`${tag} ${name.padEnd(24)} WASM: ${wasmMs.toFixed(3)}ms  JS: ${jsMs.toFixed(3)}ms  ${ratio}x`);
}

console.log("=== lisp-rlm-wasm vs Native JS ===\n");

// 1. Simple arithmetic
row("add(1+2)",
  benchWasm("add", "(+ 1 2)"),
  benchJS("add", () => 1 + 2));

row("mul(6*7)",
  benchWasm("mul", "(* 6 7)"),
  benchJS("mul", () => 6 * 7));

// 2. Closures
row("closure(make-adder)",
  benchWasm("closure", `(begin (define make-adder (lambda (n) (lambda (x) (+ n x)))) ((make-adder 3) 7))`),
  benchJS("closure", () => { const ma = n => x => n + x; return ma(3)(7); }));

// 3. Recursive fib
row("fib(10)",
  benchWasm("fib10", `(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 10))`, 5000),
  benchJS("fib10", () => { const fib = n => n < 2 ? n : fib(n-1) + fib(n-2); return fib(10); }, 5000));

row("fib(20)",
  benchWasm("fib20", `(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 20))`, 1000),
  benchJS("fib20", () => { const fib = n => n < 2 ? n : fib(n-1) + fib(n-2); return fib(20); }, 1000));

// 4. Factorial
row("fact(20)",
  benchWasm("fact20", `(begin (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1)))))) (fact 20))`, 5000),
  benchJS("fact20", () => { const fact = n => n === 0 ? 1 : n * fact(n - 1); return fact(20); }, 5000));

// 5. Map
const list100 = new Array(100).fill(0).map((_, i) => i);
row("map(+1) x100",
  benchWasm("map", `(map (lambda (x) (+ x 1)) (quote ${list100.join(' ')}))`, 5000),
  benchJS("map", () => list100.map(x => x + 1), 5000));

// 6. Filter
row("filter(>50) x100",
  benchWasm("filter", `(filter (lambda (x) (> x 50)) (quote ${list100.join(' ')}))`, 5000),
  benchJS("filter", () => list100.filter(x => x > 50), 5000));

// 7. Reduce
row("reduce(+) x100",
  benchWasm("reduce", `(reduce + 0 (quote ${list100.join(' ')}))`, 5000),
  benchJS("reduce", () => list100.reduce((a, b) => a + b, 0), 5000));

// 8. Reverse
row("reverse x100",
  benchWasm("reverse", `(reverse (quote ${list100.join(' ')}))`, 5000),
  benchJS("reverse", () => [...list100].reverse(), 5000));

// 9. Countdown (tail recursion vs loop)
row("countdown 1000",
  benchWasm("cd1000", `(begin (define cd (lambda (n) (if (= n 0) 0 (cd (- n 1))))) (cd 1000))`, 1000),
  benchJS("cd1000", () => { let n = 1000; while(n > 0) n--; return n; }, 1000));

// 10. String concat
row("str-concat x10",
  benchWasm("strcat", `(str-concat "hello" " " "world" "!" " " "test" " " "string" " " "ops" "!")`, 10000),
  benchJS("strcat", () => "hello" + " " + "world" + "!" + " " + "test" + " " + "string" + " " + "ops" + "!", 10000));

// 11. Parse-only (JS equivalent: JSON.parse or eval)
row("parse quote x10",
  benchWasm("parse", `(quote (a b c d e f g h i j))`, 10000),
  benchJS("parse", () => JSON.parse('["a","b","c","d","e","f","g","h","i","j"]'), 10000));

console.log("\n🟢 = WASM faster  🟡 = WASM within 3x  🔴 = WASM >3x slower");
console.log("Note: WASM includes parse+compile+run each call. JS is JIT-compiled native.");
