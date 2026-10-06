/// <reference lib="webworker" />
/**
 * Compile worker: owns the compiler WASM so a bad contract (wasm32
 * stack overflow, huge alloc) crashes THIS worker, not the tab.
 *
 * Protocol (postMessage):
 *   -> { type: 'compile', id, source, target, lang }
 *   <- { type: 'ok', id, wasmBytes, size, timeMs, wat, exports }
 *   <- { type: 'err', id, error }
 */
import init, {
  compile_p1, compile_p2_core, compile_pure, compile_ts, disassemble_wasm,
} from '../../public/wasm/lisp_rlm_browser.js';

let ready = false;

async function ensureReady(): Promise<void> {
  if (ready) return;
  await init();
  ready = true;
}

self.onmessage = async (e: MessageEvent) => {
  const { type, id, source, target, lang } = e.data ?? {};
  if (type !== 'compile') return;

  const start = performance.now();
  try {
    await ensureReady();
    let wasmBytes: Uint8Array;
    switch (target) {
      case 'p1':
        wasmBytes = lang === 'ts' ? compile_ts(source) : compile_p1(source);
        break;
      case 'p2':
        wasmBytes = compile_p2_core(source);
        break;
      case 'pure':
        wasmBytes = compile_pure(source);
        break;
      default:
        wasmBytes = lang === 'ts' ? compile_ts(source) : compile_p1(source);
    }
    const timeMs = performance.now() - start;
    let wat: string | null = null;
    try {
      wat = disassemble_wasm(wasmBytes);
    } catch {
      wat = null;
    }
    self.postMessage({ type: 'ok', id, wasmBytes, size: wasmBytes.length, timeMs, wat });
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    self.postMessage({ type: 'err', id, error: message, timeMs: performance.now() - start });
  }
};
