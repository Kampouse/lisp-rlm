/**
 * lisp-rlm TS type definitions.
 * Reference: `near-compile file.ts --target near`
 */

// ═══════════════════════════════════════════════════════════════════
// near.* — NEAR host functions (namespace API)
// ═══════════════════════════════════════════════════════════════════

declare const near: {
  // ── Storage ──
  /** Read string value. Returns "" if key missing. */
  storageGet(key: string): string;
  /** Write string value. */
  storageSet(key: string, value: string): void;
  /** Remove key. */
  storageRemove(key: string): void;
  /** Check if key exists. */
  storageHas(key: string): number;
  /** Read raw bytes. */
  storageGetBytes(key: string): string;
  /** Write raw bytes. */
  storageSetBytes(key: string, value: string): void;
  /** Storage used in bytes. */
  storageUsage(): number;

  // ── Context (zero-arg) ──
  /** Caller account ID. */
  predecessorAccountId(): string;
  /** This contract's account ID. */
  currentAccountId(): string;
  /** Current block height (u64). */
  blockHeight(): number;
  /** Current block index (alias for blockHeight). */
  blockIndex(): number;
  /** Current block timestamp (decimal string, nanoseconds). */
  blockTimestamp(): string;
  /** Gas attached to the call. */
  prepaidGas(): number;
  /** Gas used so far. */
  usedGas(): number;
  /** Deposit attached (low 64 bits as number). */
  attachedDeposit(): number;
  /** Deposit attached (high 64 bits). */
  attachedDepositHigh(): number;
  /** Raw input JSON string. */
  input(): string;
  /** Random seed (u64). */
  randomSeed(): number;

  // ── Crypto ──
  /** ed25519 signature verification. Returns 1 if valid. */
  ed25519Verify(message: string, signature: string, public_key: string): number;
  /** Schnorr (BIP-340) signature verification. */
  schnorrVerify(message: string, signature: string, public_key: string): number;
  /** SHA-256 hash. */
  sha256(data: string): string;
  /** Ethereum address recovery. */
  ecrecover(hash: string, sig: string): string;

  // ── Cross-contract calls ──
  /** Fire-and-forward cross-contract call. Result forwarded to caller.
   *  Args: target, method, args_json, gas, deposit */
  call(
    target: string,
    method: string,
    args: string,
    gas: number,
    deposit: number,
  ): void;
  /** Cross-contract call with async callback.
   *  The callback must be a separate exported function.
   *  Returns nil — the callback runs in a new invocation. */
  callAwait(
    target: string,
    method: string,
    args: string,
    gas: number,
    callback: string,
    cbGas: number,
    cbArgs: string,
  ): void;
  /** Read promise result inside a callback. Index 0. */
  promiseResult(index: number): string;
  /** Number of promise results available. */
  promiseResultsCount(): number;
  /** Send NEAR (i64 yocto). */
  transfer(receiver: string, amount: number): void;
  /** Send NEAR (u128 decimal string). */
  transferU128(receiver: string, amount: string): void;
  /** Yield checkpoint: suspends contract, waits for external resume.
   *  Returns data_id (u64). Resume with yieldResume(data_id, payload). */
  yieldCreate(method: string, args: string, gas: number, weight: number): number;
  /** Resume a yielded checkpoint with payload. */
  yieldResume(dataId: number, payload: string): number;

  // ── IO ──
  /** Log a string. */
  log(msg: string): void;
  /** Log a number. */
  logNum(n: number): void;
  /** Abort with message. */
  panic(msg: string): never;
  /** Abort (no message). */
  abort(): never;

  // ── JSON helpers (used internally, available as escape hatches) ──
  /** Get string field from JSON. */
  jsonGetStr(json: string, path: string): string;
  /** Return JSON string to caller (view functions). */
  jsonReturnStr(value: string): never;
};

// ═══════════════════════════════════════════════════════════════════
// Global builtins — string functions
// ═══════════════════════════════════════════════════════════════════

/** String length. */
declare function strLength(s: string): number;
/** String slice (start, end). */
declare function strSlice(s: string, start: number, end: number): string;
/** Concatenate two strings. */
declare function strCat(a: string, b: string): string;
/** Find substring index. */
declare function strIndexOf(s: string, needle: string): number;
/** Convert string to number. */
declare function strToNum(s: string): number;
/** Check if string starts with prefix. */
declare function strStartsWith(s: string, prefix: string): number;
/** Check if string contains substring. */
declare function strContains(s: string, needle: string): number;
/** Check if string ends with suffix. */
declare function strEndsWith(s: string, suffix: string): number;
/** Convert any value to string. */
declare function toStr(value: any): string;
declare function toString(value: any): string;
/** Hex encode. */
declare function hexEncode(s: string): string;
/** Hex decode. */
declare function hexDecode(s: string): string;

