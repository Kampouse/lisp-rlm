import init, { compile_p1, compile_p2, compile_p2_core, compile_pure, disassemble_wasm } from '../../public/wasm/lisp_rlm_browser.js';

let initialized = false;
let initPromise: Promise<void> | null = null;

export type CompileTarget = 'p1' | 'p2' | 'pure';

export interface CompileResult {
  success: boolean;
  wasmBytes: Uint8Array | null;
  size: number;
  timeMs: number;
  error: string | null;
  wat: string | null;
  exports: string[];
  runResult: string | null;
}

export async function initCompiler(): Promise<void> {
  if (initialized) return;
  if (initPromise) return initPromise;

  initPromise = init().then(() => {
    initialized = true;
  });

  return initPromise;
}

export function isInitialized(): boolean {
  return initialized;
}

export function compile(source: string, target: CompileTarget): CompileResult {
  const start = performance.now();

  try {
    let wasmBytes: Uint8Array;
    switch (target) {
      case 'p1': wasmBytes = compile_p1(source); break;
      case 'p2': wasmBytes = compile_p2(source); break;
      case 'pure': wasmBytes = compile_pure(source); break;
      default: wasmBytes = compile_p1(source); break;
    }
    const timeMs = performance.now() - start;

    // Disassemble to WAT
    let wat: string | null = null;
    let exports: string[] = [];
    try {
      wat = disassemble_wasm(wasmBytes);
      exports = extractExports(wat);
    } catch {
      // disassembly is optional
    }

    return {
      success: true,
      wasmBytes,
      size: wasmBytes.length,
      timeMs,
      error: null,
      wat,
      exports,
      runResult: null,
    };
  } catch (err: unknown) {
    const timeMs = performance.now() - start;
    const message = err instanceof Error ? err.message : String(err);

    return {
      success: false,
      wasmBytes: null,
      size: 0,
      timeMs,
      error: message,
      wat: null,
      exports: [],
      runResult: null,
    };
  }
}

/** Compile P2 as core WASM (browser-runnable, before component wrapping). */
export function compileP2Core(source: string): Uint8Array {
  return compile_p2_core(source);
}

/** Build a minimal `env` import object with no-op stubs for all NEAR host functions.
 *  compile_fuzz WASM may still import some env functions even in pure mode. */
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
    sha256: stub, keccak256: stub, random_seed: stub, ed25519_verify: stubI64,
    value_return: stub, panic: stub, panic_utf8: stub, log_utf8: stub, log_utf16: stub,
    promise_create: stubI64, promise_then: stubI64, promise_and: stubI64,
    promise_results_count: stubI64, promise_result: stub, promise_return: stub,
    storage_iter_prefix: stubI64, storage_iter_range: stubI64, storage_iter_next: stubI64,
    promise_batch_create: stubI64, promise_batch_then: stubI64,
  };
}

/** WASI Preview 1 polyfill for browser. */
interface WasiState {
  stdin: Uint8Array;
  stdinOffset: number;
  stdout: string;
  memory: WebAssembly.Memory;
}

