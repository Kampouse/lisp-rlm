/**
 * lisp-rlm TypeScript dialect — ambient surface (LSP/editor contract).
 *
 * Mirrors src/ts_frontend.rs (map_builtin_call, map_member_fn) and the
 * near/* set dispatched by src/wasm_emit/lambda.rs. KEEP IN SYNC — when a
 * builtin is added/renamed there, update this file in the same commit.
 *
 * Usage (local editors): add to the top of your contract file
 *   /// <reference path="../../ts/lisp-rlm.d.ts" />
 * or include this file in tsconfig "files". The browser IDE injects it
 * into Monaco's TS worker automatically (App.svelte → addExtraLib).
 *
 * Numbers: JS `number` (f64) in annotations, but the dialect's arithmetic
 * is integer; u128-scale values cross as decimal strings (strToNum/toStr).
 * Booleans lower to 0/1 ints.
 */

// ── free function builtins (camelCase → snake_case lisp builtins) ──────

// ── arrays (lisp TAG_ARRAY values; `arr[i]`, `arr.length`, `arr.push`,
// `for (const x of arr)` all lower to vec-nth/vec-length/vec-push/while) ──
declare interface LispArr<T> {
  readonly length: number;
  [index: number]: T;
  push(v: T): void;
  join(separator: string): string;
  // 2026-08-30: arrow callbacks — expression-bodied or single-return
  // blocks (M1). Lower to (map f xs) / (filter f xs) / (reduce f init xs).
  // Same emitters as lisp source → same ~115K-element runtime ceiling.
  map<U>(f: (x: T) => U): LispArr<U>;
  filter(f: (x: T) => boolean): LispArr<T>;
  reduce<U>(f: (acc: U, x: T) => U, init: U): U;
}
declare function strSplit(s: string, delimiter: string): LispArr<string>;
declare function strJoin(separator: string, parts: LispArr<string>): string;

declare function strCat(...parts: string[]): string;
declare function strLength(s: string): number;
declare function strSlice(s: string, start: number, end: number): string;
declare function strIndexOf(haystack: string, needle: string): number;
declare function strToNum(s: string): number;
declare function toStr(n: number): string;
declare const toString: typeof toStr; // alias — shadows nothing at call sites
declare function jsonGet(key: string, json: string): string;
declare function hexDecode(hex: string): string;
declare function sha256Hash(msg: string): string;
// NOTE: predicate builtins return 0/1 ints (dialect semantics), not
// booleans — `ok === 1` comparisons are idiomatic and must typecheck.
declare function schnorrVerify(
  pubkeyHex: string,
  sigHex: string,
  msgHashHex: string,
): number;

// ── u128 as decimal strings (namespace passthrough → u128/*) ───────────
declare const u128: {
  add(a: number | string, b: number | string): string;
  sub(a: number | string, b: number | string): string;
  mul(a: number | string, b: number | string): string;
  div(a: number | string, b: number | string): string;
  mod(a: number | string, b: number | string): string;
  lt(a: number | string, b: number | string): number;
  gt(a: number | string, b: number | string): number;
  eq(a: number | string, b: number | string): number;
  fromI64(n: number): string;
  toI64(s: string): number;
  isZero(s: string): number;
};

// ── the `near` namespace (member passthrough, camelCase auto-snakifies) ─

declare const near: {
  // storage (string → string)
  storageSet(key: string, value: string): void;
  storageGet(key: string): string | null;
  storageHas(key: string): boolean;
  storageRemove(key: string): void;
  storageUsage(): number;

  // args / returns
  jsonGetStr(key: string): string | null;
  /** {"k": ["a","b"]} → LispArr<string>; max 64 elements, nil if missing */
  jsonArr(key: string): LispArr<string>;
  jsonGetInt(key: string): number | null;
  jsonReturnStr(v: string): void;
  jsonReturnInt(v: number): void;

  // env
  predecessorAccountId(): string;
  currentAccountId(): string;
  signerAccountId(): string;
  blockIndex(): number;
  blockTimestamp(): number;

  // money (u128 scale → decimal strings)
  attachedDeposit(): string;
  attachedDepositU128(): string;
  accountBalance(): string;
  // compile-time u128 constant as (lo64, hi64) split — see wasm_emit
  // deposit check: writes attached_deposit to TEMP_MEM, compares u128
  depositGte(lo64: number, hi64: number): number;
  transfer(toAccountId: string, yoctoAmount: string): void;
  transferU128(toAccountId: string, amount: string): void;
  storeU128(key: string, value: string): void;
  loadU128(key: string): string;

  // misc
  log(s: string): void;
  logNum(n: number): void;
  abort(msg: string): void;
  // long-tail (promise batches, validators, ecrecover, random_seed, …)
  // exists at the lisp level but is not TS-declared yet — add typed
  // entries here in the same commit you first use one from TS.
};

// ── JS std shims (2026-08-30) ─────────────────────────────────────────
// console.log → near/log (args space-joined, auto to-string'd)
declare const console: { log(...parts: (string | number | boolean)[]): void };
// Math.abs/max/min → abs/max/min (variadic, integer math)
declare const Math: {
  abs(x: number): number;
  max(...xs: number[]): number;
  min(...xs: number[]): number;
};
// JSON.stringify(scalar) → json-quote (str → "…" with escapes, num → decimal)
// JSON.stringifyArr(arr) → JSON array text via map(json-quote)
// JSON.parse: NOT NEEDED — tx args arrive parsed; use typed params / near.jsonGet
declare const JSON: {
  stringify(v: string | number | boolean): string;
  stringifyArr(arr: LispArr<string | number>): string;
};
