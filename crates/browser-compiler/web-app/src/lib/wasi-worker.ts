/// WASI Worker - runs WASM in Worker thread with real HTTP via Atomics.wait()
/// Requires SharedArrayBuffer (COOP/COEP headers)

let wasmInstance: WebAssembly.Instance | null = null;
let wasmMemory: WebAssembly.Memory | null = null;
let capturedOutput = '';

// In-memory storage (localStorage not available in Worker)
const memoryStorage: Map<string, string> = new Map();

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
    const value = memoryStorage.get(key) || '';
    const written = writeString(wasmMemory!, valBufPtr, value);
    new DataView(wasmMemory!.buffer).setUint32(valLenPtr, written, true);
    return 0;
  },

  storage_set: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const value = readString(wasmMemory!, valPtr, valLen);
    memoryStorage.set(key, value);
    return 0;
  },

  storage_has: (keyPtr: number, keyLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    return memoryStorage.has(key) ? 1 : 0;
  },

  storage_delete: (keyPtr: number, keyLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    memoryStorage.delete(key);
    return 0;
  },

  storage_increment: (keyPtr: number, keyLen: number, amountPtr: number, amountLen: number, resultBufPtr: number, resultBufLen: number, resultLenPtr: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const amountStr = readString(wasmMemory!, amountPtr, amountLen);
    const amount = BigInt(amountStr);
    const current = BigInt(memoryStorage.get(key) || '0');
    const newValue = current + amount;
    memoryStorage.set(key, newValue.toString());
    const written = writeString(wasmMemory!, resultBufPtr, newValue.toString());
    new DataView(wasmMemory!.buffer).setUint32(resultLenPtr, written, true);
    return 0;
  },

  storage_decrement: (keyPtr: number, keyLen: number, amountPtr: number, amountLen: number, resultBufPtr: number, resultBufLen: number, resultLenPtr: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const amountStr = readString(wasmMemory!, amountPtr, amountLen);
    const amount = BigInt(amountStr);
    const current = BigInt(memoryStorage.get(key) || '0');
    const newValue = current - amount;
    memoryStorage.set(key, newValue.toString());
    const written = writeString(wasmMemory!, resultBufPtr, newValue.toString());
    new DataView(wasmMemory!.buffer).setUint32(resultLenPtr, written, true);
    return 0;
  },

  storage_set_if_absent: (keyPtr: number, keyLen: number, valPtr: number, valLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    if (memoryStorage.get(key) === null) {
      const value = readString(wasmMemory!, valPtr, valLen);
      memoryStorage.set(key, value);
      return 1; // Set
    }
    return 0; // Not set
  },

  storage_set_if_equals: (keyPtr: number, keyLen: number, expectedPtr: number, expectedLen: number, newValPtr: number, newValLen: number): number => {
    const key = readString(wasmMemory!, keyPtr, keyLen);
    const expected = readString(wasmMemory!, expectedPtr, expectedLen);
    const current = memoryStorage.get(key) || '';
    if (current === expected) {
      const newVal = readString(wasmMemory!, newValPtr, newValLen);
      memoryStorage.set(key, newVal);
      return 1; // Set
    }
    return 0; // Not set
  },

  storage_list_keys: (prefixPtr: number, prefixLen: number, keysBufPtr: number, keysBufLen: number, keysLenPtr: number): number => {
    const prefix = readString(wasmMemory!, prefixPtr, prefixLen);
    const keys: string[] = [];
    for (let i = 0; i < memoryStorage.size; i++) {
      const key = Array.from(memoryStorage.keys())[i];
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
    memoryStorage.clear();
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

// wasi:http polyfill — implements resource lifecycle + fetch() for browser
// Resources are tracked as numeric handles; actual HTTP happens in `handle`
const wasiHttpResources: Map<number, any> = new Map();
let nextResourceId = 1;

function allocResource(val: any): number {
  const id = nextResourceId++;
  wasiHttpResources.set(id, val);
  return id;
}

// Data segments baked into WASM at compile time — we read them from memory
const BUFSIZE = 163840; // SENTINEL_BUF
const RETAREA = 131072; // used by wasi:http shim

// Read i32 string pair (ptr, len) from WASM memory
function readWasmString(mem: WebAssembly.Memory, ptr: number, len: number): string {
  if (len === 0) return '';
  return new TextDecoder().decode(new Uint8Array(mem.buffer).slice(ptr, ptr + len));
}

// Write bytes to WASM memory, return bytes written
function writeWasmBytes(mem: WebAssembly.Memory, ptr: number, data: Uint8Array): number {
  const buf = new Uint8Array(mem.buffer);
  const n = Math.min(data.length, buf.length - ptr);
  buf.set(data.slice(0, n), ptr);
  return n;
}

const wasiHttpTypes: Record<string, (...args: number[]) => number> = {
  // Resource drops — just delete the handle
  '[resource-drop]input-stream': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]output-stream': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]incoming-response': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]future-incoming-response': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]outgoing-body': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]outgoing-request': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]incoming-body': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]fields': (rid: number) => { wasiHttpResources.delete(rid); return 0; },
  '[resource-drop]pollable': (rid: number) => { wasiHttpResources.delete(rid); return 0; },

  // Constructors — return a resource handle
  '[constructor]fields': () => allocResource({ type: 'fields', entries: [] }),
  '[constructor]outgoing-request': (headersRid: number) => {
    const h = wasiHttpResources.get(headersRid);
    return allocResource({ type: 'outgoing-request', headers: h, method: 'GET', scheme: 'HTTPS', authority: '', path: '' });
  },

  // Methods on outgoing-request
  '[method]outgoing-request.set-method': (reqRid: number, methodPtr: number, methodLen: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (req) req.method = readWasmString(wasmMemory!, methodPtr, methodLen);
    return 0;
  },
  '[method]outgoing-request.set-scheme': (reqRid: number, schemePtr: number, schemeLen: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (req) req.scheme = readWasmString(wasmMemory!, schemePtr, schemeLen);
    return 0;
  },
  '[method]outgoing-request.set-authority': (reqRid: number, authPtr: number, authLen: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (req) req.authority = readWasmString(wasmMemory!, authPtr, authLen);
    return 0;
  },
  '[method]outgoing-request.set-path-with-query': (reqRid: number, pathPtr: number, pathLen: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (req) req.path = readWasmString(wasmMemory!, pathPtr, pathLen);
    return 0;
  },
  '[method]outgoing-request.body': (reqRid: number, outBodyRid: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (req) {
      const bodyRid = allocResource({ type: 'outgoing-body', data: [] });
      new DataView(wasmMemory!.buffer).setUint32(outBodyRid, bodyRid, true);
    }
    return 0;
  },

  // Methods on outgoing-body
  '[method]outgoing-body.write': (bodyRid: number, outStreamRid: number) => {
    const body = wasiHttpResources.get(bodyRid);
    if (body) {
      const streamRid = allocResource({ type: 'output-stream' });
      new DataView(wasmMemory!.buffer).setUint32(outStreamRid, streamRid, true);
    }
    return 0;
  },
  '[static]outgoing-body.finish': (bodyRid: number, trailersPtr: number) => {
    // trailersPtr is an option — 0 means none
    wasiHttpResources.delete(bodyRid);
    return 0;
  },

  // Methods on fields
  '[method]fields.set': (fieldsRid: number, keyPtr: number, keyLen: number, valPtr: number, valLen: number) => {
    const f = wasiHttpResources.get(fieldsRid);
    if (f) f.entries.push({ key: readWasmString(wasmMemory!, keyPtr, keyLen), value: readWasmString(wasmMemory!, valPtr, valLen) });
    return 0;
  },

  // Methods on future-incoming-response
  '[method]future-incoming-response.subscribe': (firRid: number) => {
    const fir = wasiHttpResources.get(firRid);
    if (fir) {
      // Create a "ready" pollable
      fir.pollable = allocResource({ type: 'pollable' });
    }
    return 0;
  },
  '[method]future-incoming-response.get': (firRid: number) => {
    const fir = wasiHttpResources.get(firRid);
    if (fir && fir.response) {
      return 0; // ok — result is written to memory
    }
    return 1; // error
  },

  // Methods on incoming-response
  '[method]incoming-response.consume': (respRid: number, outBodyRid: number) => {
    const resp = wasiHttpResources.get(respRid);
    if (resp) {
      const bodyRid = allocResource({ type: 'incoming-body', data: resp.bodyData || new Uint8Array() });
      new DataView(wasmMemory!.buffer).setUint32(outBodyRid, bodyRid, true);
    }
    return 0;
  },

  // Methods on incoming-body
  '[method]incoming-body.stream': (bodyRid: number, outStreamRid: number) => {
    const body = wasiHttpResources.get(bodyRid);
    if (body) {
      const streamRid = allocResource({ type: 'input-stream', data: body.data });
      new DataView(wasmMemory!.buffer).setUint32(outStreamRid, streamRid, true);
    }
    return 0;
  },

  // Methods on input-stream (blocking-read into WASM memory)
  '[method]input-stream.blocking-read': (streamRid: number, bufPtr: number, bufLen: number, outLenPtr: number) => {
    const stream = wasiHttpResources.get(streamRid);
    if (stream && stream.data) {
      const n = Math.min(stream.data.length, bufLen);
      writeWasmBytes(wasmMemory!, bufPtr, stream.data.slice(0, n));
      new DataView(wasmMemory!.buffer).setUint32(outLenPtr, n, true);
      stream.data = stream.data.slice(n); // consume
      return 0; // ok
    }
    return 1; // error / end of stream
  },

  // Methods on output-stream (blocking-write-and-flush)
  '[method]output-stream.blocking-write-and-flush': (streamRid: number, dataPtr: number, dataLen: number) => {
    // The data to write is the HTTP body content that was copied to BUFSIZE
    // We read it from the known buffer location (BUFSIZE)
    const bodyData = new Uint8Array(wasmMemory!.buffer).slice(BUFSIZE, BUFSIZE + dataLen);
    // Attach to the outgoing-body resource (we need to find it)
    // The shim copies body to 131088 (RETAREA+16), then writes to stream
    // We track it via the body resource
    for (const [rid, res] of wasiHttpResources) {
      if (res.type === 'outgoing-body') {
        res.data = bodyData;
        break;
      }
    }
    return 0;
  },

  // get-stdout / get-stdin return resource handles
};

