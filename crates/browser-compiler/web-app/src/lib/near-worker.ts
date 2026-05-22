/**
 * NEAR Worker — runs P1 NEAR contracts with mocked runtime.
 * 
 * Provides browser implementations of NEAR host functions:
 * - storage_set/get/has/remove (backed by Map → localStorage)
 * - block_height, signer_account_id, etc (mock values)
 * - log (stdout capture)
 * - value_return (return value handling)
 */

let memory: WebAssembly.Memory | null = null;
let stdout = '';
let returnValue: bigint | null = null;

// Mock NEAR storage (persisted to localStorage)
const storage = new Map<string, bigint>();
let storageKey = 'near_mock_storage';

// Mock blockchain context
const nearContext = {
  current_account_id: 'browser-contract.testnet',
  signer_account_id: 'browser-user.testnet',
  predecessor_account_id: 'browser-caller.testnet',
  block_index: BigInt(12345678),
  block_timestamp: BigInt(Date.now() * 1_000_000),
  epoch_height: BigInt(42),
  account_balance: BigInt('1000000000000000000000000'), // 1 NEAR
  attached_deposit: BigInt(0),
  prepaid_gas: BigInt(300_000_000_000_000),
  used_gas: BigInt(1_000_000_000_000),
};

function loadStorage() {
  try {
    const saved = localStorage.getItem(storageKey);
    if (saved) {
      const data = JSON.parse(saved);
      for (const [k, v] of Object.entries(data)) {
        storage.set(k, BigInt(v as string));
      }
    }
  } catch {}
}

function saveStorage() {
  try {
    const data: Record<string, string> = {};
    storage.forEach((v, k) => {
      data[k] = v.toString();
    });
    localStorage.setItem(storageKey, JSON.stringify(data));
  } catch {}
}

// Register management for large data
const registers = new Map<number, Uint8Array>();
let nextRegister = 1;

function readRegister(ptr: number, len: number) {
  const bytes = new Uint8Array(memory!.buffer);
  return new TextDecoder().decode(bytes.slice(ptr, ptr + len));
}

function writeRegister(data: Uint8Array): number {
  const reg = nextRegister++;
  registers.set(reg, data);
  return reg;
}

self.onmessage = async (e: MessageEvent) => {
  const { type, data } = e.data;

  switch (type) {
    case 'init': {
      storageKey = data.storageKey ?? 'near_mock_storage';
      loadStorage();
      break;
    }

    case 'run': {
      const { wasmBytes, functionName, args } = data;
      stdout = '';
      returnValue = null;

      try {
        const result = await runNearWasm(wasmBytes, functionName, args);
        self.postMessage({ type: 'done', stdout: result.stdout, returnValue: result.returnValue });
      } catch (err) {
        self.postMessage({ type: 'error', error: String(err) });
      }
      break;
    }

    case 'clear_storage': {
      storage.clear();
      localStorage.removeItem(storageKey);
      break;
    }
  }
};

async function runNearWasm(
  wasmBytes: Uint8Array,
  functionName: string | null,
  args: bigint[]
): Promise<{ stdout: string; returnValue: bigint | null }> {
  memory = new WebAssembly.Memory({ initial: 256, maximum: 4096 });

  const imports: WebAssembly.Imports = {
    env: buildNearEnv(),
  };

  const { instance } = await WebAssembly.instantiate(wasmBytes, imports);
  const exports = instance.exports as Record<string, WebAssembly.ExportValue>;

  try {
    // Call the function or _start
    if (functionName && exports[functionName]) {
      const fn = exports[functionName] as Function;
      const result = fn(...args);
      if (typeof result === 'bigint') {
        returnValue = result;
      }
    } else {
      const startFn = exports._start ?? exports.run;
      if (typeof startFn === 'function') {
        (startFn as Function)();
      }
    }
  } catch (e) {
    // Check if this is a value return (not an error)
    if (e instanceof Error && e.message === 'NEAR_RETURN') {
      // returnValue was set by value_return - this is expected
    } else {
      throw e;
    }
  }

  return { stdout, returnValue };
}

