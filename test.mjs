import { readFile } from 'fs/promises';
import { fileURLToPath } from 'url';
import path from 'path';

const wasmPath = path.join(path.dirname(fileURLToPath(import.meta.url)),
  'target/wasm32-unknown-unknown/release/lisp_rlm_wasm.wasm');

const wasmBuffer = await readFile(wasmPath);
const { instance } = await WebAssembly.instantiate(wasmBuffer, {});

const memory = instance.exports.memory;

function evalLisp(code) {
  const bytes = new TextEncoder().encode(code);
  const inputPtr = instance.exports.__wasm_exported_allocate !== undefined
    ? null : 0;

  // Allocate input in WASM memory
  const inputBase = 0; // we'll write directly into memory
  const mem = new DataView(memory.buffer);
  const inputOffset = memory.buffer.byteLength; // won't work, need alloc

  // Actually, we need to put bytes somewhere in memory.
  // Simple approach: use the stack area. The WASM linear memory grows,
  // let's write after the static data. Use a high offset.
  const BASE = 1_000_000; // safe offset in linear memory

  const inputBuf = new Uint8Array(memory.buffer);
  inputBuf.set(bytes, BASE);

  const outLenPtr = BASE + bytes.length + 16;
  // Ensure outLenPtr aligned
  const resultPtr = instance.exports.eval_lisp(BASE, bytes.length, outLenPtr);

  if (resultPtr === 0) return "ERROR: null pointer returned";

  const outLen = new DataView(memory.buffer).getUint32(outLenPtr, true);
  const resultBytes = new Uint8Array(memory.buffer).slice(resultPtr, resultPtr + outLen);
  return new TextDecoder().decode(resultBytes);
}

// Tests
const tests = [
  ["(+ 1 2)", "3"],
  ["(* 6 7)", "42"],
  ["(define x 10)", "10"],
  ["(if (> 3 2) \"yes\" \"no\")", "yes"],
  ["(lambda (x) (+ x 1))", fn => fn !== undefined], // just check no crash
  ["(car '(1 2 3))", "1"],
  ["(cdr '(1 2 3))", "(2 3)"],
  ["(begin (define a 5) (define b 3) (+ a b))", "8"],
  ["garbage)))", s => s.startsWith("PARSE_ERROR")],
];

console.log("Testing lisp-rlm-wasm:\n");
let passed = 0;
for (const [code, expected] of tests) {
  const result = evalLisp(code);
  const ok = typeof expected === 'function'
    ? expected(result)
    : result === expected;
  console.log(`${ok ? '✅' : '❌'} (${code}) → ${result}${ok ? '' : ` (expected: ${expected})`}`);
  if (ok) passed++;
}
console.log(`\n${passed}/${tests.length} passed`);
