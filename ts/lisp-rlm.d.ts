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

// ── arrays (lisp TAG_ARRAY values; `arr[i]`, `arr.length`, `arr.push`,// `for (const x of arr)` all lower to vec-nth/vec-length/vec-push/while) ──
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

// ── M2 objects: JSON-string values ──────────────────────────────────────
// `{ k: v }` literals fold into json-set chains and are plain JSON text:
// storage/returns/interop need no conversion. Reads: `o.key` ("" when
// absent), nested `o.a.b` lowers to one dot-path call. Numeric reads need strToNum;
// rebuild via jsonSet with an ENCODED value (jsonQuote(s) for strings,
// toStr(n) for numbers — object literals self-encode).
declare type LispObj = string;
/** integer aliases — the compiler treats these as number */
declare type i32 = number;
declare type i64 = number;
declare type u128 = number;
/** JSON-escape a string and wrap it in quotes → encoded VALUE for jsonSet. */
declare function jsonQuote(s: string): string;
/** Set/replace a top-level key → NEW object (immutable; rebind: o = jsonSet(o, k, v)).
 *  Value: pre-encoded (jsonQuote(s)/object literal/jsonSet(...)), or a raw
 *  bigint literal — the frontend auto-encodes those. */
declare function jsonSet(obj: LispObj, key: string, value: any): any;

declare function strCat(...parts: string[]): string;
declare function strLength(s: string): number;
/** alias of strLength (both spellings lower to str-length) */
declare function strLen(s: string): number;
declare function strSlice(s: string, start: number, end: number): string;
declare function strIndexOf(haystack: string, needle: string): number;
declare function strToNum(s: string): number;
declare function toStr(n: any): string;
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

// Free-function u128 spellings (u128Add → u128/add etc). Args `any`:
// they coerce both NUM (bigint literal / param) and STR decimal strings.
declare function u128Add(a: any, b: any): string;
declare function u128Sub(a: any, b: any): string;
declare function u128Mul(a: any, b: any): string;
declare function u128Div(a: any, b: any): string;
declare function u128Mod(a: any, b: any): string;
declare function u128Lt(a: any, b: any): number;
declare function u128Gt(a: any, b: any): number;
declare function u128Eq(a: any, b: any): number;
declare function u128IsZero(s: string): number;

// ── the `near` namespace (member passthrough, camelCase auto-snakifies) ─

