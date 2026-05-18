/// <reference lib="es2023" />
/// <reference lib="webworker" />

/**
 * HTTP Worker for synchronous-ish WASM imports.
 * 
 * WASM imports must be synchronous, but fetch() is async.
 * Solution: Worker thread + Atomics.wait()/notify() to block WASM execution
 * until the async fetch completes.
 * 
 * Requires SharedArrayBuffer, which needs these headers:
 *   Cross-Origin-Opener-Policy: same-origin
 *   Cross-Origin-Embedder-Policy: require-corp
 */

let pendingResponse: string | null = null;
let sharedBuffer: Int32Array | null = null;
let memory: WebAssembly.Memory | null = null;
let respBufPtr = 0;
let respBufLen = 0;
let respLenPtr = 0;

self.onmessage = async (e: MessageEvent) => {
  const { type, data } = e.data;

  switch (type) {
    case 'init': {
      // Store shared memory reference
      sharedBuffer = new Int32Array(data.sharedBuffer);
      memory = new WebAssembly.Memory(data.memoryDesc);
      break;
    }

    case 'http_request': {
      // Fetch the URL
      const { url, respBufPtr: ptr, respBufLen: len, respLenPtr: lp } = data;
      
      try {
        const response = await fetch(url, {
          headers: { 'User-Agent': 'lisp-rlm-browser' }
        });
        const text = await response.text();
        pendingResponse = text;
        
        // Write response to WASM memory
        if (memory && sharedBuffer) {
          const bytes = new TextEncoder().encode(text);
          const mem = new Uint8Array(memory.buffer);
          const toCopy = Math.min(bytes.length, len);
          mem.set(bytes.slice(0, toCopy), ptr);
          new DataView(memory.buffer).setUint32(lp, toCopy, true);
        }
        
        // Notify WASM thread that response is ready
        Atomics.store(sharedBuffer!, 0, 1);
        Atomics.notify(sharedBuffer!, 0);
      } catch (err) {
        // Return error as JSON
        const errorJson = JSON.stringify({ error: true, message: String(err) });
        if (memory && sharedBuffer) {
          const bytes = new TextEncoder().encode(errorJson);
          const mem = new Uint8Array(memory.buffer);
          const toCopy = Math.min(bytes.length, len);
          mem.set(bytes.slice(0, toCopy), ptr);
          new DataView(memory.buffer).setUint32(lp, toCopy, true);
        }
        Atomics.store(sharedBuffer!, 0, 1);
        Atomics.notify(sharedBuffer!, 0);
      }
      break;
    }

    case 'set_memory': {
      // Update memory reference after WASM instantiation
      memory = data.memory;
      break;
    }
  }
};

// Export for type checking
export type HttpWorkerMessage = 
  | { type: 'init'; data: { sharedBuffer: SharedArrayBuffer; memoryDesc: WebAssembly.MemoryDescriptor } }
  | { type: 'http_request'; data: { url: string; respBufPtr: number; respBufLen: number; respLenPtr: number } }
  | { type: 'set_memory'; data: { memory: WebAssembly.Memory } };