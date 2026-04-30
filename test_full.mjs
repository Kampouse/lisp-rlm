import { readFile } from 'fs/promises';
import { fileURLToPath } from 'url';
import path from 'path';

const wasmPath = path.join(path.dirname(fileURLToPath(import.meta.url)),
  'target/wasm32-unknown-unknown/release/lisp_rlm_wasm.wasm');

const wasmBuffer = await readFile(wasmPath);
const { instance } = await WebAssembly.instantiate(wasmBuffer, {});
const memory = instance.exports.memory;

const BASE = 1_000_000;

function evalLisp(code) {
  const bytes = new TextEncoder().encode(code);
  const inputBuf = new Uint8Array(memory.buffer);
  inputBuf.set(bytes, BASE);

  const outLenPtr = BASE + bytes.length + 16;
  const resultPtr = instance.exports.eval_lisp(BASE, bytes.length, outLenPtr);

  if (resultPtr === 0) return "ERROR: null pointer";

  const outLen = new DataView(memory.buffer).getUint32(outLenPtr, true);
  const resultBytes = new Uint8Array(memory.buffer).slice(resultPtr, resultPtr + outLen);
  return new TextDecoder().decode(resultBytes);
}

// Run a sequence of forms, return last result
function runSeq(forms, env_name) {
  // We can't share env across calls (each evalLisp creates fresh env)
  // so we batch everything in one call with begin
  return evalLisp(`(begin ${forms})`);
}

// Test pairs: [expression, expected_result_string]
const tests = [
  // ── Arithmetic ──
  ["(+ 1 2)", "3"],
  ["(+ 1 2 3 4 5)", "15"],
  ["(- 10 3)", "7"],
  ["(* 6 7)", "42"],
  ["(* 2 3 4)", "24"],
  ["(/ 10 2)", "5"],
  ["(mod 10 3)", "1"],
  ["(abs -5)", "5"],
  ["(max 3 7 2)", "7"],
  // ── min is stdlib, skip bare test ──

  // ── Comparison (returns bool: true/false) ──
  ["(> 3 2)", "true"],
  ["(< 3 2)", "false"],
  ["(>= 3 3)", "true"],
  ["(<= 2 3)", "true"],
  ["(= 5 5)", "true"],
  ["(= 5 6)", "false"],
  ["(!= 5 6)", "true"],

  // ── Booleans / predicates (from stdlib via desugar) ──
  ["(and true true)", "true"],
  ["(and true false)", "false"],
  ["(or false true)", "true"],
  ["(or false false)", "false"],
  ["(not false)", "true"],
  ["(not true)", "false"],

  // ── String ops (strings display with quotes) ──
  ["(str-concat \"hello\" \" \" \"world\")", "\"hello world\""],
  ["(str-upcase \"hello\")", "\"HELLO\""],
  ["(str-downcase \"HELLO\")", "\"hello\""],

  // ── List ops ──
  ["(car (quote (1 2 3)))", "1"],
  ["(car '(1 2 3))", "1"],
  ["(cdr (quote (1 2 3)))", "(2 3)"],
  ["(cons 1 (quote (2 3)))", "(1 2 3)"],
  ["(list 1 2 3)", "(1 2 3)"],
  ["(len (quote (1 2 3)))", "3"],
  ["(nil? (quote ()))", "false"], // empty list ≠ nil in lisp-rlm
  ["(nil? nil)", "true"],
  ["(nil? 42)", "false"],
  ["(append (quote (1 2)) (quote (3 4)))", "(1 2 3 4)"],
  ["(reverse (quote (1 2 3)))", "(3 2 1)"],
  ["(map (lambda (x) (+ x 1)) (quote (1 2 3)))", "(2 3 4)"],
  ["(filter (lambda (x) (> x 2)) (quote (1 2 3 4)))", "(3 4)"],

  // ── Define / begin ──
  ["(begin (define x 42) x)", "42"],
  ["(begin (define x 10) (define y 20) (+ x y))", "30"],

  // ── Lambda / closures ──
  ["((lambda (x) (+ x 1)) 5)", "6"],
  ["(begin (define add1 (lambda (x) (+ x 1))) (add1 10))", "11"],
  ["(begin (define make-adder (lambda (n) (lambda (x) (+ n x)))) ((make-adder 3) 7))", "10"],

  // ── Let ──
  ["(let ((x 3) (y 4)) (+ x y))", "7"],

  // ── If / cond (if is special form via desugar) ──
  ["(if true 1 2)", "1"],
  ["(if false 1 2)", "2"],
  ["(if (> 5 3) \"yes\" \"no\")", "\"yes\""],

  // ── Recursive function ──
  ["(begin (define fib (lambda (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))) (fib 10))", "55"],

  // ── factorial ──
  ["(begin (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1)))))) (fact 12))", "479001600"],

  // ── Type predicates (return bool) ──
  ["(number? 42)", "true"],
  ["(number? \"hi\")", "false"],
  ["(string? \"hi\")", "true"],
  ["(nil? nil)", "true"],
  ["(list? (quote (1 2)))", "true"],

  // ── Lenient behavior (not errors, returns nil/0) ──
  ["(car 42)", "nil"],
  ["(+ 1 \"two\")", "0"],

  // ── Error cases ──
  ["garbage)))", s => s.startsWith("PARSE_ERROR")],

  // ── Nested structures ──
  ["(quote (a (b c) d))", "(a (b c) d)"],
  ["(car (cdr (quote (1 2 3))))", "2"],

  // ── Higher-order ──
  ["(begin (define apply-twice (lambda (f x) (f (f x)))) (apply-twice (lambda (x) (+ x 3)) 5))", "11"],
  ["(reduce + 0 (quote (1 2 3 4 5)))", "15"],
];

console.log(`Testing lisp-rlm-wasm (${tests.length} cases):\n`);

let passed = 0, failed = 0;
const failures = [];

for (const [code, expected] of tests) {
  const result = evalLisp(code);
  const ok = typeof expected === 'function' ? expected(result) : result === expected;
  if (ok) {
    passed++;
    console.log(`✅ ${code}`);
  } else {
    failed++;
    failures.push(code);
    console.log(`❌ ${code} → ${result} (expected: ${expected})`);
  }
}

console.log(`\n${passed}/${tests.length} passed, ${failed} failed`);
if (failures.length) console.log(`\nFailed: ${failures.join(', ')}`);
process.exit(failed > 0 ? 1 : 0);
