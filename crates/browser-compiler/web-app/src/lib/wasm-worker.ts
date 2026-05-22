/**
 * WASM Worker — runs WASI code with synchronous HTTP via Atomics.wait().
 * 
 * Pattern:
 *   Main Thread — UI + async fetch
 *   Worker — WASM execution + Atomics.wait() blocking
 * 
 * Communication: SharedArrayBuffer + postMessage
 */

let sharedBuffer: Int32Array | null = null;
let sharedData: Uint8Array | null = null;
let memory: WebAssembly.Memory | null = null;

// WASI state
let stdout = '';
let stdin = new Uint8Array(0);
let stdinOffset = 0;

self.onmessage = async (e: MessageEvent) => {
  const { type, data } = e.data;

  switch (type) {
    case 'init': {
      // Shared memory for coordination
      sharedBuffer = new Int32Array(data.sharedBuffer);
      sharedData = new Uint8Array(data.sharedBuffer);
      break;
    }

    case 'run': {
      const { wasmBytes, stdinData } = data;
      stdin = stdinData ?? new Uint8Array(0);
      stdinOffset = 0;
      stdout = '';

      try {
        const result = await runWasm(wasmBytes);
        self.postMessage({ type: 'done', stdout: result });
      } catch (err) {
        self.postMessage({ type: 'error', error: String(err) });
      }
      break;
    }

    case 'http_response': {
      // Main thread completed fetch — write to shared buffer and wake us
      const { response } = data;
      if (sharedData && response) {
        const bytes = new TextEncoder().encode(response);
        sharedData.set(bytes.slice(0, sharedData.length - 4), 4); // Leave room for length
        new DataView(sharedData.buffer).setUint32(0, Math.min(bytes.length, sharedData.length - 4), true);
      }
      Atomics.store(sharedBuffer!, 0, 1);
      Atomics.notify(sharedBuffer!, 0);
      break;
    }
  }
};

