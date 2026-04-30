import { readFile as readFileAsync } from 'fs/promises';

async function loadWasm(path) {
  const buf = await readFileAsync(path);
  const { instance } = await WebAssembly.instantiate(buf, {});
  return instance;
}

function bench(name, fn, iters = 50000) {
  fn(); // warmup
  const start = performance.now();
  for (let i = 0; i < iters; i++) fn();
  return (performance.now() - start) / iters;
}

// Load interpreter
const interpBuf = await readFileAsync('target/wasm32-unknown-unknown/release/lisp_rlm_wasm.wasm');
const { instance: interp } = await WebAssembly.instantiate(interpBuf, {});
const mem = interp.exports.memory;
const BASE = 1_000_000;

function ev(code) {
  const b = new TextEncoder().encode(code);
  new Uint8Array(mem.buffer).set(b, BASE);
  const p = interp.exports.eval_lisp(BASE, b.length, BASE + b.length + 16);
  const len = new DataView(mem.buffer).getUint32(BASE + b.length + 16, true);
  return new TextDecoder().decode(new Uint8Array(mem.buffer).slice(p, p + len));
}

// Load emitted WASM
const fibE = await loadWasm('/tmp/fib_emitted.wasm');
const factE = await loadWasm('/tmp/fact_emitted.wasm');
const cdE = await loadWasm('/tmp/countdown_emitted.wasm');
const gcdE = await loadWasm('/tmp/gcd_fixed.wasm');
const sumsqE = await loadWasm('/tmp/sumsq_fixed.wasm');

console.log("=== Emitted WASM (from our emitter) Benchmarks ===\n");

function row(name, emitted, interp, js) {
  const eVsJs = (emitted / js).toFixed(2);
  const iVsJs = (interp / js).toFixed(1);
  const eVsI = (interp / emitted).toFixed(0);
  console.log(`${name.padEnd(24)} Emitted: ${emitted.toFixed(4)}ms  Interp: ${interp.toFixed(3)}ms  JS: ${js.toFixed(4)}ms  | E/J=${eVsJs}x  I/J=${iVsJs}x  I/E=${eVsI}x`);
}

// fib
row("fib(10)",
  bench("e", () => fibE.exports.run(10n)),
  bench("i", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 10))`), 5000),
  bench("j", () => { const f=n=>n<2n?n:f(n-1n)+f(n-2n); return f(10n); }));

row("fib(20)",
  bench("e", () => fibE.exports.run(20n), 10000),
  bench("i", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 20))`), 1000),
  bench("j", () => { const f=n=>n<2n?n:f(n-1n)+f(n-2n); return f(20n); }, 10000));

row("fib(25)",
  bench("e", () => fibE.exports.run(25n), 2000),
  bench("i", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 25))`), 200),
  bench("j", () => { const f=n=>n<2n?n:f(n-1n)+f(n-2n); return f(25n); }, 2000));

// fact
row("fact(20)",
  bench("e", () => factE.exports.run(20n)),
  bench("i", () => ev(`(begin (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1)))))) (fact 20))`), 5000),
  bench("j", () => { const f=n=>n===0n?1n:n*f(n-1n); return f(20n); }));

// countdown
row("countdown(1K)",
  bench("e", () => cdE.exports.run(1000n)),
  bench("i", () => ev(`(begin (define cd (lambda (n) (if (= n 0) 0 (cd (- n 1))))) (cd 1000))`), 1000),
  bench("j", () => { let n=1000n; while(n>0n)n--; return n; }));

row("countdown(10K)",
  bench("e", () => cdE.exports.run(10000n), 10000),
  bench("i", () => ev(`(begin (define cd (lambda (n) (if (= n 0) 0 (cd (- n 1))))) (cd 10000))`), 200),
  bench("j", () => { let n=10000n; while(n>0n)n--; return n; }, 10000));

// gcd
row("gcd(48,18)",
  bench("e", () => gcdE.exports.gcd(48n, 18n)),
  bench("i", () => ev(`(begin (define gcd (lambda (a b) (if (= b 0) a (gcd b (mod a b))))) (gcd 48 18))`), 5000),
  bench("j", () => { const g=(a,b)=>b===0n?a:g(b,a%b); return g(48n,18n); }));

// sum-squares
row("sum_sq(3,4)",
  bench("e", () => sumsqE.exports.sum_squares(3n, 4n)),
  bench("i", () => ev(`(begin (define square (lambda (x) (* x x))) (define sum-squares (lambda (a b) (+ (square a) (square b)))) (sum-squares 3 4))`), 5000),
  bench("j", () => { const sq=x=>x*x; return sq(3n)+sq(4n); }));

console.log("\nE/J = emitted vs JS  |  I/J = interpreter vs JS  |  I/E = interpreter vs emitted (speedup)");