function createWasiImports(state: WasiState): WebAssembly.Imports {
  const memView = () => new DataView(state.memory.buffer);

  return {
    wasi_snapshot_preview1: {
      fd_read: (fd: number, iovsPtr: number, iovsLen: number, nreadPtr: number): number => {
        if (fd !== 0) return 8;
        let total = 0;
        for (let i = 0; i < iovsLen && state.stdinOffset < state.stdin.length; i++) {
          const view = memView();
          const iovBufPtr = view.getUint32(iovsPtr + i * 8, true);
          const iovBufLen = view.getUint32(iovsPtr + i * 4 + 4, true);
          const toCopy = Math.min(iovBufLen, state.stdin.length - state.stdinOffset);
          new Uint8Array(state.memory.buffer).set(state.stdin.slice(state.stdinOffset, state.stdinOffset + toCopy), iovBufPtr);
          state.stdinOffset += toCopy;
          total += toCopy;
        }
        return 0;
      },
      fd_write: (fd: number, iovsPtr: number, iovsLen: number, nwrittenPtr: number): number => {
        if (fd !== 1 && fd !== 2) return 8;
        for (let i = 0; i < iovsLen; i++) {
          const view = memView();
          const iovBufPtr = view.getUint32(iovsPtr + i * 8, true);
          const iovBufLen = view.getUint32(iovsPtr + i * 4 + 4, true);
          const chunk = new Uint8Array(state.memory.buffer).slice(iovBufPtr, iovBufPtr + iovBufLen);
          state.stdout += new TextDecoder().decode(chunk);
        }
        return 0;
      },
      proc_exit: (_code: number): void => {
        throw new Error(`exit(${_code})`);
      },
      random_get: (bufPtr: number, bufLen: number): number => {
        const bytes = new Uint8Array(state.memory.buffer);
        crypto.getRandomValues(bytes.slice(bufPtr, bufPtr + bufLen));
        return 0;
      },
      environ_sizes_get: (countPtr: number, bufLenPtr: number): number => {
        memView().setUint32(countPtr, 0, true);
        memView().setUint32(bufLenPtr, 0, true);
        return 0;
      },
      environ_get: (): number => 0,
      fd_seek: (_fd: number, _offset: bigint, _whence: number, _newoffsetPtr: number): number => 0,
    },
    outlayer: {
      // Mock HTTP for browser testing — real HTTP happens on OutLayer
      http_get: (urlPtr: number, urlLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): number => {
        const bytes = new Uint8Array(state.memory.buffer);
        const url = new TextDecoder().decode(bytes.slice(urlPtr, urlPtr + urlLen));
        // Return mock JSON for browser testing
        const mockResponse = JSON.stringify({
          url,
          mocked: true,
          message: "Browser test mode — real HTTP on OutLayer",
          args: {},
          headers: { "User-Agent": "lisp-rlm-browser" },
          origin: "127.0.0.1"
        });
        const respBytes = new TextEncoder().encode(mockResponse);
        const toCopy = Math.min(respBytes.length, respBufLen);
        bytes.set(respBytes.slice(0, toCopy), respBufPtr);
        new DataView(state.memory.buffer).setUint32(respLenPtr, toCopy, true);
        return 0; // success
      },
      // Storage stubs
      storage_set: (k: number, kl: number, v: number, vl: number): number => {
        const mem = new Uint8Array(state.memory.buffer);
        const key = new TextDecoder().decode(mem.slice(k, k + kl));
        const val = new TextDecoder().decode(mem.slice(v, v + vl));
        localStorage.setItem(`lisp-rlm:${key}`, val);
        return 0;
      },
      storage_get: (k: number, kl: number, buf: number, bl: number, lp: number): number => {
        const mem = new Uint8Array(state.memory.buffer);
        const key = new TextDecoder().decode(mem.slice(k, k + kl));
        const val = localStorage.getItem(`lisp-rlm:${key}`) ?? '';
        const vb = new TextEncoder().encode(val);
        mem.set(vb.slice(0, bl), buf);
        new DataView(state.memory.buffer).setUint32(lp, vb.length, true);
        return 0;
      },
      storage_has: (k: number, kl: number): number => {
        const key = new TextDecoder().decode(new Uint8Array(state.memory.buffer).slice(k, k + kl));
        return localStorage.getItem(`lisp-rlm:${key}`) !== null ? 1 : 0;
      },
      storage_delete: (k: number, kl: number): number => {
        const key = new TextDecoder().decode(new Uint8Array(state.memory.buffer).slice(k, k + kl));
        localStorage.removeItem(`lisp-rlm:${key}`);
        return 0;
      },
      storage_increment: () => 0,
      storage_decrement: () => 0,
      storage_set_if_absent: () => 0,
      storage_set_if_equals: () => 0,
      storage_list_keys: () => 0,
      storage_clear_all: () => { localStorage.clear(); return 0; },
      storage_set_worker: () => 0,
      storage_get_worker: () => 0,
      storage_set_worker_public: () => 0,
      storage_get_worker_from_project: () => 0,
      view: () => 0,
      call: () => 0,
      transfer: () => 0,
      env_signer: (buf: number, len: number, lp: number): number => {
        const s = 'browser-user';
        const sb = new TextEncoder().encode(s);
        new Uint8Array(state.memory.buffer).set(sb.slice(0, len), buf);
        new DataView(state.memory.buffer).setUint32(lp, Math.min(sb.length, len), true);
        return 0;
      },
      env_predecessor: (buf: number, len: number, lp: number): number => {
        const s = 'browser-predecessor';
        const sb = new TextEncoder().encode(s);
        new Uint8Array(state.memory.buffer).set(sb.slice(0, len), buf);
        new DataView(state.memory.buffer).setUint32(lp, Math.min(sb.length, len), true);
        return 0;
      },
    },
    env: buildEnvStubs(),
  };
}