// ═══════════════════════════════════════════════════════════════════
// Global builtins — JSON
// ═══════════════════════════════════════════════════════════════════

/** Get value from JSON string at path. */
declare function jsonGet(json: string, path: string): any;

// ═══════════════════════════════════════════════════════════════════
// Global builtins — arrays / lists / HOFs
// ═══════════════════════════════════════════════════════════════════

/** Array length. */
declare function len(arr: any[]): number;
/** Get element at index. */
declare function nth(arr: any[], index: number): any;
/** First element. */
declare function car(arr: any[]): any;
/** Rest of array. */
declare function cdr(arr: any[]): any[];
/** Prepend element. */
declare function cons(elem: any, arr: any[]): any[];
/** Append two arrays. */
declare function append(a: any[], b: any[]): any[];
/** Map function over array. */
declare function map(fn: any, arr: any[]): any[];
/** Filter array by predicate. */
declare function filter(fn: any, arr: any[]): any[];
/** Reduce array to single value. */
declare function reduce(fn: any, init: any, arr: any[]): any;

// ═══════════════════════════════════════════════════════════════════
// Global builtins — u128 arithmetic (string-based)
// ═══════════════════════════════════════════════════════════════════

declare function u128Add(a: string, b: string): string;
declare function u128Sub(a: string, b: string): string;
declare function u128Mul(a: string, b: string): string;
declare function u128Div(a: string, b: string): string;
declare function u128Mod(a: string, b: string): string;
declare function u128Lt(a: string, b: string): number;
declare function u128Gt(a: string, b: string): number;
declare function u128Eq(a: string, b: string): number;

// ═══════════════════════════════════════════════════════════════════
// CamelCase aliases (global function style)
// ═══════════════════════════════════════════════════════════════════

/** Cross-contract call (global alias for near.call). */
declare function nearCall(
  target: string,
  method: string,
  args: string,
  gas: number,
  deposit: number,
): void;
/** Cross-contract call with callback (global alias for near.callAwait). */
declare function nearCallAwait(
  target: string,
  method: string,
  args: string,
  gas: number,
  callback: string,
  cbGas: number,
  cbArgs: string,
): void;
/** Transfer NEAR (global alias for near.transfer). */
declare function nearTransfer(receiver: string, amount: number): void;
/** Transfer NEAR u128 (global alias for near.transferU128). */
declare function nearTransferU128(receiver: string, amount: string): void;
/** Yield checkpoint (global alias for near.yieldCreate). */
declare function yieldCreate(method: string, args: string, gas: number, weight: number): number;
/** Resume yielded checkpoint (global alias for near.yieldResume). */
declare function yieldResume(dataId: number, payload: string): number;
/** Yield checkpoint (near-prefixed alias). */
declare function nearYieldCreate(method: string, args: string, gas: number, weight: number): number;
/** Resume yielded checkpoint (near-prefixed alias). */
declare function nearYieldResume(dataId: number, payload: string): number;

// ═══════════════════════════════════════════════════════════════════
// M2 TS subset — supported syntax
// ═══════════════════════════════════════════════════════════════════
//
// Supported:
//   - export function / function / arrow functions
//   - const / let declarations
// // - if / else / for / while
// // - return (including early return)
// // - = / += / -= assignments
// // - i++ / i-- (update expressions)
// // - String methods: .length, .slice(), .startsWith(), .endsWith(),
// //   .indexOf(), .includes(), .charAt(), .concat(), .toString()
// // - Template literals: `hello ${name}`
// // - Array literals: [1, 2, 3]
// // - Array indexing: arr[i]
// // - Object literals: { key: val } (→ json-obj)
// // - map / filter / reduce
// // - async/await (V1: single `const x = await nearCall(...)` per function)
// //   - async/await (V1: single `const x = await nearCall(...)` per function)
// //   - Destructuring, spread, rest
// //   - class, interface, type, enum
// //   - try/catch, throw
// //   - import/export (single-file only)
// //   - Ternary (use if/else)
// //   - &&, ||, ?? as expressions (use if/else)
// //   - switch, for...of, for...in
// //   - any method calls on non-string/non-array receivers
