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

// NEAR mock runtime context — configurable from the UI
export interface NearContext {
  currentAccount: string;
  signerAccount: string;
  predecessorAccount: string;
  signerPublicKey: string;
  blockIndex: bigint;
  blockTimestamp: bigint;
  epochHeight: bigint;
  accountBalance: bigint;
  attachedDeposit: bigint;
  accountLockedBalance: bigint;
  prepaidGas: bigint;
  rpcUrl: string;
}

const DEFAULT_NEAR_CONTEXT: NearContext = {
  currentAccount: 'browser-contract.testnet',
  signerAccount: 'browser-user.testnet',
  predecessorAccount: 'browser-caller.testnet',
  signerPublicKey: 'ed25519:mock-public-key',
  blockIndex: 12345678n,
  blockTimestamp: BigInt(Date.now() * 1_000_000),
  epochHeight: 42n,
  accountBalance: BigInt('1000000000000000000000000'), // 1M NEAR
  attachedDeposit: 0n,
  accountLockedBalance: 0n,
  prepaidGas: BigInt(300_000_000_000_000), // 300 Tgas
  rpcUrl: 'https://rpc.mainnet.near.org',
};

let nearContext: NearContext = { ...DEFAULT_NEAR_CONTEXT };

export function setNearContext(ctx: Partial<NearContext>) {
  nearContext = { ...nearContext, ...ctx };
}

export function getNearContext(): NearContext {
  return { ...nearContext };
}

export function resetNearContext() {
  nearContext = { ...DEFAULT_NEAR_CONTEXT };
}

// NEAR mock storage (persisted to localStorage)
const nearStorage = new Map<string, Uint8Array>();
const nearRegisters = new Map<number, Uint8Array>();
let nearReturnValue: Uint8Array | null = null;
let nearStdout = '';
let nearLogs: string[] = [];
let nearPanicMsg: string | null = null;
let nearMemory: WebAssembly.Memory | null = null;
let nearInputBuffer: Uint8Array = new Uint8Array(0);

// Storage diff tracking
let nearStorageBefore: Map<string, Uint8Array> = new Map();
let nearStorageDiff: Array<{ key: string; oldVal: string | null; newVal: string | null }> = [];

// Receipt / promise DAG tracking
interface PromiseNode {
  index: number;
  accountId: string;
  methodName: string;
  argsSize: number;
  result?: Uint8Array;
  type: 'create' | 'then' | 'and';
  callbackIdx?: number;
}
let nearPromiseNodes: PromiseNode[] = [];

function nearMemView() { return new DataView(nearMemory!.buffer); }
function nearMemBytes() { return new Uint8Array(nearMemory!.buffer); }

// Promise result table for cross-contract calls
const nearPromiseResults: Map<number, Uint8Array> = new Map();
const nearPendingPromises: Array<{ accountId: string; methodName: string; argsBase64: string }> = [];
let nearPromiseCounter = 0;
let nearPass = 0; // 0 = first pass (queue), 1 = second pass (serve results)

/** Synchronous NEAR RPC view call — uses XMLHttpRequest */
function nearRpcViewCall(accountId: string, methodName: string, argsBase64: string): Uint8Array | null {
  const rpcUrl = nearContext.rpcUrl;
  try {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', rpcUrl, false); // synchronous
    xhr.setRequestHeader('Content-Type', 'application/json');
    xhr.timeout = 15000;
    const body = JSON.stringify({
      jsonrpc: '2.0',
      id: 'lisp-rlm',
      method: 'query',
      params: {
        request_type: 'call_function',
        finality: 'final',
        account_id: accountId,
        method_name: methodName,
        args_base64: argsBase64,
      },
    });
    xhr.send(body);
    if (xhr.status === 200) {
      const resp = JSON.parse(xhr.responseText);
      if (resp.result?.result) {
        return new Uint8Array(resp.result.result);
      }
    }
    return null;
  } catch {
    return null;
  }
}

