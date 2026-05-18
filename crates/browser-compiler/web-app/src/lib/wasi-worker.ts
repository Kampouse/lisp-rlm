/// WASI Worker - runs WASM in Worker thread with real HTTP via Atomics.wait()
/// Requires SharedArrayBuffer (COOP/COEP headers)

let wasmInstance: WebAssembly.Instance | null = null;
let wasmMemory: WebAssembly.Memory | null = null;
let capturedOutput = '';

// Shared buffer for Atomics communication (first 4 bytes: signal, next 4: length, rest: data)
let sharedBuffer: Int32Array | null = null;

// Read null-terminated UTF-8 string from WASM memory
function readString(memory: WebAssembly.Memory, ptr: number, len: number): string {
  const buf = new Uint8Array(memory.buffer);
  const bytes = buf.slice(ptr, ptr + len);
  return new TextDecoder().decode(bytes);
}

// Copy string to WASM memory
function writeString(memory: WebAssembly.Memory, ptr: number, str: string): number {
  const encoder = new TextEncoder();
  const bytes = encoder.encode(str);
  const buf = new Uint8Array(memory.buffer);
  for (let i = 0; i < bytes.length && i < buf.length - ptr; i++) {
    buf[ptr + i] = bytes[i];
  }
  return bytes.length;
}

// WASI Preview 1 polyfills
const wasiPolyfills = {
  fd_write: (fd: number, iovs: number, iovsLen: number, nwritten: number): number => {
    if (fd !== 1 && fd !== 2) return 8; // EBADF
    const mem = new Uint8Array(wasmMemory!.buffer);
    const view = new DataView(wasmMemory!.buffer);
    let written = 0;
    for (let i = 0; i < iovsLen; i++) {
      const iovPtr = iovs + i * 8;
      const bufPtr = view.getUint32(iovPtr, true);
      const bufLen = view.getUint32(iovPtr + 4, true);
      const chunk = new TextDecoder().decode(mem.slice(bufPtr, bufPtr + bufLen));
      capturedOutput += chunk;
      written += bufLen;
    }
    new DataView(wasmMemory!.buffer).setUint32(nwritten, written, true);
    return 0;
  },

  fd_read: (fd: number, iovs: number, iovsLen: number, nread: number): number => {
    return 8; // EBADF (no stdin)
  },

  fd_seek: (fd: number, offset: bigint, whence: number, newoffset: number): number => {
    return 8; // EBADF
  },

  fd_close: (fd: number): number => {
    return 0;
  },

  random_get: (buf: number, bufLen: number): number => {
    const mem = new Uint8Array(wasmMemory!.buffer);
    for (let i = 0; i < bufLen; i++) {
      mem[buf + i] = Math.floor(Math.random() * 256);
    }
    return 0;
  },

  proc_exit: (code: number): void => {
    // Signal completion
    self.postMessage({ type: 'done', stdout: capturedOutput, exitCode: code });
  },

  sched_yield: (): number => 0,

  // Environment variables (none in browser)
  environ_sizes_get: (environCount: number, environBufSize: number): number => {
    const view = new DataView(wasmMemory!.buffer);
    view.setUint32(environCount, 0, true);
    view.setUint32(environBufSize, 0, true);
    return 0;
  },

  environ_get: (environ: number, environBuf: number): number => {
    return 0;
  },

  // Command-line arguments (none in browser)
  args_sizes_get: (argc: number, argvBufSize: number): number => {
    const view = new DataView(wasmMemory!.buffer);
    view.setUint32(argc, 0, true);
    view.setUint32(argvBufSize, 0, true);
    return 0;
  },

  args_get: (argv: number, argvBuf: number): number => {
    return 0;
  },

  // Clock/time
  clock_time_get: (clockId: number, precision: bigint, time: number): number => {
    const view = new DataView(wasmMemory!.buffer);
    // Return current time in nanoseconds
    const now = BigInt(Date.now()) * 1_000_000n;
    view.setBigUint64(time, now, true);
    return 0;
  },

  clock_res_get: (clockId: number, resolution: number): number => {
    const view = new DataView(wasmMemory!.buffer);
    view.setBigUint64(resolution, 1_000_000n, true); // 1ms resolution
    return 0;
  },
};

