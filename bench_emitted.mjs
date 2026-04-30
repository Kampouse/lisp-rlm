import { readFile as readFileAsync } from 'fs/promises';
import { execSync } from 'child_process';
import { writeFileSync } from 'fs';

// First, use the interpreter WASM to compile Lisp → WAT
// Then use wat2wasm to compile WAT → WASM
// Then benchmark

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

// Generate WAT for fib
const fibWat = `(module
  (memory (export "memory") 1)
  (func $fib (param $n i64) (result i64)
    (local.get $n)
    (i64.const 2)
    (i64.lt_s)
    (if (result i64)
      (then (local.get $n))
      (else
        (local.get $n) (i64.const 1) (i64.sub) (call $fib)
        (local.get $n) (i64.const 2) (i64.sub) (call $fib)
        (i64.add))))
  (func (export "fib") (param i64) (result i64)
    (call $fib (local.get 0)))
)`;

// Generate WAT for factorial
const factWat = `(module
  (memory (export "memory") 1)
  (func $fact (param $n i64) (result i64)
    (local.get $n)
    (i64.const 0)
    (i64.eq)
    (if (result i64)
      (then (i64.const 1))
      (else
        (local.get $n)
        (local.get $n) (i64.const 1) (i64.sub) (call $fact)
        (i64.mul))))
  (func (export "fact") (param i64) (result i64)
    (call $fact (local.get 0)))
)`;

// Generate WAT for countdown (tail recursive)
const countdownWat = `(module
  (memory (export "memory") 1)
  (func $countdown (param $n i64) (result i64)
    (local.get $n)
    (i64.const 0)
    (i64.eq)
    (if (result i64)
      (then (i64.const 0))
      (else
        (local.get $n) (i64.const 1) (i64.sub) (call $countdown))))
  (func (export "countdown") (param i64) (result i64)
    (call $countdown (local.get 0)))
)`;

// Compile WAT to WASM using wat2wasm
function compileWat(name, wat) {
  const watFile = `/tmp/${name}.wat`;
  const wasmFile = `/tmp/${name}.wasm`;
  writeFileSync(watFile, wat);
  try {
    execSync(`wat2wasm ${watFile} -o ${wasmFile}`, { stdio: 'pipe' });
  } catch (e) {
    console.error(`wat2wasm failed for ${name}:`, e.stderr?.toString());
    return null;
  }
  return wasmFile;
}

async function loadWasm(wasmFile) {
  const buf = await readFileAsync(wasmFile);
  const { instance } = await WebAssembly.instantiate(buf, {});
  return instance;
}

function bench(name, fn, iters = 10000) {
  fn(); // warmup
  const start = performance.now();
  for (let i = 0; i < iters; i++) fn();
  return (performance.now() - start) / iters;
}

console.log("=== Emitted WASM vs Interpreter vs Native JS ===\n");

// Compile WAT files
const fibWasmFile = compileWat('fib_pure', fibWat);
const factWasmFile = compileWat('fact_pure', factWat);
const cdWasmFile = compileWat('countdown_pure', countdownWat);

if (!fibWasmFile || !factWasmFile || !cdWasmFile) {
  console.log("wat2wasm not found. Install: brew install wabt");
  process.exit(1);
}

const fibEmitted = await loadWasm(fibWasmFile);
const factEmitted = await loadWasm(factWasmFile);
const cdEmitted = await loadWasm(cdWasmFile);

function row(name, emitted, interp, js) {
  const ratioE = (emitted / js).toFixed(2);
  const ratioI = (interp / js).toFixed(1);
  console.log(`${name.padEnd(28)} Emitted: ${emitted.toFixed(4)}ms  Interp: ${interp.toFixed(3)}ms  JS: ${js.toFixed(4)}ms  (E=${ratioE}x, I=${ratioI}x)`);
}

// --- fib ---
row("fib(10)",
  bench("fib10 emitted", () => fibEmitted.exports.fib(10n), 50000),
  bench("fib10 interp", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 10))`), 5000),
  bench("fib10 js", () => { const f = n => n < 2n ? n : f(n-1n) + f(n-2n); return f(10n); }, 50000));

row("fib(20)",
  bench("fib20 emitted", () => fibEmitted.exports.fib(20n), 10000),
  bench("fib20 interp", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 20))`), 1000),
  bench("fib20 js", () => { const f = n => n < 2n ? n : f(n-1n) + f(n-2n); return f(20n); }, 10000));

row("fib(25)",
  bench("fib25 emitted", () => fibEmitted.exports.fib(25n), 2000),
  bench("fib25 interp", () => ev(`(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 25))`), 200),
  bench("fib25 js", () => { const f = n => n < 2n ? n : f(n-1n) + f(n-2n); return f(25n); }, 2000));

// --- factorial ---
row("fact(20)",
  bench("fact20 emitted", () => factEmitted.exports.fact(20n), 50000),
  bench("fact20 interp", () => ev(`(begin (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1)))))) (fact 20))`), 5000),
  bench("fact20 js", () => { const f = n => n === 0n ? 1n : n * f(n-1n); return f(20n); }, 50000));

// --- countdown ---
row("countdown(1000)",
  bench("cd1k emitted", () => cdEmitted.exports.countdown(1000n), 10000),
  bench("cd1k interp", () => ev(`(begin (define cd (lambda (n) (if (= n 0) 0 (cd (- n 1))))) (cd 1000))`), 1000),
  bench("cd1k js", () => { let n = 1000n; while(n > 0n) n--; return n; }, 10000));

row("countdown(10000)",
  bench("cd10k emitted", () => cdEmitted.exports.countdown(10000n), 5000),
  bench("cd10k interp", () => ev(`(begin (define cd (lambda (n) (if (= n 0) 0 (cd (- n 1))))) (cd 10000))`), 200),
  bench("cd10k js", () => { let n = 10000n; while(n > 0n) n--; return n; }, 5000));

console.log("\nE = emitted WASM / JS speed ratio, I = interpreter / JS speed ratio");