function buildNearEnv(): Record<string, Function> {
  return {
    // ===== NEAR Host Functions (Lisp compiler ABI) =====
    // 
    // The Lisp compiler uses these calling conventions:
    //   storage_write (#17): (key_len, key_ptr, val_len, val_ptr, register_id) -> i64
    //   storage_read  (#18): (key_len, key_ptr, register_id) -> i64
    //   storage_remove (#19): (key_len, key_ptr, register_id) -> i64
    //   storage_has_key (#20): (key_len, key_ptr) -> i64
    //   read_register  (#0):  (register_id, ptr) -> void
    //
    // key_ptr/key_len come from tagged string: key_ptr = lower 32 bits, key_len = upper 32 bits >> 32
    // Values are stored as raw 8-byte little-endian i64 at val_ptr
    
    // #0: read_register(register_id, dest_ptr) — copies register data to memory
    read_register: (registerId: bigint, destPtr: bigint) => {
      const data = nearRegisters.get(Number(registerId));
      if (data) {
        nearMemBytes().set(data, Number(destPtr));
      }
    },
    // #1: register_len(register_id) -> u64
    register_len: (registerId: bigint): bigint => {
      const data = nearRegisters.get(Number(registerId));
      return data ? BigInt(data.length) : 0n;
    },

    // #17: storage_write(key_len, key_ptr, val_len, val_ptr, register_id) -> i64
    storage_write: (keyLen: bigint, keyPtr: bigint, valLen: bigint, valPtr: bigint, registerId: bigint): bigint => {
      const keyBytes = nearMemBytes().slice(Number(keyPtr), Number(keyPtr + keyLen));
      const key = new TextDecoder().decode(keyBytes);
      const valueBytes = nearMemBytes().slice(Number(valPtr), Number(valPtr + valLen));
      const existed = nearStorage.has(key) ? 1n : 0n;
      // Store the value bytes (raw i64 LE)
      nearStorage.set(key, valueBytes);
      saveNearStorage();
      // If key existed before, write old value to register
      if (existed) {
        // (optional: write evicted value to register — not critical for mock)
      }
      return existed;
    },
    // #18: storage_read(key_len, key_ptr, register_id) -> i64 (0=not found, 1=found)
    storage_read: (keyLen: bigint, keyPtr: bigint, registerId: bigint): bigint => {
      const keyBytes = nearMemBytes().slice(Number(keyPtr), Number(keyPtr + keyLen));
      const key = new TextDecoder().decode(keyBytes);
      const valueBytes = nearStorage.get(key);
      if (valueBytes === undefined) {
        return 0n;
      }
      // Write value to register
      nearRegisters.set(Number(registerId), valueBytes);
      return 1n;
    },
    // #19: storage_remove(key_len, key_ptr, register_id) -> i64
    storage_remove: (keyLen: bigint, keyPtr: bigint, registerId: bigint): bigint => {
      const keyBytes = nearMemBytes().slice(Number(keyPtr), Number(keyPtr + keyLen));
      const key = new TextDecoder().decode(keyBytes);
      const existed = nearStorage.has(key) ? 1n : 0n;
      if (existed) {
        // Write evicted value to register
        nearRegisters.set(Number(registerId), nearStorage.get(key)!);
      }
      nearStorage.delete(key);
      saveNearStorage();
      return existed;
    },
    // #20: storage_has_key(key_len, key_ptr) -> i64
    storage_has_key: (keyLen: bigint, keyPtr: bigint): bigint => {
      const keyBytes = nearMemBytes().slice(Number(keyPtr), Number(keyPtr + keyLen));
      const key = new TextDecoder().decode(keyBytes);
      return nearStorage.has(key) ? 1n : 0n;
    },

    // ===== Context =====
    current_account_id: (resultPtr: bigint) => {
      writeNearString(Number(resultPtr), nearContext.currentAccount);
    },
    signer_account_id: (resultPtr: bigint) => {
      writeNearString(Number(resultPtr), nearContext.signerAccount);
    },
    predecessor_account_id: (resultPtr: bigint) => {
      writeNearString(Number(resultPtr), nearContext.predecessorAccount);
    },
    block_index: (): bigint => nearContext.blockIndex,
    block_timestamp: (): bigint => nearContext.blockTimestamp,
    epoch_height: (): bigint => nearContext.epochHeight,
    account_balance: (resultPtr: bigint) => {
      writeNearU128(Number(resultPtr), nearContext.accountBalance);
    },
    attached_deposit: (resultPtr: bigint) => {
      writeNearU128(Number(resultPtr), nearContext.attachedDeposit);
    },
    prepaid_gas: (): bigint => nearContext.prepaidGas,
    used_gas: (): bigint => BigInt(1_000_000_000_000),
    input: (registerId: bigint) => {
      nearRegisters.set(Number(registerId), nearInputBuffer.slice());
    },

    // ===== Crypto (stubs) =====
    sha256: (dataPtr: bigint, dataLen: bigint, resultPtr: bigint) => {
      nearMemBytes().set(new Uint8Array(32), Number(resultPtr));
    },
    random_seed: (resultPtr: bigint) => {
      const seed = new Uint8Array(32);
      crypto.getRandomValues(seed);
      nearMemBytes().set(seed, Number(resultPtr));
    },
    keccak256: (dataPtr: bigint, dataLen: bigint, resultPtr: bigint) => {
      nearMemBytes().set(new Uint8Array(32), Number(resultPtr));
    },
    ed25519_verify: (): bigint => 1n,

    // ===== Promise (cross-contract view calls via RPC) =====
    // promise_create(account_id_len, account_id_ptr, method_name_len, method_name_ptr,
    //                arguments_len, arguments_ptr, amount_ptr, gas) → promise_index
    promise_create: (accountIdLen: bigint, accountIdPtr: bigint, methodNameLen: bigint, methodNamePtr: bigint,
                     argsLen: bigint, argsPtr: bigint, _amountPtr: bigint, _gas: bigint): bigint => {
      const mem = nearMemBytes();
      const accountId = new TextDecoder().decode(mem.slice(Number(accountIdPtr), Number(accountIdPtr + accountIdLen)));
      const methodName = new TextDecoder().decode(mem.slice(Number(methodNamePtr), Number(methodNamePtr + methodNameLen)));
      const argsBytes = mem.slice(Number(argsPtr), Number(argsPtr + argsLen));
      const argsBase64 = argsLen > 0n ? btoa(String.fromCharCode(...argsBytes)) : '';

      nearStdout += `  ⤏ ${accountId}/${methodName}(${argsLen > 0n ? `${argsLen}B args` : 'no args'})\n`;

      const pidx = nearPromiseCounter++;

      // Track receipt node
      nearPromiseNodes.push({
        index: pidx,
        accountId,
        methodName,
        argsSize: Number(argsLen),
        type: 'create',
      });

      if (nearPass === 0) {
        // First pass: queue the RPC call
        nearPendingPromises.push({ accountId, methodName, argsBase64 });
      }
      // On second pass, the result is already in nearPromiseResults

      return BigInt(pidx);
    },

    promise_then: (_promiseIdx: bigint, _accountIdLen: bigint, _accountIdPtr: bigint,
                   _methodNameLen: bigint, _methodNamePtr: bigint, _argsLen: bigint, _argsPtr: bigint,
                   _amountPtr: bigint, _gas: bigint): bigint => {
      // Callbacks not supported in view-only mode — return a new promise index
      nearStdout += `  ⚠ promise_then not supported in view-only mode\n`;
      return BigInt(nearPromiseCounter++);
    },

    promise_and: (_ptr: bigint, _count: bigint): bigint => {
      return BigInt(nearPromiseCounter++);
    },

    promise_results_count: (): bigint => {
      return BigInt(nearPromiseResults.size);
    },

    // promise_result(result_idx, register_id) → writes result bytes to register
    promise_result: (resultIdx: bigint, registerId: bigint) => {
      const data = nearPromiseResults.get(Number(resultIdx));
      if (data) {
        nearRegisters.set(Number(registerId), data);
      }
    },

    promise_return: (_promiseIdx: bigint) => {
      // No-op in mock — the return value is already captured
    },
    promise_batch_create: (): bigint => BigInt(0),
    promise_batch_then: (): bigint => BigInt(0),
    promise_batch_action_create_account: () => {},
    promise_batch_action_deploy_contract: () => {},
    promise_batch_action_function_call: () => {},
    promise_batch_action_transfer: () => {},
    promise_batch_action_stake: () => {},
    promise_batch_action_add_key_with_full_access: () => {},
    promise_batch_action_add_key_with_function_call: () => {},
    promise_batch_action_delete_key: () => {},
    promise_batch_action_delete_account: () => {},

    // ===== Misc =====
    storage_usage: (): bigint => BigInt(nearStorage.size * 64),
    signer_account_pk: (resultPtr: bigint) => {
      writeNearString(Number(resultPtr), nearContext.signerPublicKey);
    },
    account_locked_balance: (resultPtr: bigint) => {
      writeNearU128(Number(resultPtr), nearContext.accountLockedBalance);
    },
    storage_iter_prefix: (): bigint => BigInt(0),
    storage_iter_range: (): bigint => BigInt(0),
    storage_iter_next: () => {},
    // #25: value_return(len, ptr)
    value_return: (len: bigint, ptr: bigint) => {
      if (nearReturnValue !== null) return; // only capture the first call
      const bytes = nearMemBytes().slice(Number(ptr), Number(ptr + len));
      nearReturnValue = bytes;
    },
    // #26: panic
    panic: () => {
      nearPanicMsg = 'panic';
      throw new Error('NEAR panic');
    },
    // #27: panic_utf8(len, ptr)
    panic_utf8: (len: bigint, ptr: bigint) => {
      const msg = new TextDecoder().decode(nearMemBytes().slice(Number(ptr), Number(ptr + len)));
      nearPanicMsg = msg;
      throw new Error(`NEAR panic: ${msg}`);
    },
    // #28: log_utf8(len, ptr)
    log_utf8: (len: bigint, ptr: bigint) => {
      const raw = nearMemView().getBigInt64(Number(ptr), true);
      const tagType = Number(raw & BigInt(7));
      const payload = raw >> BigInt(3);
      let msg: string;
      switch (tagType) {
        case 0: msg = payload.toString(); break;
        case 1: msg = payload === BigInt(0) ? 'false' : 'true'; break;
        case 4: msg = 'nil'; break;
        case 5: {
          const lo = Number(payload & BigInt(0xFFFFFFFF));
          const hi = Number((payload >> BigInt(32)) & BigInt(0xFFFFFFFF));
          msg = new TextDecoder().decode(nearMemBytes().slice(lo, lo + hi));
          break;
        }
        default: msg = `tagged(${tagType}:${payload})`;
      }
      nearStdout += msg + '\n';
      nearLogs.push(msg);
    },
    // #29: log_utf16(len, ptr)
    log_utf16: () => {},
  };
}

