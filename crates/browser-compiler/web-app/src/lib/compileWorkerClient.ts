/**
 * Main-side dispatcher for the compile worker (improvement #4).
 *
 * Why: the compiler wasm runs on the MAIN thread today, so a wasm32 stack
 * overflow or infinite loop in a bad contract freezes/kills the whole tab.
 * Routing compile() through a dedicated worker makes that survivable:
 *
 *   - worker crash (onerror)  -> pending requests reject with a
 *     `WORKER_CRASHED:` error; a FRESH worker spawns for the next compile.
 *     No main-thread re-run (the same poison would freeze the tab).
 *   - timeout (60s)           -> worker terminated, pending reject with
 *     `WORKER_TIMEOUT:`; fresh worker spawns next time. No main re-run.
 *   - spawn failure (no worker support / CSP) -> `WORKER_UNSUPPORTED:`
 *     callers may fall back to the main-thread path.
 *
 * Error-message prefixes are the contract with compileWithFallback().
 */
import { annotateErrorLines } from './errorLines.ts';

export interface WorkerCompileRequest {
  source: string;
  target: 'p1' | 'p2' | 'pure';
  lang: 'lisp' | 'ts';
}

export interface WorkerCompileOk {
  type: 'ok';
  id: number;
  wasmBytes: Uint8Array;
  size: number;
  timeMs: number;
  wat: string | null;
}

interface PendingEntry {
  resolve: (r: WorkerCompileOk) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

let worker: Worker | null = null;
let nextId = 1;
const pending = new Map<number, PendingEntry>();
const COMPILE_TIMEOUT_MS = 60_000;

function spawnWorker(): Worker {
  const w = new Worker(new URL('./compile.worker.ts', import.meta.url), {
    type: 'module',
  });
  w.onmessage = (e: MessageEvent) => {
    const msg = e.data;
    const entry = pending.get(msg?.id);
    if (!entry) return;
    pending.delete(msg.id);
    clearTimeout(entry.timer);
    if (msg.type === 'ok') {
      entry.resolve(msg);
    } else {
      // Real compile error from the compiler itself — worker stays alive.
      const raw = msg.error ?? 'compile failed';
      entry.reject(new Error(annotateErrorLines(raw, msg.source ?? '')));
    }
  };
  w.onerror = (e: ErrorEvent) => {
    e.preventDefault();
    const err = new Error(
      `WORKER_CRASHED: ${e.message || 'compile worker died (source too heavy?)'}`,
    );
    for (const [, entry] of pending) {
      clearTimeout(entry.timer);
      entry.reject(err);
    }
    pending.clear();
    worker = null; // fresh worker spawns on next getWorker()
  };
  return w;
}

function getWorker(): Worker {
  if (!worker) worker = spawnWorker();
  return worker;
}

export function compileViaWorker(
  req: WorkerCompileRequest,
): Promise<WorkerCompileOk> {
  return new Promise((resolve, reject) => {
    let w: Worker;
    try {
      w = getWorker();
    } catch (err) {
      reject(
        new Error(
          `WORKER_UNSUPPORTED: ${err instanceof Error ? err.message : String(err)}`,
        ),
      );
      return;
    }
    const id = nextId++;
    const timer = setTimeout(() => {
      const entry = pending.get(id);
      if (entry) {
        pending.delete(id);
        // 60s = hang (frontend infinite loop). Terminate so the next
        // compile gets a clean worker.
        w.terminate();
        worker = null;
        entry.reject(new Error('WORKER_TIMEOUT: compile exceeded 60s'));
      }
    }, COMPILE_TIMEOUT_MS);
    pending.set(id, { resolve, reject, timer });
    try {
      w.postMessage({ type: 'compile', id, ...req });
    } catch (err) {
      clearTimeout(timer);
      pending.delete(id);
      reject(
        new Error(
          `WORKER_UNSUPPORTED: ${err instanceof Error ? err.message : String(err)}`,
        ),
      );
    }
  });
}

/** True while a worker compile is in flight (UI can show a spinner). */
export function hasPendingWorkerCompile(): boolean {
  return pending.size > 0;
}