function buildNearEnv(): Record<string, Function> {
  return {
    // ===== Storage =====
    storage_write: (keyPtr: number, keyLen: number, valPtr: number, valLen: number, resultPtr: number): bigint => {
      const bytes = new Uint8Array(memory!.buffer);
      const key = readRegister(keyPtr, keyLen);
      const existed = storage.has(key) ? 1n : 0n;
      // Value is in register 0 (NEAR SDK pattern)
      const valueBytes = registers.get(0) ?? bytes.slice(valPtr, valPtr + valLen);
      const value = BigInt(new TextDecoder().decode(valueBytes));
      storage.set(key, value);
      saveStorage();
      const view = new DataView(memory!.buffer);
      view.setBigUint64(resultPtr, existed, true);
      return existed;
    },

    storage_read: (keyPtr: number, keyLen: number, resultPtr: number): bigint => {
      const key = readRegister(keyPtr, keyLen);
      const value = storage.get(key);
      if (value === undefined) {
        const view = new DataView(memory!.buffer);
        view.setBigUint64(resultPtr, 0n, true);
        return 0n; // ERR_NOT_FOUND
      }
      const bytes = new TextEncoder().encode(value.toString());
      registers.set(0, bytes);
      const view = new DataView(memory!.buffer);
      view.setBigUint64(resultPtr, BigInt(bytes.length), true);
      return 1n; // SUCCESS
    },

    storage_remove: (keyPtr: number, keyLen: number, resultPtr: number): bigint => {
      const key = readRegister(keyPtr, keyLen);
      const existed = storage.has(key);
      storage.delete(key);
      saveStorage();
      const view = new DataView(memory!.buffer);
      view.setBigUint64(resultPtr, existed ? 1n : 0n, true);
      return existed ? 1n : 0n;
    },

    storage_has_key: (keyPtr: number, keyLen: number): bigint => {
      const key = readRegister(keyPtr, keyLen);
      return storage.has(key) ? 1n : 0n;
    },

    // ===== Context =====
    current_account_id: (resultPtr: number) => {
      writeStringToMemory(resultPtr, nearContext.current_account_id);
    },
    
    signer_account_id: (resultPtr: number) => {
      writeStringToMemory(resultPtr, nearContext.signer_account_id);
    },

    predecessor_account_id: (resultPtr: number) => {
      writeStringToMemory(resultPtr, nearContext.predecessor_account_id);
    },

    block_index: (): bigint => nearContext.block_index,
    block_timestamp: (): bigint => nearContext.block_timestamp,
    epoch_height: (): bigint => nearContext.epoch_height,

    account_balance: (resultPtr: number) => {
      writeU128ToMemory(resultPtr, nearContext.account_balance);
    },

    attached_deposit: (resultPtr: number) => {
      writeU128ToMemory(resultPtr, nearContext.attached_deposit);
    },

    prepaid_gas: (): bigint => nearContext.prepaid_gas,
    used_gas: (): bigint => nearContext.used_gas,

    // ===== Input =====
    input: (resultPtr: number) => {
      // Empty input for now (could be passed in args)
      const view = new DataView(memory!.buffer);
      view.setUint32(resultPtr, 0, true); // 0 length
    },

    // ===== Output =====
    value_return: (valueLo: number, valueHi: number) => {
      const view = new DataView(memory!.buffer);
      const lo = view.getBigUint64(valueLo, true);
      const hi = view.getBigUint64(valueHi, true);
      returnValue = lo | (hi << 64n);
      throw new Error('NEAR_RETURN'); // Early exit pattern
    },

    // ===== Logging =====
    log_utf8: (len: number, ptr: number) => {
      const bytes = new Uint8Array(memory!.buffer);
      // NEAR SDK uses register pattern: len is register, ptr is...
      // Actually for log_utf8, it's (len_ptr, len_len) or (register_id, 0)
      if (len === 0) {
        // Log from register (we don't support this pattern yet)
        stdout += '(empty log)\n';
      } else {
        const msg = new TextDecoder().decode(bytes.slice(ptr, ptr + len));
        stdout += msg + '\n';
        console.log('[NEAR]', msg);
      }
    },

    log_utf16: (len: number, ptr: number) => {
      const bytes = new Uint8Array(memory!.buffer);
      const msg = new TextDecoder('utf-16').decode(bytes.slice(ptr, ptr + len * 2));
      stdout += msg + '\n';
      console.log('[NEAR]', msg);
    },

    // ===== Panic =====
    panic: () => {
      throw new Error('NEAR panic');
    },

    panic_utf8: (len: number, ptr: number) => {
      const bytes = new Uint8Array(memory!.buffer);
      const msg = new TextDecoder().decode(bytes.slice(ptr, ptr + len));
      throw new Error(`NEAR panic: ${msg}`);
    },

    abort: (msgPtr: number, msgLen: number, filePtr: number, fileLen: number, line: number, col: number) => {
      const bytes = new Uint8Array(memory!.buffer);
      const msg = new TextDecoder().decode(bytes.slice(msgPtr, msgPtr + msgLen));
      const file = new TextDecoder().decode(bytes.slice(filePtr, filePtr + fileLen));
      throw new Error(`Abort at ${file}:${line}:${col}: ${msg}`);
    },

    // ===== Registers =====
    read_register: (registerId: number, ptr: number) => {
      const data = registers.get(registerId);
      if (data) {
        new Uint8Array(memory!.buffer).set(data, ptr);
      }
    },

    register_len: (registerId: number): bigint => {
      const data = registers.get(registerId);
      return data ? BigInt(data.length) : 0n;
    },

    write_register: (registerId: number, len: number, ptr: number) => {
      const bytes = new Uint8Array(memory!.buffer);
      registers.set(registerId, bytes.slice(ptr, ptr + len));
    },

    // ===== Crypto =====
    sha256: (dataPtr: number, dataLen: number, resultPtr: number) => {
      // Stub - would need crypto.subtle in async context
      const zeros = new Uint8Array(32);
      new Uint8Array(memory!.buffer).set(zeros, resultPtr);
    },

    random_seed: (resultPtr: number) => {
      const seed = new Uint8Array(32);
      crypto.getRandomValues(seed);
      new Uint8Array(memory!.buffer).set(seed, resultPtr);
    },

    // ===== Stubs =====
    ...buildStubs(),
  };
}