// OutLayer polyfills with real HTTP
const outlayerPolyfills = {
  // View functions (return empty string/0 in browser)
  view: (keyPtr: number, keyLen: number, valBufPtr: number, valBufLen: number, valLenPtr: number): number => {
    new DataView(wasmMemory!.buffer).setUint32(valLenPtr, 0, true);
    return 0;
  },

  call: (contractPtr: number, contractLen: number, methodPtr: number, methodLen: number, argsPtr: number, argsLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): number => {
    // Cannot make contract calls in browser
    new DataView(wasmMemory!.buffer).setUint32(respLenPtr, 0, true);
    return 1; // Error
  },

  transfer: (recipientPtr: number, recipientLen: number, amountPtr: number, amountLen: number): number => {
    // Cannot transfer in browser
    return 1; // Error
  },

  http_get: (urlPtr: number, urlLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): number => {
    const url = readString(wasmMemory!, urlPtr, urlLen);
    
    // Signal main thread we need HTTP
    self.postMessage({ type: 'http_request', url });
    
    // Block until response arrives (synchronous!)
    Atomics.wait(sharedBuffer!, 0, 0);
    
    // Read length from shared buffer (index 1)
    const responseLen = Atomics.load(sharedBuffer!, 1);
    
    // Read response data from shared buffer (skip first 8 bytes)
    const sharedBytes = new Uint8Array(sharedBuffer!.buffer);
    const responseData = sharedBytes.slice(8, 8 + responseLen);
    
    // Write result to WASM memory
    const toCopy = Math.min(responseData.length, respBufLen);
    const memBytes = new Uint8Array(wasmMemory!.buffer);
    memBytes.set(responseData.slice(0, toCopy), respBufPtr);
    
    const view = new DataView(wasmMemory!.buffer);
    view.setUint32(respLenPtr, toCopy, true);
    
    // Reset signal for next call
    Atomics.store(sharedBuffer!, 0, 0);
    
    return 0; // Success
  },

  storage_get: (keyPtr: number, keyLen: number, valBufPtr: number, valBufLen: number, valLenPtr: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const value = localStorage.getItem(key) || '';
    const written = writeString(wasmMemory!, valBufPtr, value);
    new DataView(wasmMemory!.buffer).setUint32(valLenPtr, written, true);
    return 0;
  },

  storage_set: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const value = readString(wasmMemory!, valPtr, valLen);
    localStorage.setItem(key, value);
    return 0;
  },

  storage_has: (keyPtr: number, keyLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    return localStorage.getItem(key) !== null ? 1 : 0;
  },

  storage_delete: (keyPtr: number, keyLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    localStorage.removeItem(key);
    return 0;
  },

  storage_increment: (keyPtr: number, keyLen: number, amountPtr: number, amountLen: number, resultBufPtr: number, resultBufLen: number, resultLenPtr: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const amountStr = readString(wasmMemory!, amountPtr, amountLen);
    const amount = BigInt(amountStr);
    const current = BigInt(localStorage.getItem(key) || '0');
    const newValue = current + amount;
    localStorage.setItem(key, newValue.toString());
    const written = writeString(wasmMemory!, resultBufPtr, newValue.toString());
    new DataView(wasmMemory!.buffer).setUint32(resultLenPtr, written, true);
    return 0;
  },

  storage_decrement: (keyPtr: number, keyLen: number, amountPtr: number, amountLen: number, resultBufPtr: number, resultBufLen: number, resultLenPtr: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const amountStr = readString(wasmMemory!, amountPtr, amountLen);
    const amount = BigInt(amountStr);
    const current = BigInt(localStorage.getItem(key) || '0');
    const newValue = current - amount;
    localStorage.setItem(key, newValue.toString());
    const written = writeString(wasmMemory!, resultBufPtr, newValue.toString());
    new DataView(wasmMemory!.buffer).setUint32(resultLenPtr, written, true);
    return 0;
  },

  storage_set_if_absent: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    if (localStorage.getItem(key) === null) {
      const value = readString(wasmMemory!, valPtr, valLen);
      localStorage.setItem(key, value);
      return 1; // Set
    }
    return 0; // Not set
  },

  storage_set_if_equals: (keyPtr: number, keyLen: number, expectedPtr: number, expectedLen: number, newValPtr: number, newValLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const expected = readString(wasmMemory!, expectedPtr, expectedLen);
    const current = localStorage.getItem(key) || '';
    if (current === expected) {
      const newVal = readString(wasmMemory!, newValPtr, newValLen);
      localStorage.setItem(key, newVal);
      return 1; // Set
    }
    return 0; // Not set
  },

  storage_list_keys: (prefixPtr: number, prefixLen: number, keysBufPtr: number, keysBufLen: number, keysLenPtr: number): number => {
    const prefix = readString(wasmMemory!, prefixPtr, prefixLen);
    const keys: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key && key.startsWith(prefix)) {
        keys.push(key);
      }
    }
    const result = keys.join('\n');
    const written = writeString(wasmMemory!, keysBufPtr, result);
    new DataView(wasmMemory!.buffer).setUint32(keysLenPtr, written, true);
    return 0;
  },

  storage_clear_all: (): number => {
    localStorage.clear();
    return 0;
  },

  storage_set_worker: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    return outlayerPolyfills.storage_set(keyPtr, keyLen, valPtr, valLen);
  },

  storage_get_worker: (keyPtr: number, keyLen: number, valBufPtr: number, valBufLen: number, valLenPtr: number): number => {
    return outlayerPolyfills.storage_get(keyPtr, keyLen, valBufPtr, valBufLen, valLenPtr);
  },

  storage_set_worker_public: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    return outlayerPolyfills.storage_set(keyPtr, keyLen, valPtr, valLen);
  },

  storage_get_worker_from_project: (projectPtr: number, projectLen: number, workerPtr: number, workerLen: number, valBufPtr: number, valBufLen: number, valLenPtr: number): number => {
    new DataView(wasmMemory!.buffer).setUint32(valLenPtr, 0, true);
    return 0;
  },

  env_signer: (bufPtr: number, bufLen: number, lenPtr: number): number => {
    // No signer in browser
    new DataView(wasmMemory!.buffer).setUint32(lenPtr, 0, true);
    return 0;
  },

  env_predecessor: (bufPtr: number, bufLen: number, lenPtr: number): number => {
    // No predecessor in browser
    new DataView(wasmMemory!.buffer).setUint32(lenPtr, 0, true);
    return 0;
  },
};

const imports = {
  wasi_snapshot_preview1: wasiPolyfills,
  outlayer: outlayerPolyfills,
};

// Handle messages from main thread
self.onmessage = async (e: MessageEvent) => {
  const { type, bytes, sharedBuffer: sb } = e.data;
  
  if (type === 'init') {
    sharedBuffer = new Int32Array(sb);
    
    try {
      const { instance, module } = await WebAssembly.instantiate(bytes, imports);
      wasmInstance = instance;
      wasmMemory = instance.exports.memory as WebAssembly.Memory;
      
      // Run WASM in next tick so message loop can process postMessage from imports
      setTimeout(() => {
        try {
          const exports = instance.exports as Record<string, unknown>;
          if (typeof exports._start === 'function') {
            exports._start();
            // proc_exit will send 'done', but if it doesn't get called:
            setTimeout(() => {
              self.postMessage({ type: 'done', stdout: capturedOutput });
            }, 100);
          }
        } catch (err) {
          self.postMessage({ type: 'error', error: String(err) });
        }
      }, 0);
      
    } catch (err) {
      self.postMessage({ type: 'error', error: String(err) });
    }
    return;
  }
};

export {};