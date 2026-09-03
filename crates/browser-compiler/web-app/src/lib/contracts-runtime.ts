// Multi-contract sandbox runtime — local promise/receipt execution.
//
// Hosts N compiled contracts (separate wasm instances + accounts), executes
// cross-contract promises LOCALLY with nearcore-style semantics: the
// scheduled function-call runs on the TARGET contract after the caller
// returns; callbacks run on the ORIGINATING contract with the callee's
// return value readable via near.promiseResult(0).
//
// Isolated from the legacy single-contract runNear path on purpose: existing
// examples keep the battle-tested RPC-view pipeline; multi-contract examples
// (sidecars) run here. See TASK-multi-contract-sandbox.md.

import { compile } from './compiler.ts';

export interface ContractSpec {
  account: string;      // e.g. "ft.pg"
  name: string;         // display name
  source: string;       // TS source
}

export interface SidecarSpec extends ContractSpec {}

export interface ReceiptTrace {
  id: number;
  from: string;         // originator account
  to: string;           // target account
  method: string;
  args: string;
  ret: string | null;   // decoded return (null = trap/abort)
  ok: boolean;
  error?: string;
}

interface ContractRT {
  account: string;
  name: string;
  wasm: WebAssembly.Instance;
  memory: WebAssembly.Memory;
  storage: Map<string, Uint8Array>;
  registers: Map<number, Uint8Array>;
  input: Uint8Array;
  returnValue: Uint8Array | null;
  logs: string[];
  promiseResults: Map<number, Uint8Array>; // promise idx → result bytes
  exports: string[];
}

export interface CtxOverride {
  signerAccount?: string;
  blockTimestamp?: bigint;
  blockIndex?: bigint;
}

export interface MultiResult {
  stdout: string;
  logs: string[];
  receipts: ReceiptTrace[];
  returnValue: Uint8Array | null;
  panic: string | null;
  storage: Map<string, string>; // account → pretty storage dump
}

export function clearMultiStorage() {
  try { globalThis.localStorage?.removeItem('pg_multi_storage'); } catch { /* noop */ }
}

const enc = new TextEncoder();
const dec = new TextDecoder();