declare const near: {
  // storage (string → string)
  /** Value: pre-encoded string, or raw lattice value (num/bigint
   *  arithmetic results auto-encode at the storage boundary). */
  storageSet(key: string, value: any): void;
  /** Returns "" if missing. Typed `any`: records read via dynamic
   *  field access (p = pool(); p.ra) — `string` would fight the
   *  runtime model in the editor. */
  storageGet(key: string): any;
  storageHas(key: string): boolean;
  storageHasKey(key: string): boolean;
  storageRemove(key: string): void;
  storageUsage(): number;
  /**
   * Start a storage iterator over all keys with the given prefix.
   * Returns an iterator id (number) — pass to near.iterNext().
   * Exhausts in lexicographic key order.
   */
  iterPrefix(prefix: string): number;
  /**
   * Advance a storage iterator. Returns the next key, or null when
   * exhausted (pair with `??` / check for null).
   */
  iterNext(iterId: number): string | null;

  // args / returns
  /**
   * Read a string arg from the transaction input JSON.
   * Missing key → null (nil at runtime, 2026-08-31 semantics) — pair
   * with `??`:
   *   let g = near.jsonGetStr("g") ?? "default";
   * Bare use on a miss yields nil: strLength sees 0, but str-concat
   * renders "nil" — guard explicitly.
   */
  jsonGetStr(key: string): string | null;
  /** {"k": ["a","b"]} → LispArr<string>; max 64 elements, nil if missing */
  jsonArr(key: string): LispArr<string>;
  /**
   * Read a numeric arg from the transaction input JSON.
   * Missing key → null — pair with `??`:
   *   let n = near.jsonGetInt("n") ?? 0;
   */
  jsonGetInt(key: string): number | null;
  jsonReturnStr(v: string): void;
  jsonReturnInt(v: number): void;

  // env
  predecessorAccountId(): string;
  currentAccountId(): string;
  signerAccountId(): string;
  blockIndex(): number;
  /** u128-scale ns since epoch — crosses as NUM (lattice); typed any. */
  blockTimestamp(): any;

  // money (u128 scale → decimal strings)
  attachedDeposit(): string;
  attachedDepositU128(): string;
  /** High 64 bits of the attached deposit as i64 (raw-ABI pairing with attachedDeposit). */
  attachedDepositHigh(): number;
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
  // ── cross-contract (async promise machinery) ──
  // callAwait: schedule an async call on `target`, then invoke `callback`
  // (an exported fn on THIS contract) with the callee's result readable
  // via near.promiseResult(0). Deposit fixed at 0 — use raw batches for payable.
  callAwait(target: string, method: string, argsJson: string, gas: number,
            callback: string, cbGas: number, cbArgsJson: string): void;
  // inside a callback: read the callee's return ("0" = first promise result).
  // Returns the raw value or NIL on failure — branch on it, fail closed.
  promiseResult(idx: number): string;

  // ── async/await (V1) ──
  // `export async function` with `const x = await near.call(...)` as the
  // FIRST statement compiles to entry + <name>__resume continuation:
  // params saved to storage, result bound in the continuation. Zero deposit.
  call(target: string, method: string, argsJson: string, gas: number, deposit: number): void;

  // ── promise yield (NEAR resumable calls) ──
  // yieldCreate: schedule SELF.<method>(args) and yield execution — gas
  // reserves for the resume; weight is the yield weight. Returns yield idx.
  yieldCreate(method: string, argsJson: string, gas: number, weight: number): number;
  // yieldResume: resume a yielded promise — (dataId, payload).
  yieldResume(dataId: string, payload: string): number;

  // ── crypto / hashing (host functions; all compile-verified) ──
  /** SHA-256 of a byte string → hex digest (64 hex chars). */
  sha256(msg: string): string;
  /** kebab-alias spelling (same op). */
  sha256Hash(msg: string): string;
  keccak256(msg: string): string;
  keccak512(msg: string): string;
  ripemd160(msg: string): string;
  /** 32 bytes of validator randomness for the current block. */
  randomSeed(): string;
  /** Ed25519: (signature, message, public_key) → 1/0. */
  ed25519Verify(sig: string, msg: string, pk: string): number;
  /** secp256r1: (sig 64B r||s, msg digest, pk 33B compressed) → 1/0.
   *  Requires NEAR protocol 85+. */
  p256Verify(sig: string, msg: string, pk: string): number;
  /** Ethereum-style: (msgHash, sig, v, malleabilityFlag 0/1)
   *  → recovered address hex, or "" on failure. */
  ecrecover(msgHash: string, sig: string, v: number, malleability: number): string;
  // BLS12-381 + BN254 precompiles (hex-encoded point buffers) — advanced use.
  altBn128G1Sum(buf: string): string;
  altBn128G1Multiexp(pairs: string): string;
  altBn128PairingCheck(buf: string): number;
  bls12381P1Sum(buf: string): string;
  bls12381P2Sum(buf: string): string;
  bls12381G1Multiexp(pairs: string): string;
  bls12381G2Multiexp(pairs: string): string;
  // EIP-2537 remainder (engine hosts 59-67; added for #16 BLS msig):
  // pairing input = concat of (G1 48B || G2 96B) pairs, ≥1 pair, 384B each
  bls12381PairingCheck(pairs: string): number;
  bls12381MapFpToG1(fp: string): string;
  bls12381MapFp2ToG2(fp2: string): string;
  bls12381P1Decompress(g1: string): string;
  bls12381P2Decompress(g2: string): string;

  // ── context / gas ──
  /** Full signer public key (hex) — pairs with ed25519Verify. */
  signerAccountPk(): string;
  prepaidGas(): number;
  usedGas(): number;
  /** Raw transaction input JSON (the full args object as a string). */
  input(): string;
  /** Abort execution with a panic message (state rolls back). */
  panic(msg: string): void;

  // ── raw promises (lower-level than callAwait) ──
  /** All three take deposit as i64 (use 0) BEFORE gas. Return promise idx. */
  promiseCreate(target: string, method: string, argsJson: string, deposit: number, gas: number): number;
  promiseThen(p: number, target: string, method: string, argsJson: string, deposit: number, gas: number): number;
  promiseAnd(p1: number, p2: number, p3?: number): number;

  // ── promise batches (multi-action promises; strings, not raw ABI) ──
  promiseBatchCreate(target: string): number;
  promiseBatchThen(p: number, target: string): number;
  promiseBatchActionTransfer(p: number, yoctoAmount: string): void;
  /** Note arg order: deposit (string) BEFORE gas. */
  promiseBatchActionFunctionCall(p: number, method: string, argsJson: string, yoctoDeposit: string, gas: number): void;
  promiseBatchActionCreateAccount(p: number): void;
  /** Return a promise as this call's outcome (async return pattern). */
  promiseReturn(p: number): void;
  /** Number of promise results readable in this callback. */
  promiseResultsCount(): number;
  /** Whether promise result idx succeeded (1/0) — callbacks only. */
  promiseSucceeded(idx: number): number;
  // Raw-ABI forms (ptr/len pairs, not strings) also exist for stake,
  // addKeyWithFullAccess, addKeyWithFunctionCall, deleteKey, deleteAccount,
  // deployContract — awkward from TS; reach for them only if you must.
};

// ── JS std shims (2026-08-30) ─────────────────────────────────────────
// console.log → near/log (args space-joined, auto to-string'd).
// Math.abs/max/min → abs/max/min (variadic, integer math).
// JSON.stringify(scalar) → json-quote; JSON.parse: NOT NEEDED — tx args
// arrive parsed; use typed params / near.jsonGet.
// (console/Math/JSON value types come from lib — not redeclared here.)
interface JSON {
  /** JSON array text via map(json-quote). */
  stringifyArr(arr: LispArr<string | number>): string;
}

// ── legacy snake_case builtins (pass through to lisp names verbatim) ──
declare function near_storage_get(key: string): any;
declare function near_storage_set(key: string, value: string): void;
declare function near_predecessor_account_id(): string;

// ── storage.* namespace (aliases → near/storage_*) ──────────────────────
declare const storage: {
  get(key: string): any;
  read(key: string): any;
  set(key: string, value: any): void;
  write(key: string, value: any): void;
  del(key: string): void;
  remove(key: string): void;
  has(key: string): boolean;
  hasKey(key: string): boolean;
};