const wasiHttpOutgoing: Record<string, (...args: number[]) => number> = {
  // This is where the actual HTTP request happens!
  'handle': (reqRid: number, outPtr: number) => {
    const req = wasiHttpResources.get(reqRid);
    if (!req) {
      // Write error result
      new DataView(wasmMemory!.buffer).setUint32(outPtr, 1, true); // err
      return 0;
    }

    // Build URL from request components
    const scheme = req.scheme || 'https';
    const authority = req.authority || '';
    const path = req.path || '/';
    const url = `${scheme.toLowerCase()}://${authority}${path}`;

    // Find body data
    let bodyBytes: Uint8Array | undefined;
    for (const [rid, res] of wasiHttpResources) {
      if (res.type === 'outgoing-body' && res.data && res.data.length > 0) {
        bodyBytes = new Uint8Array(res.data);
        break;
      }
    }

    // Post to main thread for fetch (Atomics pattern)
    if (bodyBytes) {
      const bodyStr = new TextDecoder().decode(bodyBytes);
      self.postMessage({ type: 'http_request', url, method: req.method, body: bodyStr });
    } else {
      self.postMessage({ type: 'http_request', url, method: req.method });
    }

    // Block until response arrives
    Atomics.wait(sharedBuffer!, 0, 0);
    const responseLen = Atomics.load(sharedBuffer!, 1);
    const sharedBytes = new Uint8Array(sharedBuffer!.buffer);
    const responseData = sharedBytes.slice(8, 8 + responseLen);

    // Write response to response buffer (RETAREA)
    writeWasmBytes(wasmMemory!, RETAREA, responseData);

    // Create incoming-response resource
    const respRid = allocResource({
      type: 'incoming-response',
      statusCode: 200,
      bodyData: responseData,
    });

    // Create future-incoming-response resource (already resolved)
    const firRid = allocResource({ type: 'future-incoming-response', response: respRid });

    // Write success result: ok(0), then the future-incoming-response handle
    // Layout: [ok_tag: u32, resp_handle: u32] at outPtr
    new DataView(wasmMemory!.buffer).setUint32(outPtr, 0, true); // ok
    new DataView(wasmMemory!.buffer).setUint32(outPtr + 4, firRid, true); // handle

    // Reset signal
    Atomics.store(sharedBuffer!, 0, 0);

    return 0;
  },
};

const wasiIoStreams = {
  '[method]output-stream.blocking-write-and-flush': wasiHttpTypes['[method]output-stream.blocking-write-and-flush'],
};

const wasiIoPoll = {
  'poll': (pollableRidsPtr: number, pollableRidsLen: number) => {
    // All pollables are immediately ready
    return allocResource({ type: 'pollable' });
  },
};

const wasiCliStdout = {
  'get-stdout': () => allocResource({ type: 'output-stream' }),
};

const wasiCliStdin = {
  'get-stdin': () => allocResource({ type: 'input-stream' }),
};

const imports = {
  wasi_snapshot_preview1: wasiPolyfills,
  outlayer: outlayerPolyfills,
  'wasi:io/streams@0.2.2': wasiIoStreams,
  'wasi:http/types@0.2.2': wasiHttpTypes,
  'wasi:http/outgoing-handler@0.2.2': wasiHttpOutgoing,
  'wasi:io/poll@0.2.2': wasiIoPoll,
  'wasi:cli/stdout@0.2.2': wasiCliStdout,
  'wasi:cli/stdin@0.2.2': wasiCliStdin,
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