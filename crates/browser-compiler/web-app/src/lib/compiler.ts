import init, { compile_p1, compile_p2, compile_pure, disassemble_wasm } from '../../public/wasm/lisp_rlm_browser.js';

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

/** Build a minimal `env` import object with no-op stubs for all NEAR host functions.
 *  compile_fuzz WASM may still import some env functions even in pure mode. */
function buildEnvStubs(): Record<string, Function> {
  // Common host function signatures — all return 0n for i64 returns, void otherwise
  const stub = () => {};
  const stubI64 = () => 0n;
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

/** Run pure WASM in the browser (only works for compile_pure target).
 *  compile_fuzz stores the tagged result at memory offset 64 (TEMP_MEM).
 *  Tagged integers: n*2+1 (odd). Booleans: 0/2. Nil: 0. */
export async function runPure(wasmBytes: Uint8Array): Promise<string> {
  const importObject = { env: buildEnvStubs() };
  const { instance } = await WebAssembly.instantiate(wasmBytes.buffer as ArrayBuffer, importObject) as any;
  const exports = instance.exports as Record<string, WebAssembly.ExportValue>;

  // Try calling the "run" export (compile_fuzz exports last function as "run")
  if (typeof exports.run === 'function') {
    try {
      (exports.run as Function)();

      // Read tagged result from memory offset 64 (TEMP_MEM = 64 bytes = 8 i64 slots)
      // Tag encoding: (value << 3) | tag_type  where TAG_BITS = 3
      // TAG_NUM=0, TAG_BOOL=1, TAG_FNREF=2, TAG_CLOSURE=3, TAG_NIL=4, TAG_STR=5
      const memory = exports.memory as WebAssembly.Memory;
      if (memory) {
        const buf = new DataView(memory.buffer);
        const tagged = buf.getBigInt64(64, true); // little-endian i64 at offset 64
        const tagType = tagged & 0x7n;   // low 3 bits
        const payload = tagged >> 3n;     // upper bits (arithmetic shift)

        switch (tagType) {
          case 0n: return payload.toString();        // TAG_NUM — integer value
          case 1n: return payload === 0n ? 'false' : 'true'; // TAG_BOOL
          case 2n: return `<fn#${payload}>`;          // TAG_FNREF
          case 3n: return `<closure@${payload}>`;     // TAG_CLOSURE
          case 4n: return 'nil';                      // TAG_NIL
          case 5n: {                                  // TAG_STR
            const lo = payload & 0xFFFFFFFFn;
            const hi = (payload >> 32n) & 0xFFFFFFFFn;
            // lo = heap offset, hi = length — read bytes from memory
            const bytes = new Uint8Array(memory.buffer);
            const strBytes = bytes.slice(Number(lo), Number(lo) + Number(hi));
            return `"${new TextDecoder().decode(strBytes)}"`;
          }
          default: return `tagged: ${tagged} (type=${tagType}, payload=${payload})`;
        }
      }
      return 'run() → no memory export';
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return `error: ${msg}`;
    }
  }

  // Try the "_run" export (NEAR default)
  if (typeof exports._run === 'function') {
    return '(NEAR contract — use Deploy to run on-chain)';
  }

  // List available exports
  const fns = Object.entries(exports)
    .filter(([, v]) => typeof v === 'function')
    .map(([name]) => name);

  if (fns.length > 0) {
    return `Available exports: ${fns.join(', ')}`;
  }

  return '(no runnable exports)';
}

function extractExports(wat: string): string[] {
  const exports: string[] = [];
  for (const match of wat.matchAll(/\(export\s+"([^"]+)"\s+\(func/g)) {
    exports.push(match[1]);
  }
  return exports;
}

function formatValue(v: unknown): string {
  if (typeof v === 'bigint') return v.toString();
  if (typeof v === 'number') return v.toString();
  if (typeof v === 'undefined') return 'void';
  return String(v);
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
    const paddedHex = hex.padEnd(47, ' ');
    lines.push(`${addr}  ${paddedHex}  |${ascii}|`);
  }

  if (bytes.length > maxBytes) {
    lines.push(`        ... (${bytes.length - maxBytes} more bytes)`);
  }

  return lines.join('\n');
}
