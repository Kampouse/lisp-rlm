/**
 * Run WASI WASM in Worker thread with real HTTP via Atomics.wait()
 * 
 * Requires SharedArrayBuffer, so Vite must serve:
 *   Cross-Origin-Opener-Policy: same-origin
 *   Cross-Origin-Embedder-Policy: require-corp
 */

import { compileP2Core, type CompileResult } from './compiler';

// TypeScript type for our Worker
interface WasiWorker extends Worker {
  postMessage(message: any): void;
}

export async function runWasiWithWorker(bytes: Uint8Array): Promise<string> {
  return new Promise((resolve, reject) => {
    // Shared buffer for Atomics communication
    // First 4 bytes: signal (0=wait, 1=ready)
    // Next 4 bytes: response length
    // Remaining: HTTP response data (up to 64KB)
    const sharedBuffer = new SharedArrayBuffer(65536);
    const sharedView = new Int32Array(sharedBuffer);
    const sharedBytes = new Uint8Array(sharedBuffer);
    Atomics.store(sharedView, 0, 0);

    // Spawn Worker
    const worker = new Worker(
      new URL('./wasi-worker.ts', import.meta.url),
      { type: 'module' }
    );

    let stdout = '';

    worker.onmessage = async (e: MessageEvent) => {
      const { type, url, error, stdout: out, exitCode } = e.data;

      if (type === 'http_request') {
        // Main thread does async HTTP
        try {
          const resp = await fetch(url, { headers: { 'User-Agent': 'lisp-rlm-browser' } });
          const text = await resp.text();
          
          // Write response to shared memory (skip first 8 bytes for signal + length)
          const responseBytes = new TextEncoder().encode(text);
          const maxLen = sharedBytes.length - 8;
          const toCopy = Math.min(responseBytes.length, maxLen);
          sharedBytes.set(responseBytes.slice(0, toCopy), 8);
          
          // Set response length
          Atomics.store(sharedView, 1, toCopy);
          
          // Wake worker
          Atomics.store(sharedView, 0, 1);
          Atomics.notify(sharedView, 0);
        } catch (err) {
          // Write error as JSON
          const errorJson = JSON.stringify({ error: true, message: String(err), url });
          const errorBytes = new TextEncoder().encode(errorJson);
          sharedBytes.set(errorBytes.slice(0, Math.min(errorBytes.length, sharedBytes.length - 8)), 8);
          Atomics.store(sharedView, 1, Math.min(errorBytes.length, sharedBytes.length - 8));
          Atomics.store(sharedView, 0, 1);
          Atomics.notify(sharedView, 0);
        }
        return;
      }

      if (type === 'done') {
        stdout = out ?? '';
        worker.terminate();
        resolve(stdout);
        return;
      }

      if (type === 'error') {
        worker.terminate();
        reject(new Error(error));
        return;
      }
    };

    worker.onerror = (err) => {
      worker.terminate();
      reject(err);
    };

    // Initialize Worker with WASM
    worker.postMessage({
      type: 'init',
      bytes,
      sharedBuffer,
    });
  });
}