/** Run pure WASM in the browser */
export async function runPure(wasmBytes: Uint8Array): Promise<string> {
  const importObject = { env: buildEnvStubs() };
  const { instance } = await WebAssembly.instantiate(wasmBytes.buffer as ArrayBuffer, importObject) as any;
  const exports = instance.exports as Record<string, unknown>;

  if (typeof exports.run === 'function') {
    try {
      (exports.run as Function)();
      const memory = exports.memory as WebAssembly.Memory | undefined;
      if (memory) {
        const buf = new DataView(memory.buffer);
        const tagged = buf.getBigInt64(64, true);
        const tagType = tagged & BigInt(7);
        const payload = tagged >> BigInt(3);

        switch (tagType) {
          case BigInt(0): return payload.toString();
          case BigInt(1): return payload === BigInt(0) ? 'false' : 'true';
          case BigInt(2): return `<fn#${payload}>`;
          case BigInt(3): return `<closure@${payload}>`;
          case BigInt(4): return 'nil';
          case BigInt(5): {
            const lo = Number(payload & BigInt(0xFFFFFFFF));
            const hi = Number((payload >> BigInt(32)) & BigInt(0xFFFFFFFF));
            const bytes = new Uint8Array(memory.buffer).slice(lo, lo + hi);
            return `"${new TextDecoder().decode(bytes)}"`;
          }
          default: return `tagged: ${tagged}`;
        }
      }
      return 'run() → no memory export';
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return `error: ${msg}`;
    }
  }

  if (typeof exports._run === 'function') {
    return '(NEAR contract — use Deploy to run on-chain)';
  }

  const fns = Object.entries(exports)
    .filter(([, v]) => typeof v === 'function')
    .map(([name]) => name);

  return fns.length > 0 ? `Available exports: ${fns.join(', ')}` : '(no runnable exports)';
}

/** Run P2 WASI WASM in the browser with polyfilled imports. */
export async function runWasi(wasmBytes: Uint8Array, stdinData?: Uint8Array): Promise<string> {
  // WASI state - will be populated after instantiation when memory is available
  let stdout = '';
  const stdin = stdinData ?? new Uint8Array(0);
  let stdinOffset = 0;
  
  // Will be set after instantiation
  let getMemory: () => WebAssembly.Memory | undefined = () => undefined;

  const wasi = {
    fd_read: (fd: number, iovsPtr: number, iovsLen: number, nreadPtr: number): number => {
      if (fd !== 0) return 8;
      const mem = getMemory();
      if (!mem) return 8;
      const view = new DataView(mem.buffer);
      const bytes = new Uint8Array(mem.buffer);
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
      const mem = getMemory();
      if (!mem) return 8;
      const view = new DataView(mem.buffer);
      const bytes = new Uint8Array(mem.buffer);
      for (let i = 0; i < iovsLen; i++) {
        const iovBufPtr = view.getUint32(iovsPtr + i * 8, true);
        const iovBufLen = view.getUint32(iovsPtr + i * 4 + 4, true);
        const chunk = bytes.slice(iovBufPtr, iovBufPtr + iovBufLen);
        stdout += new TextDecoder().decode(chunk);
      }
      view.setUint32(nwrittenPtr, 0, true);
      return 0;
    },
    proc_exit: (code: number): void => {
      throw new Error(`WASI exit(${code})`);
    },
    random_get: (bufPtr: number, bufLen: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      crypto.getRandomValues(new Uint8Array(mem.buffer).slice(bufPtr, bufPtr + bufLen));
      return 0;
    },
    environ_sizes_get: (): number => 0,
    environ_get: (): number => 0,
    fd_seek: (): number => 0,
  };

  const outlayer = {
    http_get: async (urlPtr: number, urlLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): Promise<number> => {
      const mem = getMemory();
      if (!mem) return 1;
      const bytes = new Uint8Array(mem.buffer);
      const url = new TextDecoder().decode(bytes.slice(urlPtr, urlPtr + urlLen));
      try {
        const resp = await fetch(url);
        const text = await resp.text();
        const textBytes = new TextEncoder().encode(text);
        const toCopy = Math.min(textBytes.length, respBufLen);
        bytes.set(textBytes.slice(0, toCopy), respBufPtr);
        new DataView(mem.buffer).setUint32(respLenPtr, toCopy, true);
        return 0;
      } catch (e) {
        const errBytes = new TextEncoder().encode(`error: ${e instanceof Error ? e.message : String(e)}`);
        const toCopy = Math.min(errBytes.length, respBufLen);
        bytes.set(errBytes.slice(0, toCopy), respBufPtr);
        new DataView(mem.buffer).setUint32(respLenPtr, toCopy, true);
        return 1;
      }
    },
    storage_set: (k: number, kl: number, v: number, vl: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const bytes = new Uint8Array(mem.buffer);
      localStorage.setItem(
        `lisp-rlm:${new TextDecoder().decode(bytes.slice(k, k + kl))}`,
        new TextDecoder().decode(bytes.slice(v, v + vl))
      );
      return 0;
    },
    storage_get: (k: number, kl: number, buf: number, bl: number, lp: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const bytes = new Uint8Array(mem.buffer);
      const key = new TextDecoder().decode(bytes.slice(k, k + kl));
      const val = localStorage.getItem(`lisp-rlm:${key}`) ?? '';
      const vb = new TextEncoder().encode(val);
      bytes.set(vb.slice(0, bl), buf);
      new DataView(mem.buffer).setUint32(lp, vb.length, true);
      return 0;
    },
    storage_has: (k: number, kl: number): number => {
      const mem = getMemory();
      if (!mem) return 0;
      const key = new TextDecoder().decode(new Uint8Array(mem.buffer).slice(k, k + kl));
      return localStorage.getItem(`lisp-rlm:${key}`) !== null ? 1 : 0;
    },
    storage_delete: (k: number, kl: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const key = new TextDecoder().decode(new Uint8Array(mem.buffer).slice(k, k + kl));
      localStorage.removeItem(`lisp-rlm:${key}`);
      return 0;
    },
    storage_increment: () => 0,
    storage_decrement: () => 0,
    storage_set_if_absent: () => 0,
    storage_set_if_equals: () => 0,
    storage_list_keys: () => 0,
    storage_clear_all: () => { localStorage.clear(); return 0; },
    storage_set_worker: () => 0,
    storage_get_worker: () => 0,
    storage_set_worker_public: () => 0,
    storage_get_worker_from_project: () => 0,
    view: () => 0,
    call: () => 0,
    transfer: () => 0,
    env_signer: (buf: number, len: number, lp: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const s = 'browser-user';
      const sb = new TextEncoder().encode(s);
      new Uint8Array(mem.buffer).set(sb.slice(0, len), buf);
      new DataView(mem.buffer).setUint32(lp, Math.min(sb.length, len), true);
      return 0;
    },
    env_predecessor: (buf: number, len: number, lp: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const s = 'browser-predecessor';
      const sb = new TextEncoder().encode(s);
      new Uint8Array(mem.buffer).set(sb.slice(0, len), buf);
      new DataView(mem.buffer).setUint32(lp, Math.min(sb.length, len), true);
      return 0;
    },
  };

  // Build import object
  const importObj: WebAssembly.Imports = {
    wasi_snapshot_preview1: wasi as unknown as Record<string, WebAssembly.ImportValue>,
    outlayer: outlayer as unknown as Record<string, WebAssembly.ImportValue>,
    env: buildEnvStubs() as unknown as Record<string, WebAssembly.ImportValue>,
  };

  // Instantiate
  const { instance } = await WebAssembly.instantiate(wasmBytes.buffer as ArrayBuffer, importObj);
  const exports = instance.exports as Record<string, WebAssembly.ExportValue>;
  
  // Set memory getter
  getMemory = () => exports.memory as WebAssembly.Memory | undefined;

  // Call _start or run
  const startFn = exports._start ?? exports.run;
  if (typeof startFn === 'function') {
    try {
      (startFn as Function)();
    } catch (e) {
      if (e instanceof Error && e.message.includes('exit')) {
        // Normal exit via proc_exit
      } else {
        throw e;
      }
    }
  }

  return stdout || '(no output)';
}