export async function runMulti(
  main: ContractSpec,
  sidecars: SidecarSpec[],
  opts: { method: string; input?: string },
  ctx: CtxOverride = {},
): Promise<MultiResult> {
  const ctxState = {
    signer: ctx.signerAccount ?? 'browser-user.testnet',
    predecessor: 'browser-user.testnet',
    current: 'unknown.pg',
    ts: ctx.blockTimestamp ?? BigInt(Date.now()) * 1_000_000n,
    idx: 12345678n,
  };

  const contracts = new Map<string, ContractRT>();
  let receiptCounter = 0;
  const traces: ReceiptTrace[] = [];
  const stdoutLines: string[] = [];

  // ── receipts (nearcore-style: drain AFTER the caller returns) ──
  interface FuncAction { method: string; args: string; gas: bigint }
  interface Batch {
    id: number;
    origin: string;             // account that created this batch
    target: string;             // account the batch executes on
    actions: FuncAction[];
    thenBatches: Batch[];       // callbacks chained via promise_batch_then
    parentPromise?: number;     // promise idx this batch is a callback of
  }
  const batches: Batch[] = [];
  const batchByPromise = new Map<number, Batch>();
  let promiseCounter = 0;
  let promiseReturnIdx: number | null = null; // promise_return(cb)

  // active contract whose host state the env closures touch
  let cur: ContractRT;

  const memBytes = (c: ContractRT) => new Uint8Array(c.memory.buffer);
  const memView = (c: ContractRT) => new DataView(c.memory.buffer);

  function writeRegStr(c: ContractRT, rid: number, s: string) {
    c.registers.set(rid, enc.encode(s));
  }

  async function instantiate(spec: ContractSpec): Promise<ContractRT> {
    const res = compile(spec.source, 'p1', 'ts');
    if (!res.success || !res.wasmBytes) throw new Error(`${spec.name}: compile failed — ${res.error ?? 'no output'}`);
    const wasmBytes = res.wasmBytes;
    const rt: ContractRT = {
      account: spec.account, name: spec.name,
      wasm: undefined as unknown as WebAssembly.Instance, memory: undefined as unknown as WebAssembly.Memory,
      storage: new Map(), registers: new Map(),
      input: new Uint8Array(0), returnValue: null, logs: [],
      promiseResults: new Map(), exports: [],
    };
    const env = buildEnv(rt);
    const mod = new WebAssembly.Module(wasmBytes as unknown as BufferSource);
    const inst = (await WebAssembly.instantiate(mod, { env })) as WebAssembly.Instance;
    rt.wasm = inst;
    rt.memory = inst.exports.memory as WebAssembly.Memory;
    rt.exports = Object.keys(inst.exports).filter(k => typeof (inst.exports as never as Record<string, unknown>)[k] === 'function');
    return rt;
  }

  function buildEnv(rt: ContractRT): Record<string, (...a: bigint[]) => unknown> {
    // helpers resolving against the ACTIVE contract (swapped per receipt)
    const b = () => memBytes(cur);
    const takeArgs = (kl: bigint, kp: bigint) => dec.decode(b().slice(Number(kp), Number(kp + kl)));
    return {
      read_register: (rid: bigint, ptr: bigint) => { const d = cur.registers.get(Number(rid)); if (d) b().set(d, Number(ptr)); },
      register_len: (rid: bigint) => { const d = cur.registers.get(Number(rid)); return d ? BigInt(d.length) : 0n; },
      input: (rid: bigint) => cur.registers.set(Number(rid), cur.input.slice()),
      signer_account_id: (rid: bigint) => writeRegStr(cur, Number(rid), ctxState.signer),
      predecessor_account_id: (rid: bigint) => writeRegStr(cur, Number(rid), ctxState.predecessor),
      current_account_id: (rid: bigint) => writeRegStr(cur, Number(rid), ctxState.current),
      block_timestamp: () => ctxState.ts,
      block_index: () => ctxState.idx,
      storage_write: (kl: bigint, kp: bigint, vl: bigint, vp: bigint) => {
        const key = takeArgs(kl, kp);
        cur.storage.set(key, b().slice(Number(vp), Number(vp + vl)));
        return 0n;
      },
      storage_read: (kl: bigint, kp: bigint, rid: bigint) => {
        const v = cur.storage.get(takeArgs(kl, kp));
        if (!v) return 0n;
        cur.registers.set(Number(rid), v);
        return 1n;
      },
      storage_has_key: (kl: bigint, kp: bigint) => (cur.storage.has(takeArgs(kl, kp)) ? 1n : 0n),
      storage_remove: (kl: bigint, kp: bigint) => { return cur.storage.delete(takeArgs(kl, kp)) ? 1n : 0n; },
      value_return: (l: bigint, p: bigint) => { if (cur.returnValue === null) cur.returnValue = b().slice(Number(p), Number(p + l)); },
      panic: () => { throw new Error('NEAR panic'); },
      panic_utf8: (l: bigint, p: bigint) => {
        const m = dec.decode(b().slice(Number(p), Number(p) + Number(l)));
        throw new Error(m);
      },
      log_utf8: (l: bigint, p: bigint) => {
        const m = dec.decode(b().slice(Number(p), Number(p) + Number(l)));
        cur.logs.push(m); stdoutLines.push(`  [${cur.account}] ${m}`);
      },
      log_utf16: () => {},
      // ── promises: LOCAL execution when target account is a local contract ──
      promise_batch_create: (tLen: bigint, tPtr: bigint) => {
        const target = dec.decode(b().slice(Number(tPtr), Number(tPtr) + Number(tLen)));
        const pidx = promiseCounter++;
        const batch: Batch = { id: pidx, origin: cur.account, target, actions: [], thenBatches: [] };
        batches.push(batch);
        batchByPromise.set(pidx, batch);
        return BigInt(pidx);
      },
      promise_batch_then: (p: bigint, tLen: bigint, tPtr: bigint) => {
        const parent = batchByPromise.get(Number(p));
        const target = dec.decode(b().slice(Number(tPtr), Number(tPtr) + Number(tLen)));
        if (!parent) return BigInt(++promiseCounter); // unknown → inert
        const pidx = ++promiseCounter;
        const cb: Batch = { id: pidx, origin: parent.origin, target, actions: [], thenBatches: [], parentPromise: Number(p) };
        parent.thenBatches.push(cb);
        batchByPromise.set(pidx, cb);
        return BigInt(pidx);
      },
      promise_batch_action_function_call: (p: bigint, mLen: bigint, mPtr: bigint, aLen: bigint, aPtr: bigint, _dep: bigint, _gas: bigint) => {
        const batch = batchByPromise.get(Number(p));
        if (!batch) return;
        const method = dec.decode(b().slice(Number(mPtr), Number(mPtr) + Number(mLen)));
        const args = dec.decode(b().slice(Number(aPtr), Number(aPtr) + Number(aLen)));
        batch.actions.push({ method, args, gas: _gas });
      },
      promise_batch_action_transfer: () => {},
      promise_create: () => BigInt(++promiseCounter),   // legacy RPC path unused here
      promise_then: () => BigInt(++promiseCounter),
      promise_and: () => BigInt(++promiseCounter),
      promise_return: (p: bigint) => { promiseReturnIdx = Number(p); },
      promise_results_count: () => BigInt(cur.promiseResults.size),
      promise_result: (ridx: bigint, rid: bigint) => {
        const data = cur.promiseResults.get(Number(ridx));
        if (!data) return 0n;
        cur.registers.set(Number(rid), data);
        return 1n;
      },
      // stubs to keep instantiate alive for contracts that import them
      sha256: (_l: bigint, _p: bigint, out: bigint) => b().set(new Uint8Array(32), Number(out)),
      attached_deposit: (rid: bigint) => writeRegStr(cur, Number(rid), '0'),
      prepaid_gas: () => 300_000_000_000_000n,
      used_gas: () => 1n,
      storage_usage: () => 0n,
      epoch_height: () => 1n,
      signer_account_pk: (rid: bigint) => writeRegStr(cur, Number(rid), 'ed25519:mock'),
      account_balance: (rid: bigint) => writeRegStr(cur, Number(rid), '0'),
      ed25519_verify: () => 1n,
      alt_bn128_g1_multiexp: () => 1n, keccak256: (_l: bigint, _p: bigint, out: bigint) => b().set(new Uint8Array(32), Number(out)),
    };
  }

  // execute one function-call receipt on its target contract
  function execAction(origin: string, target: string, act: FuncAction, resultInto?: { on: ContractRT; idx: number }): { ret: string | null; ok: boolean; err?: string } {
    const tc = contracts.get(target);
    if (!tc) return { ret: null, ok: false, err: `unknown contract ${target}` };
    if (!tc.exports.includes(act.method)) return { ret: null, ok: false, err: `no export ${act.method} on ${target}` };

    const prev = cur;
    const prevPred = ctxState.predecessor;
    cur = tc;
    ctxState.predecessor = origin;            // NEAR: predecessor = calling contract
    tc.input = enc.encode(act.args);
    tc.returnValue = null;
    let ok = true, err: string | undefined;
    try {
      (tc.wasm.exports as never as Record<string, () => void>)[act.method]();
    } catch (e) {
      ok = false;
      err = (e as Error).message;
    }
    const ret = tc.returnValue ? dec.decode(tc.returnValue) : null;
    ctxState.predecessor = prevPred;
    cur = prev;

    const trace: ReceiptTrace = { id: receiptCounter++, from: origin, to: target, method: act.method, args: act.args, ret, ok, error: err };
    traces.push(trace);
    stdoutLines.push(`${ok ? '✓' : '✗'} ${origin} → ${target}.${act.method}(${act.args.slice(0, 60)}) ${ok && ret ? `→ ${ret}` : err ? `ABORT: ${err}` : ''}`);
    if (resultInto && ret !== null) resultInto.on.promiseResults.set(resultInto.idx, enc.encode(ret));
    return { ret, ok, err };
  }

  // drain: execute batches in order, cascading then-callbacks with results
  function drain(): string | null {
    let finalRet: string | null = null;
    while (batches.length > 0) {
      const batch = batches.shift()!;
      // execute actions sequentially
      for (const act of batch.actions) {
        const r = execAction(batch.origin, batch.target, act);
        // chain result into callbacks (promise idx = batch.id for 1:1 callAwait)
        if (batch.thenBatches.length > 0) {
          const cbContract = contracts.get(batch.origin);
          if (cbContract) {
            if (r.ok && r.ret !== null) cbContract.promiseResults.set(0, enc.encode(r.ret));
            else cbContract.promiseResults.delete(0);
          }
        }
      }
      // then-callbacks execute AFTER the parent batch (nearcore receipt order)
      for (const cb of batch.thenBatches) {
        for (const act of cb.actions) {
          const r = execAction(cb.origin, cb.target, act);
          finalRet = r.ok ? (r.ret ?? finalRet) : finalRet;
        }
      }
    }
    return finalRet;
  }

  // ── boot ──
  const mainRt = await instantiate(main);
  contracts.set(main.account, mainRt);
  for (const sc of sidecars) {
    const rt = await instantiate(sc);
    contracts.set(sc.account, rt);
  }
  // restore persisted per-contract storage (survives Run clicks, like the
  // single-contract sandbox's localStorage persistence)
  try {
    const saved = JSON.parse(globalThis.localStorage?.getItem('pg_multi_storage') ?? '{}') as Record<string, [string, string][]>;
    for (const [acct, entries] of Object.entries(saved)) {
      const rt = contracts.get(acct);
      if (!rt) continue;
      for (const [k, b64] of entries) {
        const bin = atob(b64);
        const arr = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
        rt.storage.set(k, arr);
      }
    }
  } catch { /* fresh sandbox */ }
  const persistStorage = () => {
    try {
      const out: Record<string, [string, string][]> = {};
      for (const [acct, rt] of contracts) {
        out[acct] = [...rt.storage.entries()].map(([k, v]) => {
          let s = '';
          for (const b of v) s += String.fromCharCode(b);
          return [k, btoa(s)];
        });
      }
      globalThis.localStorage?.setItem('pg_multi_storage', JSON.stringify(out));
    } catch { /* storage unavailable */ }
  };

  // initial call on main
  cur = mainRt;
  ctxState.current = main.account;
  mainRt.input = enc.encode(opts.input ?? '{}');
  let panic: string | null = null;
  let directRet: Uint8Array | null = null;
  try {
    (mainRt.wasm.exports as never as Record<string, () => void>)[opts.method]();
  } catch (e) {
    panic = (e as Error).message;
  }
  directRet = mainRt.returnValue;

  // receipts drain after the caller returns
  let cascadeRet: string | null = null;
  if (panic === null) cascadeRet = drain();

  const storageDump = new Map<string, string>();
  for (const [acct, rt] of contracts) {
    const lines: string[] = [];
    rt.storage.forEach((v, k) => lines.push(`${k} = ${dec.decode(v)}`));
    storageDump.set(acct, lines.join('\n'));
  }

  persistStorage();
  const retBytes = directRet ?? (cascadeRet !== null ? enc.encode(cascadeRet) : null);
  return {
    stdout: stdoutLines.join('\n'),
    logs: [...mainRt.logs, ...traces.map(t => `${t.from}→${t.to}.${t.method}`)],
    receipts: traces,
    returnValue: retBytes,
    panic,
    storage: storageDump,
  };
}