async function runWasm(wasmBytes: Uint8Array): Promise<string> {
  // Create memory (shared with main thread for HTTP responses)
  memory = new WebAssembly.Memory({ initial: 256, maximum: 4096, shared: true });
  
  // WASI imports
  const imports: WebAssembly.Imports = {
    wasi_snapshot_preview1: {
      fd_read: (fd: number, iovsPtr: number, iovsLen: number, nreadPtr: number): number => {
        if (fd !== 0) return 8;
        const bytes = new Uint8Array(memory!.buffer);
        const view = new DataView(memory!.buffer);
        let total = 0;
        for (let i = 0; i < iovsLen && stdinOffset < stdin.length; i++) {
          const iovBufPtr = view.getUint32(iovsPtr + i * 8, true);
          const iovBufLen = view.getUint32(iovsPtr + i * 4 + 4, true);
          const toCopy = Math.min(iovBufLen, stdin.length - stdinOffset);
          bytes.set(stdin.slice(stdinOffset, stdinOffset + toCopy), iovBufPtr);
          stdinOffset += toCopy;
          total += toCopy;
        }
        view.setUint32(nreadPtr, total, true);
        return 0;
      },
      fd_write: (fd: number, iovsPtr: number, iovsLen: number, nwrittenPtr: number): number => {
        if (fd !== 1 && fd !== 2) return 8;
        const bytes = new Uint8Array(memory!.buffer);
        const view = new DataView(memory!.buffer);
        for (let i = 0; i < iovsLen; i++) {
          const iovBufPtr = view.getUint32(iovsPtr + i * 8, true);
          const iovBufLen = view.getUint32(iovsPtr + i * 4 + 4, true);
          const chunk = bytes.slice(iovBufPtr, iovBufPtr + iovBufLen);
          stdout += new TextDecoder().decode(chunk);
        }
        return 0;
      },
      proc_exit: (code: number): void => {
        throw new Error(`WASI exit(${code})`);
      },
      random_get: (bufPtr: number, bufLen: number): number => {
        crypto.getRandomValues(new Uint8Array(memory!.buffer).slice(bufPtr, bufPtr + bufLen));
        return 0;
      },
      environ_sizes_get: (): number => 0,
      environ_get: (): number => 0,
      fd_seek: (): number => 0,
    },
    outlayer: {
      // SYNCHRONOUS HTTP! Uses Atomics.wait() to block until main thread fetches
      http_get: (urlPtr: number, urlLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): number => {
        const bytes = new Uint8Array(memory!.buffer);
        const url = new TextDecoder().decode(bytes.slice(urlPtr, urlPtr + urlLen));
        
        // Request main thread to fetch
        self.postMessage({ type: 'http_request', url, respBufPtr, respBufLen, respLenPtr });
        
        // Block until response arrives
        Atomics.wait(sharedBuffer!, 0, 0);
        
        // Read response from shared buffer
        const view = new DataView(sharedData!.buffer);
        const responseLen = view.getUint32(0, true);
        const responseData = sharedData!.slice(4, 4 + responseLen);
        
        // Copy to WASM memory
        bytes.set(responseData.slice(0, respBufLen), respBufPtr);
        new DataView(memory!.buffer).setUint32(respLenPtr, Math.min(responseLen, respBufLen), true);
        
        return 0;
      },
      storage_set: (k: number, kl: number, v: number, vl: number): number => {
        const bytes = new Uint8Array(memory!.buffer);
        const key = new TextDecoder().decode(bytes.slice(k, k + kl));
        const val = new TextDecoder().decode(bytes.slice(v, v + vl));
        self.postMessage({ type: 'storage_set', key, val });
        return 0;
      },
      storage_get: (k: number, kl: number, buf: number, bl: number, lp: number): number => {
        const bytes = new Uint8Array(memory!.buffer);
        const key = new TextDecoder().decode(bytes.slice(k, k + kl));
        // Request from main thread (localStorage is main-thread only)
        self.postMessage({ type: 'storage_get', key, buf, bl, lp });
        Atomics.wait(sharedBuffer!, 0, 0);
        return 0;
      },
      storage_has: (k: number, kl: number): number => {
        const key = new TextDecoder().decode(new Uint8Array(memory!.buffer).slice(k, k + kl));
        self.postMessage({ type: 'storage_has', key });
        Atomics.wait(sharedBuffer!, 0, 0);
        return sharedBuffer![1]; // Result stored at index 1
      },
      storage_delete: (k: number, kl: number): number => {
        const key = new TextDecoder().decode(new Uint8Array(memory!.buffer).slice(k, k + kl));
        self.postMessage({ type: 'storage_delete', key });
        return 0;
      },
      storage_clear_all: (): number => {
        self.postMessage({ type: 'storage_clear' });
        return 0;
      },
      // Stubs for remaining outlayer functions
      storage_increment: () => 0,
      storage_decrement: () => 0,
      storage_set_if_absent: () => 0,
      storage_set_if_equals: () => 0,
      storage_list_keys: () => 0,
      storage_set_worker: () => 0,
      storage_get_worker: () => 0,
      storage_set_worker_public: () => 0,
      storage_get_worker_from_project: () => 0,
      view: () => 0,
      call: () => 0,
      transfer: () => 0,
      env_signer: (buf: number, len: number, lp: number): number => {
        const s = 'browser-user';
        new Uint8Array(memory!.buffer).set(new TextEncoder().encode(s).slice(0, len), buf);
        new DataView(memory!.buffer).setUint32(lp, Math.min(s.length, len), true);
        return 0;
      },
      env_predecessor: (buf: number, len: number, lp: number): number => {
        const s = 'browser-predecessor';
        new Uint8Array(memory!.buffer).set(new TextEncoder().encode(s).slice(0, len), buf);
        new DataView(memory!.buffer).setUint32(lp, Math.min(s.length, len), true);
        return 0;
      },
    },
    env: buildEnvStubs(),
  };

  const { instance } = await WebAssembly.instantiate(wasmBytes, imports);
  const exports = instance.exports as Record<string, WebAssembly.ExportValue>;

  try {
    const startFn = exports._start ?? exports.run;
    if (typeof startFn === 'function') {
      (startFn as Function)();
    }
  } catch (e) {
    if (!(e instanceof Error && e.message.includes('exit'))) {
      throw e;
    }
  }

  return stdout || '(no output)';
}

function buildEnvStubs(): Record<string, Function> {
  const stub = () => {};
  const stubI64 = () => BigInt(0);
  return {
    read_register: stub, register_len: stubI64, write_register: stub,
    current_account_id: stub, signer_account_id: stub, signer_account_pk: stub,
    predecessor_account_id: stub, input: stub,
    block_index: stubI64, block_timestamp: stubI64, epoch_height: stubI64,
    storage_usage: stubI64,
    account_balance: stub, account_locked_balance: stub, attached_deposit: stub,
    prepaid_gas: stubI64, used_gas: stubI64,
    storage_write: stubI64, storage_read: stubI64, storage_remove: stubI64,
    storage_has_key: stubI64,
    sha256: stub, keccak256: stub, random_seed: stub, ed25519_verify: stubI64, p256_verify: stubI64,
    value_return: stub, panic: stub, panic_utf8: stub, log_utf8: stub, log_utf16: stub,
    promise_create: stubI64, promise_then: stubI64, promise_and: stubI64,
    promise_results_count: stubI64, promise_result: stub, promise_return: stub,
    storage_iter_prefix: stubI64, storage_iter_range: stubI64, storage_iter_next: stubI64,
    promise_batch_create: stubI64, promise_batch_then: stubI64,
  };
}

export type WasmWorkerMessage =
  | { type: 'init'; data: { sharedBuffer: SharedArrayBuffer } }
  | { type: 'run'; data: { wasmBytes: Uint8Array; stdinData?: Uint8Array } }
  | { type: 'http_response'; data: { response: string } }
  | { type: 'storage_get_response'; data: { value: string } }
  | { type: 'storage_has_response'; data: { exists: boolean } };