/** Run P2 WASM Component using jco transpile. */
export async function runComponent(componentBytes: Uint8Array): Promise<string> {
  // P2 always outputs a WASM Component (not core WASM).
  // Components require the component model + preview2-shim (~500KB).
  // For now, show helpful message. Future: full jco integration.
  return `✓ Built as WASM Component (${componentBytes.length} bytes)

Components require:
• @bytecodealliance/jco transpile (~500KB JS runtime)
• WASI preview2-shim for browser
• OutLayer host polyfills (http_get → fetch, storage → localStorage)

Use ⚡ Deploy to run on OutLayer (recommended).
Or run locally: npx jco transpile component.wasm`;
}

function extractExports(wat: string): string[] {
  const exports: string[] = [];
  const re = /\(export\s+"([^"]+)"\s+\(func/g;
  let m;
  while ((m = re.exec(wat)) !== null) {
    exports.push(m[1]);
  }
  return exports;
}

export function toHexDump(bytes: Uint8Array, maxBytes: number = 256): string {
  const slice = bytes.slice(0, maxBytes);
  const lines: string[] = [];

  for (let offset = 0; offset < slice.length; offset += 16) {
    const chunk = slice.slice(offset, Math.min(offset + 16, slice.length));
    const hex = Array.from(chunk)
      .map((b) => b.toString(16).padStart(2, '0'))
      .join(' ');
    const ascii = Array.from(chunk)
      .map((b) => (b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : '.'))
      .join('');
    const addr = offset.toString(16).padStart(8, '0');
    lines.push(`${addr}  ${hex.padEnd(47, ' ')}  |${ascii}|`);
  }

  if (bytes.length > maxBytes) {
    lines.push(`        ... (${bytes.length - maxBytes} more bytes)`);
  }

  return lines.join('\n');
}