function writeNearString(ptr: number, str: string) {
  const bytes = new TextEncoder().encode(str);
  nearMemBytes().set(bytes, ptr);
  nearMemView().setUint32(ptr - 4, bytes.length, true); // Length prefix
}

function writeNearU128(ptr: number, value: bigint) {
  nearMemView().setBigUint64(ptr, value & ((1n << 64n) - 1n), true); // lo
  nearMemView().setBigUint64(ptr + 8, value >> 64n, true); // hi
}

function saveNearStorage() {
  try {
    const data: Record<string, string> = {};
    nearStorage.forEach((v, k) => {
      // Convert Uint8Array to base64 for JSON serialization
      data[k] = btoa(String.fromCharCode(...v));
    });
    localStorage.setItem('near_mock_storage', JSON.stringify(data));
  } catch {}
}

function loadNearStorage() {
  try {
    const saved = localStorage.getItem('near_mock_storage');
    if (saved) {
      const data = JSON.parse(saved);
      for (const [k, v] of Object.entries(data)) {
        const str = v as string;
        nearStorage.set(k, Uint8Array.from(atob(str), c => c.charCodeAt(0)));
      }
    }
  } catch {}
}

/** Run NEAR (P1) contract in browser with mocked runtime */
export async function runNear(
  wasmBytes: Uint8Array,
  options?: {
    method?: string;       // Call a single method. If omitted, calls all.
    input?: Uint8Array;    // Input bytes for the method (written via `input` host fn)
    gasLimit?: bigint;     // Gas limit for instruction counting
  }
): Promise<{
  stdout: string;
  returnValue: Uint8Array | null;
  methods: string[];
  gasUsed: number;
}> {
  nearStorage.clear();
  nearRegisters.clear();
  nearReturnValue = null;
  nearStdout = '';
  nearLogs = [];
  nearPanicMsg = null;
  nearPromiseResults.clear();
  nearPendingPromises.length = 0;
  nearPromiseCounter = 0;
  nearPromiseNodes = [];
  nearPass = 0;
  nearStorageDiff = [];
  loadNearStorage();

  // Set up input buffer — the `input` host fn will return this data via register
  nearInputBuffer = options?.input ?? new Uint8Array(0);

  // Gas tracking — static WASM opcode analysis (NEAR pricing)
  let gasUsed = 0;
  let gasBreakdown: { opcodes: number; opcodeGas: number; hostGas: number } | null = null;
  const gasLimit = options?.gasLimit ?? BigInt(300_000_000_000_000); // 300 Tgas

  const env = buildNearEnv();

  const imports: WebAssembly.Imports = {
    env: {
      ...env,
    },
  };

  const { instance } = await WebAssembly.instantiate(wasmBytes.buffer as ArrayBuffer, imports) as any;
  const exports = instance.exports as Record<string, unknown>;

  // Set memory reference from the module's exported memory (has data segments loaded)
  nearMemory = exports.memory as WebAssembly.Memory;

  // List all function exports (skip memory, table, etc)
  const allExports = Object.keys(exports).filter(k => typeof exports[k] === 'function');
  const methods = allExports.filter(f => f !== '_run');
  nearStdout += `Methods: ${methods.join(', ')}\n`;

  // Direct function calls (gas computed statically from WASM binary)
  const callFn = (fn: Function) => { fn(); };

  // Storage diff helpers
  const snapshotStorage = () => {
    const snap = new Map<string, Uint8Array>();
    nearStorage.forEach((v, k) => snap.set(k, v.slice()));
    return snap;
  };

  const decodeVal = (v: Uint8Array): string => {
    if (v.length === 8) {
      const n = new DataView(v.buffer, v.byteOffset, 8).getBigInt64(0, true);
      return n.toString();
    }
    try { return new TextDecoder().decode(v); } catch { return Array.from(v).map(b => b.toString(16).padStart(2, '0')).join(''); }
  };

  const computeDiff = (before: Map<string, Uint8Array>) => {
    const diff: Array<{ key: string; oldVal: string | null; newVal: string | null }> = [];
    const allKeys = new Set<string>();
    before.forEach((_, k) => allKeys.add(k));
    nearStorage.forEach((_, k) => allKeys.add(k));
    allKeys.forEach(k => {
      const had = before.has(k);
      const has = nearStorage.has(k);
      if (had && has) {
        const oldV = before.get(k)!;
        const newV = nearStorage.get(k)!;
        if (oldV.length !== newV.length || oldV.some((b, i) => b !== newV[i])) {
          diff.push({ key: k, oldVal: decodeVal(oldV), newVal: decodeVal(newV) });
        }
      } else if (!had && has) {
        diff.push({ key: k, oldVal: null, newVal: decodeVal(nearStorage.get(k)!) });
      } else if (had && !has) {
        diff.push({ key: k, oldVal: decodeVal(before.get(k)!), newVal: null });
      }
    });
    return diff;
  };

  // Helper: run a method (or all methods) — shared between passes
  const runMethods = () => {
    if (options?.method) {
      const fn = exports[options.method] as Function | undefined;
      if (!fn) {
        throw new Error(`Method "${options.method}" not found. Available: ${methods.join(', ')}`);
      }
      nearStdout += `▸ ${options.method}()\n`;
      callFn(fn);
    } else {
      for (const name of allExports) {
        if (name === '_run' || name === 'memory') continue;
        try {
          nearStdout += `▸ ${name}()\n`;
          callFn(exports[name] as Function);
        } catch (e: any) {
          if (e?.message === 'NEAR_RETURN' || e?.message === 'NEAR panic') continue;
          throw e;
        }
      }
    }
  };

  const storageBefore = snapshotStorage();

  try {
    // === Pass 1: Queue promise calls ===
    runMethods();

    // If promises were queued, execute RPCs and run again
    if (nearPendingPromises.length > 0) {
      nearStdout += `\n--- Resolving ${nearPendingPromises.length} cross-contract call(s) ---\n`;

      for (let i = 0; i < nearPendingPromises.length; i++) {
        const { accountId, methodName, argsBase64 } = nearPendingPromises[i];
        const result = await (await fetch(nearContext.rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: `lisp-rlm-${i}`,
            method: 'query',
            params: {
              request_type: 'call_function',
              finality: 'final',
              account_id: accountId,
              method_name: methodName,
              args_base64: argsBase64,
            },
          }),
        }).then(r => r.json())).result;

        if (result?.result) {
          const data = new Uint8Array(result.result as number[]);
          nearPromiseResults.set(i, data);
          nearStdout += `  ← ${accountId}/${methodName}: ${data.length}B → ${new TextDecoder().decode(data)}\n`;
        } else {
          nearPromiseResults.set(i, new Uint8Array(0));
          nearStdout += `  ← ${accountId}/${methodName}: failed (${result?.error || 'unknown error'})\n`;
        }
      }

      nearStdout += `\n--- Replaying with results ---\n`;

      // === Pass 2: Re-run with results populated ===
      nearPass = 1;
      nearPromiseCounter = 0;
      nearStdout += `Methods: ${methods.join(', ')}\n`;

      // Re-instantiate WASM for clean state
      const { instance: inst2 } = await WebAssembly.instantiate(wasmBytes.buffer as ArrayBuffer, imports) as any;
      const exports2 = inst2.exports as Record<string, unknown>;
      nearMemory = exports2.memory as WebAssembly.Memory;
      loadNearStorage();

      // Re-run the method with promise results available
      if (options?.method) {
        const fn = exports2[options.method] as Function;
        nearStdout += `▸ ${options.method}()\n`;
        callFn(fn);
      } else {
        for (const name of allExports) {
          if (name === '_run' || name === 'memory') continue;
          try {
            nearStdout += `▸ ${name}()\n`;
            callFn(exports2[name] as Function);
          } catch (e: any) {
            if (e?.message === 'NEAR_RETURN' || e?.message === 'NEAR panic') continue;
            throw e;
          }
        }
      }
    }
  } catch (err: unknown) {
    if (!(err instanceof Error && (err.message === 'NEAR_RETURN' || err.message === 'NEAR panic' || err.message?.startsWith('NEAR panic:')))) {
      throw err;
    }
  }

  // Compute storage diff
  nearStorageDiff = computeDiff(storageBefore);

  // Static gas estimation from WASM binary
  const targetMethod = options?.method ?? (methods.length === 1 ? methods[0] : undefined);
  if (targetMethod) {
    const est = estimateGas(wasmBytes, targetMethod);
    if (est) {
      gasUsed = est.totalGas;
      gasBreakdown = { opcodes: est.opcodes, opcodeGas: est.opcodeGas, hostGas: est.hostGas };
    }
  }

  // Attach results to promise nodes
  nearPromiseNodes.forEach(n => {
    const result = nearPromiseResults.get(n.index);
    if (result) n.result = result;
  });

  return {
    stdout: nearStdout, returnValue: nearReturnValue, methods, gasUsed, gasBreakdown,
    logs: nearLogs, panic: nearPanicMsg, storageDiff: nearStorageDiff, receipts: nearPromiseNodes,
  } as {
    stdout: string; returnValue: Uint8Array | null; methods: string[]; gasUsed: number;
    gasBreakdown: { opcodes: number; opcodeGas: number; hostGas: number } | null;
    logs: string[]; panic: string | null;
    storageDiff: Array<{ key: string; oldVal: string | null; newVal: string | null }>;
    receipts: typeof nearPromiseNodes;
  };
}