function buildStubs(): Record<string, Function> {
  const stub = () => {};
  const stubI64 = (): bigint => 0n;
  return {
    // Promise functions (cross-contract calls - not mocked)
    promise_create: stubI64,
    promise_then: stubI64,
    promise_and: stubI64,
    promise_results_count: stubI64,
    promise_result: stub,
    promise_return: stub,
    promise_batch_create: stubI64,
    promise_batch_then: stubI64,
    promise_batch_action_create_account: stub,
    promise_batch_action_deploy_contract: stub,
    promise_batch_action_function_call: stub,
    promise_batch_action_transfer: stub,
    promise_batch_action_stake: stub,
    promise_batch_action_add_key_with_full_access: stub,
    promise_batch_action_add_key_with_function_call: stub,
    promise_batch_action_delete_key: stub,
    promise_batch_action_delete_account: stub,

    // ED25519 verify (not mocked)
    ed25519_verify: (): bigint => 0n,
    // P-256 verify (not mocked)
    p256_verify: (): bigint => 0n,
  };
}

function writeStringToMemory(ptr: number, str: string) {
  const bytes = new TextEncoder().encode(str);
  new Uint8Array(memory!.buffer).set(bytes, ptr);
  const view = new DataView(memory!.buffer);
  view.setUint32(ptr - 4, bytes.length, true); // Length prefix
}

function writeU128ToMemory(ptr: number, value: bigint) {
  const view = new DataView(memory!.buffer);
  view.setBigUint64(ptr, value & ((1n << 64n) - 1n), true); // lo
  view.setBigUint64(ptr + 8, value >> 64n, true); // hi
}

export type NearWorkerMessage =
  | { type: 'init'; data: { storageKey?: string } }
  | { type: 'run'; data: { wasmBytes: Uint8Array; functionName?: string; args?: bigint[] } }
  | { type: 'clear_storage' };