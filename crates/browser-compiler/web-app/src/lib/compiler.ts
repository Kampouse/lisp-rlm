import init, { compile_p1, compile_p2 } from '../../public/wasm/lisp_rlm_browser.js';

let initialized = false;
let initPromise: Promise<void> | null = null;

export type CompileTarget = 'p1' | 'p2';

export interface CompileResult {
  success: boolean;
  wasmBytes: Uint8Array | null;
  size: number;
  timeMs: number;
  error: string | null;
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
    const compileFn = target === 'p1' ? compile_p1 : compile_p2;
    const wasmBytes = compileFn(source);
    const timeMs = performance.now() - start;

    return {
      success: true,
      wasmBytes,
      size: wasmBytes.length,
      timeMs,
      error: null,
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
    };
  }
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