/** Get current NEAR mock storage state */
export function getNearStorage(): Record<string, string> {
  const result: Record<string, string> = {};
  nearStorage.forEach((v, k) => {
    // Try to decode as LE i64 integer
    if (v.length === 8) {
      const view = new DataView(v.buffer, v.byteOffset, v.byteLength);
      const val = view.getBigInt64(0, true);
      result[k] = val.toString();
    } else {
      // Fallback: show as hex
      result[k] = '0x' + Array.from(v).map(b => b.toString(16).padStart(2, '0')).join('');
    }
  });
  return result;
}

/** Decode a return value from raw bytes to display string */
export function decodeReturnValue(bytes: Uint8Array | null): string | null {
  if (!bytes || bytes.length === 0) return null;
  if (bytes.length === 8) {
    // Try i64
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const val = view.getBigInt64(0, true);
    return val.toString();
  }
  if (bytes.length === 16) {
    // Try U128
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const lo = view.getBigUint64(0, true);
    const hi = view.getBigUint64(8, true);
    return (hi << 64n | lo).toString();
  }
  // Try UTF-8 string
  try {
    const str = new TextDecoder().decode(bytes);
    if (/^[\x20-\x7E]+$/.test(str)) return `"${str}"`;
  } catch {}
  // Hex
  return '0x' + Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

/** Format gas units to human-readable */
export function formatGas(gasUnits: number): string {
  if (gasUnits >= 1_000_000_000_000) {
    return `${(gasUnits / 1_000_000_000_000).toFixed(2)} Tgas`;
  }
  if (gasUnits >= 1_000_000_000) {
    return `${(gasUnits / 1_000_000_000).toFixed(2)} Ggas`;
  }
  if (gasUnits >= 1_000_000) {
    return `${(gasUnits / 1_000_000).toFixed(2)} Mgas`;
  }
  return `${gasUnits} gas`;
}

// ═══════════════════════════════════════════════
// Static WASM gas estimator (NEAR opcode pricing)
// ═══════════════════════════════════════════════

const NEAR_REGULAR_OP_COST = 822_756;

// Host function NAME → base gas cost (from NEAR mainnet protocol config)
const HOST_GAS_BY_NAME: Record<string, number> = {
  read_register: 2_517_165_186,
  register_len: 2_609_863_200,
  write_register: 2_865_522_486,
  current_account_id: 0,
  signer_account_id: 0,
  signer_account_pk: 0,
  predecessor_account_id: 0,
  input: 0,
  block_index: 0,
  block_timestamp: 0,
  epoch_height: 0,
  storage_usage: 0,
  account_balance: 0,
  account_locked_balance: 0,
  attached_deposit: 0,
  prepaid_gas: 0,
  used_gas: 0,
  storage_write: 64_196_736_000,
  storage_read: 56_356_845_749,
  storage_remove: 53_473_030_500,
  storage_has_key: 54_039_896_625,
  sha256: 4_540_970_250,
  keccak256: 5_879_491_275,
  random_seed: 0,
  ed25519_verify: 210_000_000_000,
  value_return: 0,
  panic: 0,
  panic_utf8: 0,
  log_utf8: 3_543_313_050,
  log_utf16: 3_543_313_050,
  promise_create: 0,
  promise_then: 0,
  promise_and: 1_465_013_400,
  promise_results_count: 0,
  promise_result: 0,
  promise_return: 560_152_386,
  storage_iter_prefix: 0,
  storage_iter_range: 0,
  storage_iter_next: 0,
  promise_batch_create: 0,
  promise_batch_then: 0,
  promise_batch_action_create_account: 0,
  promise_batch_action_deploy_contract: 0,
  promise_batch_action_function_call: 0,
  promise_batch_action_transfer: 0,
  promise_batch_action_stake: 0,
};

// WASM opcodes that carry extra immediates (skip over them during counting)
// Returns number of bytes to skip after the opcode byte
function wasmImmediateSize(opcode: number, bytes: Uint8Array, offset: number): number {
  // LEB128 varuint32
  const readLEB = (off: number): [number, number] => {
    let val = 0, shift = 0, read = 0;
    while (true) {
      const b = bytes[off++];
      val |= (b & 0x7f) << shift;
      read++;
      if ((b & 0x80) === 0) break;
      shift += 7;
    }
    return [val, read];
  };
  // LEB128 varint64
  const readLEBs64 = (off: number): number => {
    let read = 0;
    while (true) {
      const b = bytes[off++];
      read++;
      if ((b & 0x80) === 0) break;
    }
    return read;
  };

  const o = offset;
  switch (opcode) {
    // Memory instructions: skip alignment (varuint32) + offset (varuint32)
    case 0x28: case 0x29: case 0x2A: case 0x2B: case 0x2C: case 0x2D: case 0x2E: case 0x2F:
    case 0x35: case 0x36: case 0x37: case 0x38: case 0x39: case 0x3A: case 0x3B: case 0x3C:
    case 0x3D: case 0x3E: case 0x3F: case 0x40: {
      const [, a] = readLEB(o);
      const [, b] = readLEB(o + a);
      return a + b;
    }
    // const i32: varint32
    case 0x41: return readLEB(o)[1];
    // const i64: varint64
    case 0x42: return readLEBs64(o);
    // const f32: 4 bytes
    case 0x43: return 4;
    // const f64: 8 bytes
    case 0x44: return 8;
    // global.get / global.set: varuint32
    case 0x23: case 0x24: return readLEB(o)[1];
    // local.get / local.set / local.tee: varuint32
    case 0x20: case 0x21: case 0x22: return readLEB(o)[1];
    // br / br_if: varuint32
    case 0x0C: case 0x0D: return readLEB(o)[1];
    // br_table: vec(varuint32) + varuint32
    case 0x0E: {
      const [count, c1] = readLEB(o);
      let skip = c1;
      for (let i = 0; i <= count; i++) skip += readLEB(o + skip)[1];
      return skip;
    }
    // call: varuint32
    case 0x10: return readLEB(o)[1];
    // call_indirect: varuint32 + varuint32
    case 0x11: {
      const [, a] = readLEB(o);
      return a + readLEB(o + a)[1];
    }
    // block / loop / if: blocktype (1 byte for void, or varint33)
    case 0x02: case 0x03: case 0x04: {
      // blocktype: 0x40 = void, otherwise s33 LEB128
      if (bytes[o] === 0x40) return 1;
      return readLEBs64(o);
    }
    default: return 0;
  }
}

export interface GasEstimate {
  opcodes: number;
  opcodeGas: number;
  hostGas: number;
  totalGas: number;
  hostCalls: Record<number, number>; // host fn index → call count
}

/** Estimate gas for a specific function in compiled WASM using NEAR opcode pricing */
export function estimateGas(wasmBytes: Uint8Array, methodName: string): GasEstimate | null {
  const bytes = wasmBytes instanceof Uint8Array ? wasmBytes : new Uint8Array(wasmBytes);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  // Parse WASM sections to find code section and function section
  let codeBodyCount = 0;
  let codeSectionOffset = -1;
  let codeSectionEnd = -1;

  // Also find export section to map method name → function index
  let funcCountBeforeExport = 0; // number of functions before the export section type section
  let funcTypeCount = 0;
  let importFuncCount = 0;
  let exportFuncIndex = -1;
  const importFuncNames: string[] = []; // import func name by import-index

  // First pass: count imports (they shift function indices)
  let pos = 8; // skip magic + version
  while (pos < bytes.length) {
    const sectionId = bytes[pos++];
    const [sectionLen, lenBytes] = (() => {
      let val = 0, shift = 0, read = 0;
      while (true) {
        const b = bytes[pos++];
        val |= (b & 0x7f) << shift;
        read++;
        if ((b & 0x80) === 0) break;
        shift += 7;
      }
      return [val, read] as [number, number];
    })();
    const sectionEnd = pos + sectionLen;

    if (sectionId === 2) {
      // Import section — collect function import names
      const [numImports, n1] = (() => {
        let val = 0, shift = 0, read = 0, off = pos;
        while (true) { const b = bytes[off++]; val |= (b & 0x7f) << shift; read++; if ((b & 0x80) === 0) break; shift += 7; }
        return [val, read] as [number, number];
      })();
      let ipos = pos + n1;
      for (let i = 0; i < numImports; i++) {
        // module name
        const [modLen, m1] = (() => { let v=0,s=0,r=0,o=ipos; while(true){const b=bytes[o++];v|=(b&0x7f)<<s;r++;if((b&0x80)===0)break;s+=7;} return [v,r] as [number,number]; })();
        const modName = new TextDecoder().decode(bytes.slice(ipos + m1, ipos + m1 + modLen));
        ipos += modLen + m1;
        // field name
        const [fieldLen, f1] = (() => { let v=0,s=0,r=0,o=ipos; while(true){const b=bytes[o++];v|=(b&0x7f)<<s;r++;if((b&0x80)===0)break;s+=7;} return [v,r] as [number,number]; })();
        const fieldName = new TextDecoder().decode(bytes.slice(ipos + f1, ipos + f1 + fieldLen));
        ipos += fieldLen + f1;
        // kind
        const kind = bytes[ipos++];
        if (kind === 0) {
          // Function import: skip type index
          (() => { let r=0; while(true){const b=bytes[ipos++];r++;if((b&0x80)===0)break;} })();
          if (modName === 'env') importFuncNames.push(fieldName);
          importFuncCount++;
        } else if (kind === 1) {
          // Table: elemtype + limits
          ipos++; // elemtype
          const flags = bytes[ipos++];
          (() => { let r=0; while(true){const b=bytes[ipos++];r++;if((b&0x80)===0)break;} })();
          if (flags & 1) (() => { let r=0; while(true){const b=bytes[ipos++];r++;if((b&0x80)===0)break;} })();
        } else if (kind === 2) {
          // Memory: limits
          const flags = bytes[ipos++];
          (() => { let r=0; while(true){const b=bytes[ipos++];r++;if((b&0x80)===0)break;} })();
          if (flags & 1) (() => { let r=0; while(true){const b=bytes[ipos++];r++;if((b&0x80)===0)break;} })();
        } else if (kind === 3) {
          // Global: type + mutability
          ipos += 2;
        }
      }
    }

    if (sectionId === 7) {
      // Export section — find our method
      const readLEB = (off: number): [number, number] => {
        let val = 0, shift = 0, read = 0;
        while (true) { const b = bytes[off++]; val |= (b & 0x7f) << shift; read++; if ((b & 0x80) === 0) break; shift += 7; }
        return [val, read] as [number, number];
      };
      const [numExports, ne] = readLEB(pos);
      let epos = pos + ne;
      for (let i = 0; i < numExports; i++) {
        const [nameLen, n1] = readLEB(epos); epos += n1;
        const name = new TextDecoder().decode(bytes.slice(epos, epos + nameLen));
        epos += nameLen;
        const kind = bytes[epos++];
        const [idx, i1] = readLEB(epos); epos += i1;
        if (kind === 0 && name === methodName) {
          exportFuncIndex = idx - importFuncCount; // local function index
        }
      }
    }

    if (sectionId === 10) {
      // Code section
      codeSectionOffset = pos;
      codeSectionEnd = sectionEnd;
      const [numBodies, nb] = (() => {
        let val = 0, shift = 0, read = 0, off = pos;
        while (true) { const b = bytes[off++]; val |= (b & 0x7f) << shift; read++; if ((b & 0x80) === 0) break; shift += 7; }
        return [val, read] as [number, number];
      })();
      codeBodyCount = numBodies;
    }

    pos = sectionEnd;
  }

  if (exportFuncIndex < 0 || codeSectionOffset < 0) return null;

  // Parse code section to find the specific function body
  const readLEB = (off: number): [number, number] => {
    let val = 0, shift = 0, read = 0;
    while (true) { const b = bytes[off++]; val |= (b & 0x7f) << shift; read++; if ((b & 0x80) === 0) break; shift += 7; }
    return [val, read] as [number, number];
  };

  let bodyPos = codeSectionOffset;
  const [, nb] = readLEB(bodyPos);
  bodyPos += nb; // skip body count

  let funcBodyStart = -1;
  let funcBodyEnd = -1;

  for (let i = 0; i < codeBodyCount; i++) {
    const [bodySize, bs] = readLEB(bodyPos);
    const bodyStart = bodyPos + bs;
    const bodyEnd = bodyStart + bodySize;

    if (i === exportFuncIndex) {
      funcBodyStart = bodyStart;
      funcBodyEnd = bodyEnd;
      break;
    }
    bodyPos = bodyEnd;
  }

  if (funcBodyStart < 0) return null;

  // Count opcodes in the function body
  // Skip local declarations first
  let p = funcBodyStart;
  const [numLocalDecls, nl] = readLEB(p);
  p += nl;
  for (let i = 0; i < numLocalDecls; i++) {
    const [, c1] = readLEB(p); p += c1; // count
    p++; // type
  }

  let opcodeCount = 0;
  const hostCallCounts: Record<number, number> = {};
  let callTargets: number[] = []; // function indices called

  while (p < funcBodyEnd) {
    const op = bytes[p++];

    if (op === 0x0B) continue; // end
    if (op === 0x00) continue; // unreachable
    if (op === 0x01) continue; // nop

    if (op === 0x10) {
      // call — track target
      const [target, t1] = readLEB(p);
      p += t1;
      callTargets.push(target);
      const hostName = target < importFuncNames.length ? importFuncNames[target] : undefined;
      if (hostName && HOST_GAS_BY_NAME[hostName] !== undefined) {
        hostCallCounts[target] = (hostCallCounts[target] || 0) + 1;
      }
      opcodeCount++;
      continue;
    }

    // Skip immediates
    const skip = wasmImmediateSize(op, bytes, p);
    p += skip;
    opcodeCount++;
  }

  // Recursively estimate gas for called local functions (with cycle detection)
  const visited = new Set<number>();
  const countOpcodes = (localFuncIdx: number): { ops: number; hosts: Record<number, number> } => {
    if (visited.has(localFuncIdx)) return { ops: 0, hosts: {} };
    visited.add(localFuncIdx);

    // Find the function body
    let bp = codeSectionOffset;
    const [, nb2] = readLEB(bp); bp += nb2;
    let fStart = -1, fEnd = -1;
    for (let i = 0; i < codeBodyCount; i++) {
      const [sz, s1] = readLEB(bp);
      const start = bp + s1;
      const end = start + sz;
      if (i === localFuncIdx) { fStart = start; fEnd = end; break; }
      bp = end;
    }
    if (fStart < 0) return { ops: 0, hosts: {} };

    // Skip locals
    let fp = fStart;
    const [nld, nl2] = readLEB(fp); fp += nl2;
    for (let i = 0; i < nld; i++) { const [,c] = readLEB(fp); fp += c; fp++; }

    let ops = 0;
    const hosts: Record<number, number> = {};

    while (fp < fEnd) {
      const op = bytes[fp++];
      if (op === 0x0B || op === 0x00 || op === 0x01) continue;

      if (op === 0x10) {
        const [target, t1] = readLEB(fp); fp += t1;
        if (target < importFuncNames.length) {
          // Host call
          const hostName = importFuncNames[target];
          if (hostName && HOST_GAS_BY_NAME[hostName] !== undefined) {
            hosts[target] = (hosts[target] || 0) + 1;
          }
        } else {
          // Local call — recurse
          const sub = countOpcodes(target - importFuncCount);
          ops += sub.ops;
          for (const [k, v] of Object.entries(sub.hosts)) {
            hosts[Number(k)] = (hosts[Number(k)] || 0) + v;
          }
        }
        ops++;
        continue;
      }

      const skip = wasmImmediateSize(op, bytes, fp);
      fp += skip;
      ops++;
    }
    return { ops, hosts };
  };

  // Count sub-calls from call targets
  let subOpcodeGas = 0;
  const allHostCalls: Record<number, number> = { ...hostCallCounts };

  for (const target of callTargets) {
    if (target >= importFuncCount) {
      const sub = countOpcodes(target - importFuncCount);
      subOpcodeGas += sub.ops * NEAR_REGULAR_OP_COST;
      for (const [k, v] of Object.entries(sub.hosts)) {
        allHostCalls[Number(k)] = (allHostCalls[Number(k)] || 0) + v;
      }
    }
  }

  const opcodeGas = opcodeCount * NEAR_REGULAR_OP_COST + subOpcodeGas;
  let hostGas = 0;
  for (const [idx, count] of Object.entries(allHostCalls)) {
    const hostName = importFuncNames[Number(idx)] || '';
    hostGas += (HOST_GAS_BY_NAME[hostName] || 0) * count;
  }

  return {
    opcodes: opcodeCount,
    opcodeGas,
    hostGas,
    totalGas: opcodeGas + hostGas,
    hostCalls: allHostCalls,
  };
}

/** Clear NEAR mock storage */
export function clearNearStorage(): void {
  nearStorage.clear();
  localStorage.removeItem('near_mock_storage');
}

/** Run P2 WASI WASM in the browser with polyfilled imports. */
export async function runWasi(wasmBytes: Uint8Array, stdinData?: Uint8Array): Promise<string> {
  // Check if SharedArrayBuffer is available (requires COOP/COEP headers)
  if (typeof SharedArrayBuffer === 'undefined') {
    return runWasiSync(wasmBytes, stdinData);
  }
  
  // Use Worker + Atomics for synchronous HTTP
  return runWasiWithWorker(wasmBytes, stdinData);
}

/** Fallback: Run without Worker (mock HTTP) */
async function runWasiSync(wasmBytes: Uint8Array, stdinData?: Uint8Array): Promise<string> {
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
    // http_get must be SYNCHRONOUS — WASM imports cannot be async.
    // For browser testing, return mock JSON. Real HTTP requires OutLayer deployment.
    http_get: (urlPtr: number, urlLen: number, respBufPtr: number, respBufLen: number, respLenPtr: number): number => {
      const mem = getMemory();
      if (!mem) return 1;
      const bytes = new Uint8Array(mem.buffer);
      const url = new TextDecoder().decode(bytes.slice(urlPtr, urlPtr + urlLen));
      // Return mock JSON for browser testing — real HTTP happens on OutLayer
      const mockData = JSON.stringify({ mocked: true, url, message: 'Browser test mode — deploy to OutLayer for real HTTP' });
      const dataBytes = new TextEncoder().encode(mockData);
      const toCopy = Math.min(dataBytes.length, respBufLen);
      bytes.set(dataBytes.slice(0, toCopy), respBufPtr);
      new DataView(mem.buffer).setUint32(respLenPtr, toCopy, true);
      return 0;
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

/** Run WASM in Worker with Atomics.wait() for synchronous HTTP */
async function runWasiWithWorker(wasmBytes: Uint8Array, stdinData?: Uint8Array): Promise<string> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./wasm-worker.ts', import.meta.url), { type: 'module' });
    
    // Shared buffer for coordination (64KB for HTTP responses)
    const sharedBuffer = new SharedArrayBuffer(65536);
    const int32 = new Int32Array(sharedBuffer);
    
    let httpPending = false;
    let pendingHttpInfo: { respBufPtr: number; respBufLen: number; respLenPtr: number } | null = null;
    
    worker.onmessage = async (e: MessageEvent) => {
      const { type, data } = e.data as { type: string; data?: any };
      
      switch (type) {
        case 'done':
          worker.terminate();
          resolve(data.stdout);
          break;
          
        case 'error':
          worker.terminate();
          reject(new Error(data.error));
          break;
          
        case 'http_request': {
          // Main thread handles async fetch
          const { url, respBufPtr, respBufLen, respLenPtr } = data;
          httpPending = true;
          pendingHttpInfo = { respBufPtr, respBufLen, respLenPtr };
          
          try {
            const resp = await fetch(url, { headers: { 'User-Agent': 'lisp-rlm-browser' } });
            const text = await resp.text();
            worker.postMessage({ type: 'http_response', data: { response: text } });
          } catch (err) {
            worker.postMessage({ type: 'http_response', data: { response: JSON.stringify({ error: true, message: String(err) }) } });
          }
          break;
        }
        
        case 'storage_get': {
          const { key, buf, bl, lp } = data;
          const val = localStorage.getItem(`lisp-rlm:${key}`) ?? '';
          const vb = new TextEncoder().encode(val);
          // Can't write to worker memory directly — use shared buffer
          const sharedBytes = new Uint8Array(sharedBuffer);
          sharedBytes.set(vb.slice(0, Math.min(vb.length, 65532)), 4);
          int32[0] = vb.length; // Worker reads length from index 0
          Atomics.store(int32, 0, 1);
          Atomics.notify(int32, 0);
          break;
        }
        
        case 'storage_has': {
          const { key } = data;
          const exists = localStorage.getItem(`lisp-rlm:${key}`) !== null;
          int32[1] = exists ? 1 : 0;
          Atomics.store(int32, 0, 1);
          Atomics.notify(int32, 0);
          break;
        }
        
        case 'storage_set': {
          const { key, val } = data;
          localStorage.setItem(`lisp-rlm:${key}`, val);
          break;
        }
        
        case 'storage_delete': {
          const { key } = data;
          localStorage.removeItem(`lisp-rlm:${key}`);
          break;
        }
        
        case 'storage_clear':
          localStorage.clear();
          break;
      }
    };
    
    worker.onerror = (e) => {
      worker.terminate();
      reject(new Error(e.message));
    };
    
    // Initialize worker
    worker.postMessage({ type: 'init', data: { sharedBuffer } });
    
    // Run WASM
    worker.postMessage({ type: 'run', data: { wasmBytes, stdinData } });
  });
}