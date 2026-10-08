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
// `o.x = v;` (statement) rebinds: o = jsonSet(o, "x", encoded v) — single level.
declare type LispObj = string;

// NOTE: `Number(s)` also lowers to str->num (map_builtin_call) but is NOT
// redeclared here — it would collide with the JS global (TS 2300).
declare function parseInt(s: string): number;
declare function parseFloat(s: string): number;

// ── JSON API v3 (2026-09-15) — the JS-like layer ────────────────────────
// READING ARGS, the 10-line story:
//
//   // 1. The handle (zero copies — reads rewrite to the shared input
//   //    scanner + per-tx input cache):
//   const o = near.input();
//   const name = o.name ?? "anon";      // string, fallback on missing
//   const amt  = o.amount ?? 0;         // NUMBER — typed read, no strToNum
//   const deep = o.user.name;           // nested ("" on miss — guard it)
//
//   // 2. Typed destructuring (ONE single-pass scan for all keys):
//   const { who, count } = near.args<{ who: string, count: number }>();
//
//   // 3. Legacy getters (also nil-on-miss, pair with ??):
//   const k = near.jsonGetStr("k") ?? "";
//   const n = near.jsonGetInt("n") ?? 0;
//
// Handles are nil-ON-MISS (?? fires exactly when JS ?? would). Legacy
// `o.key` on PLAIN strings keeps the "" contract (back-compat).
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
/** Single-pass multi-key extraction from the TX INPUT (2026-09-14):
 *  jsonExtract("a", "b", "n") → ["val", "", "7"] — raw span strings
 *  (strings unquoted, numbers as text, objects/arrays as full JSON text,
 *  missing keys → ""). One scan of the input for ALL keys — cheaper gas
 *  than N jsonGetStr/Int calls on arg-heavy entrypoints. Max 8 keys. */
declare function jsonExtract(...keys: string[]): LispArr<string>;
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
declare function u128MulDiv(a: any, b: any, d: any): string;
declare function u128Div(a: any, b: any): string;
declare function u128Mod(a: any, b: any): string;
// comparisons lower to u128/lt|gt|eq : (str,str) → bool — use directly in
// if(...); do NOT wrap in toStr() (checker rejects str ≠ bool).
declare function u128Lt(a: any, b: any): boolean;
declare function u128Gt(a: any, b: any): boolean;
declare function u128Eq(a: any, b: any): boolean;
declare function u128IsZero(s: string): boolean;

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

  // db namespace — strings-at-rest key/value store (near.db.*)
  // Methods live in the `db` block below; this entry only silences the
  // typo-warning for the namespace itself.
  db: {
    /** Stored value, or null when the key is missing. Unwrap with
     *  `?? "default"`. u128 amounts are decimal STRINGS. */
    key(key: string): string | null;
    /** Store a value EXACTLY as given (strings at rest — no magic
     *  encoding). Overwrites any existing value. */
    put(key: string, value: string): void;
    /** true when the key exists (even if its value is ""). */
    has(key: string): boolean;
    /** Remove a key. Absent key deletes nothing (no error). */
    del(key: string): void;
    /** All existing keys under a prefix, lexicographic order. Rides the
     *  engine iter builtins (near-mock / RLM lab). The PROTOCOL removed
     *  raw trie enumeration (storage_iter_* answers Deprecated on
     *  mainnet) — production code keeps its own key index (near-sdk
     *  UnorderedMap pattern) instead of relying on keys(). */
    keys(prefix: string): string[];
  };

  // events
  /** Emit a contract event: NEAR log line `{"event":name,"data":{…}}`
   *  (standard events.json shape). Payload must be an object literal. */
  event(name: string, data: object): void;

  // args / returns
  /**
   * Read a string arg from the transaction input JSON.
   * Missing key → null (nil at runtime, 2026-08-31 semantics) — pair
   * with `??`:
   *   let g = near.jsonGetStr("g") ?? "default";
   * Bare use on a miss yields nil: strLength sees 0, but str-concat
   * renders it "nil" — guard explicitly.
   * Object/array values return the FULL balanced span as raw JSON text
   * (2026-09-14): `{"o": {"i": 1}}` → `{"i": 1}` (was just "{").
   * Chain with the 2-arg form or dot-path jsonGet for nested reads.
   */
  jsonGetStr(key: string): string | null;
  /** 2-arg form (2026-09-14): scan the GIVEN JSON string (not the tx
   *  input) — same behavior as jsonGet(key, json), dot-paths supported.
   *  (Previously compiled but silently ignored the second arg.) */
  jsonGetStr(key: string, json: string): string | null;
  /** JSON API v3 (2026-09-15): the tx-input HANDLE — `const o =
   *  near.input()` then `o.prop` / `o.prop ?? fallback`. Property reads
   *  rewrite at compile time to the shared input scanner + per-tx input
   *  cache (zero copies). nil-ON-MISS: `o.k ?? fb` fires exactly when JS
   *  ?? would; a NUMBER fallback selects the typed INT getter (no
   *  strToNum ceremony). Nested `o.a.b` works ("" contract — guard it);
   *  ?? on nested paths is rejected until the str-nil buffer op lands.
   *  The handle itself is not a value (binds a dead nil — never read it
   *  bare). */
  input(): LispObj;
  /** JSON API v3 (2026-09-15): typed single-pass arg binding —
   *  `const {a, n} = near.args<{a: string, n: number}>()`. ONE
   *  jsonExtract scan for all keys; number-typed fields arrive parsed.
   *  Missing fields: str → "", num → 0. Max 8 fields. */
  args<T>(): T;
  /** {"k": ["a", 12, {"n":1}]} → LispArr of raw span strings — strings
   * unquoted, numbers as text, nested objects/arrays as full JSON text
   * (2026-09-14: nested elements + max raised 64 → 512). nil if missing */
  jsonArr(key: string): LispArr<string>;
  /**
   * Read a numeric arg from the transaction input JSON.
   * Missing key → null — pair with `??`:
   *   let n = near.jsonGetInt("n") ?? 0;
   * Found-but-non-numeric ("n": "abc", true, {…}) → null too (2026-09-14):
   * a silent 0 was indistinguishable from a real zero. "12x" → 12 (prefix).
   */
  jsonGetInt(key: string): number | null;
  /** 2-arg form (2026-09-17): scan the GIVEN JSON string (not the tx
   *  input) — mirrors jsonGetStr(key, json). Dot-paths supported.
   *  Miss → null (?? fires). Found-but-non-numeric → null too.
   *  (Previously compiled but silently ignored the second arg — the
   *  literal/dynamic key was looked up in the tx input instead.) */
  jsonGetInt(key: string, json: string): number | null;
  jsonReturnStr(v: string): void;
  jsonReturnInt(v: number): void;
  /** Free-function parity as near.* members — all of these lower to the
   *  same lisp builtins as their bare-name forms. */
  jsonQuote(s: string): string;
  jsonSet(obj: LispObj, key: string, value: any): any;
  jsonGet(key: string, json: string): string;
  jsonGetArr(key: string): LispArr<string>;
  jsonExtract(...keys: string[]): LispArr<string>;
  hexDecode(hex: string): string;
  hexEncode(bytes: string): string;
  keccak256Hash(msg: string): string;

  /** RAW value_return (lisp near/return_str parity): bytes go to the
   *  caller exactly as-is — no {"result": ...} wrap. Needed when a
   *  consumer JSON.parse's the bytes directly (e.g. an outbox array).
   *  NOTE on return conventions (2026-09-30, verified against the wasm
   *  export wrapper + lending battery):
   *    - get_* exports: plain `return v` → json_return_str → {"result": v}
   *    - non-get_ exports: plain `return v` → RAW value_return of v
   *      (works for change methods too — the wrapper always value_returns
   *      the non-nil tail; nil tail = empty bytes)
   *    - near.returnStr(v): explicit raw return from any fn
   *  Use whichever reads best; near.returnStr is the unambiguous form. */
  returnStr(v: string): void;

  // env
  predecessorAccountId(): string;
  currentAccountId(): string;
  signerAccountId(): string;
  blockIndex(): number;
  /** ns since epoch as a DECIMAL STRING (2026-09-30: ns ~1.8e18 exceed
   *  JS safe integers — arithmetic on a `number` silently loses
   *  precision). Slice chars for seconds/nonce the way contracts do.
   *  For arithmetic use blockTimestampNum(). */
  blockTimestamp(): string;
  /** Raw numeric ns (lattice Num) — prefer blockTimestamp() unless you
   *  actually need arithmetic on the value. */
  blockTimestampNum(): number;
  blockHeight(): number;

  // money (u128 scale → decimal strings)
  attachedDeposit(): string;
  attachedDepositU128(): string;
  /** High 64 bits of the attached deposit as i64 (raw-ABI pairing with attachedDeposit). */
  attachedDepositHigh(): number;
  /** Legacy low-64 form (pairs with attachedDepositHigh; prefer attachedDeposit). */
  attachedDepositLow(): number;
  accountBalance(): string;
  // compile-time u128 constant as (lo64, hi64) split — see wasm_emit
  // deposit check: writes attached_deposit to TEMP_MEM, compares u128.
  // Returns a REAL bool (TAG_BOOL) — use `!depositGte(...)` for gates;
  // `== 0` never fires (tag mismatch → false).
  /** Preferred form: ONE bigint literal — the frontend splits u128 →
   *  (lo64, hi64) at compile time: `near.depositGte(12000000000000000000000n)`
   *  Nobody should hand-split a u128. */
  depositGte(yocto: bigint): boolean;
  /** Legacy two-number form (lo64, hi64). */
  depositGte(lo64: number, hi64: number): boolean;
  transfer(toAccountId: string, yoctoAmount: string): void;
  transferU128(toAccountId: string, amount: string): void;
  storeU128(key: string, value: string): void;
  loadU128(key: string): string;
  /** Read a u128 storage value → decimal string ("" if missing). */
  readU128(key: string): string;

  // misc
  log(s: string): void;
  logNum(n: number): void;
  abort(msg: string): void;
  /** schnorr/EC precompile parity as near.* members — same lowering as
   *  the bare schnorrVerify(...) free function. */
  schnorrVerify(pubkeyHex: string, sigHex: string, msgHashHex: string): number;
  schnorrSign(skHex: string, msgHashHex: string, auxHex: string): string;
  schnorrSignPk(skHex: string, msgHashHex: string, auxHex: string): string;
  schnorrPubkey(skHex: string): string;
  schnorrPubkey33(skHex: string): string;
  /** Verifiable-function randomness (NEP-364 style VRF). */
  vrfGenerate(inputHex: string): string;
  // ── cross-contract (async promise machinery) ──
  // callAwait: schedule an async call on `target`, then invoke `callback`
  // (an exported fn on THIS contract) with the callee's result readable
  // via near.promiseResult(0). Deposit fixed at 0 — use raw batches for payable.
  callAwait(target: string, method: string, argsJson: string, gas: number,
            callback: string, cbGas: number, cbArgsJson: string): void;
  // inside a callback: read the callee's return ("0" = first promise result).
  // Returns the payload string, "" on failure (fail-closed; branch on strLength < 1) — branch on it, fail closed.
  promiseResult(idx: number): string;

  // ── async/await (v2, 2026-10-08) ──
  // `export async function` + `const x = await near.call(...)`: compiles to
  // entry + <name>__resume continuation (params saved to storage, result
  // bound in the continuation). v2: awaits ANYWHERE, MULTIPLE awaits
  // (resume names get an index: <name>__resume0, …), deposits flow through
  // (payable awaits are legal — the deposit argument is carried by the
  // promise DAG), and pre-await statements run in the entry.
  call(target: string, method: string, argsJson: string, gas: number, deposit: number): void;
  // Parallel fanout: `const [a, b] = await near.all([near.call(...), near.call(...)])`
  // compiles to promise_create ×N → promise_and → ONE <name>__resume reading
  // promise_result(0..n) in array order. Array literal of near.call(...) only.
  all(calls: void[]): void;

  // ── promise yield (NEAR resumable calls) ──
  // yieldCreate: schedule SELF.<method>(args) and yield execution — gas
  // reserves for the resume; weight is the yield weight. Returns yield idx.
  yieldCreate(method: string, argsJson: string, gas: number, weight: number): number;
  // yieldResume: resume a yielded promise — (dataId, payload).
  yieldResume(dataId: string, payload: string): number;
  /** promiseYield* are the raw near.* spellings of yieldCreate/yieldResume. */
  promiseYieldCreate(method: string, argsJson: string, gas: number, weight: number): number;
  promiseYieldResume(dataId: string, payload: string): number;

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
  /** Shorter aliases for the G1/G2 point sums. */
  blsG1Sum(buf: string): string;
  blsG2Sum(buf: string): string;
  /** BN254 G2 point sum (pairs with altBn128G1Sum). */
  altBn128G2Sum(buf: string): string;

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
  /**
   * Start a cross-contract call. Deposit is a u128 decimal STRing
   * ("0" for none) — yocto values overflow JS `number`; ALL promise
   * deposits are strings, matching promiseThen and the batch forms.
   */
  promiseCreate(target: string, method: string, argsJson: string, deposit: string, gas: number): number;
  promiseThen(p: number, target: string, method: string, argsJson: string, deposit: string, gas: number): number;
  promiseAnd(p1: number, p2: number, p3?: number): number;

  // ── promise batches (multi-action promises; strings, not raw ABI) ──
  promiseBatchCreate(target: string): number;
  promiseBatchThen(p: number, target: string): number;
  promiseBatchActionTransfer(p: number, yoctoAmount: string): void;
  /** Note arg order: deposit (string) BEFORE gas. */
  promiseBatchActionFunctionCall(p: number, method: string, argsJson: string, yoctoDeposit: string | bigint, gas: number): void;
  promiseBatchActionCreateAccount(p: number): void;
  /** Global contracts (protocol 66): deploy code immutably under its sha256 code hash. */
  promiseBatchActionDeployGlobalContract(p: number, code: string): void;
  /** Global contracts: deploy code updatable by the owner account id. */
  promiseBatchActionDeployGlobalContractByAccountId(p: number, code: string): void;
  /** Global contracts: adopt an existing global under this account. */
  promiseBatchActionUseGlobalContract(p: number, sha256Hex: string): void;
  /** Function call with gas weight (batched chains). */
  promiseBatchActionFunctionCallWeight(p: number, method: string, argsJson: string, yoctoDeposit: string | bigint, gas: number, weight: number): void;
  /** Staking: stake yocto on the validator key. */
  promiseBatchActionStake(p: number, yoctoAmount: string, publicKey: string): void;
  /** Add an access key with full access. */
  promiseBatchActionAddKeyWithFullAccess(p: number, publicKey: string): void;
  /** Add a function-call access key with allowance. */
  promiseBatchActionAddKeyWithFunctionCall(p: number, publicKey: string, allowance: string, receiverId: string, methodNames: string[]): void;
  /** Delete an access key. */
  promiseBatchActionDeleteKey(p: number, publicKey: string): void;
  /** Delete this account, sending remaining balance to beneficiary. */
  promiseBatchActionDeleteAccount(p: number, beneficiaryId: string): void;
  /** Return a promise as this call's outcome (async return pattern). */
  promiseReturn(p: number): void;
  /** Whether promise result idx succeeded (1/0) — callbacks only. */
  promiseSucceeded(idx: number): number;
  /** How many promise results are readable in this callback. */
  promiseResultsCount(): number;
  /** Gas-key delegation (NEP-611): batch actions need the gas-key batch
   *  variants — full access, function-call scoped, and direct transfers. */
  promiseBatchActionAddGasKeyWithFullAccess(p: number, publicKey: string): void;
  promiseBatchActionAddGasKeyWithFunctionCall(p: number, publicKey: string, allowance: string, receiverId: string, methodNames: string[]): void;
  promiseBatchActionTransferToGasKey(p: number, yoctoAmount: string): void;
  /** Adopt an existing global contract under this account (by account id). */
  promiseBatchActionUseGlobalContractByAccountId(p: number, accountId: string): void;
  // Raw-ABI forms (ptr/len pairs, not strings) also exist for stake,
  // addKeyWithFullAccess, addKeyWithFunctionCall, deleteKey, deleteAccount,
  // deployContract — awkward from TS; reach for them only if you must.
};

// ── JS std shims (2026-08-30) ─────────────────────────────────────────
// console.log → near/log (args space-joined, auto to-string'd).
// Math.abs, Math.max, Math.min, Math.sqrt, Math.floor, Math.round,
// Math.ceil → same-named int builtins; Math.pow(a, b) → (expt a b).
// Other Math.* hard-errors at the frontend (2026-10-05).
// JSON.stringify(scalar) → json-quote; JSON.parse: NOT NEEDED — tx args
// arrive parsed; use typed params / near.jsonGet.
// (console/Math/JSON value types come from lib — not redeclared here.)
interface JSON {
  /** JSON array text via map(json-quote). */
  stringifyArr(arr: LispArr<string | number>): string;
}

// ── ergonomics-v2 (2026-10-07) ──────────────────────────────────────────
/** u128 amount of yoctoNEAR (10¹⁸ yocto = 1 NEAR) as a DECIMAL STRING —
 *  the near-sdk `Balance`/`NearToken` spelling. Sources: attachedDepositU128(),
 *  accountBalance(), transferU128 amounts. Arithmetic ONLY via u128.* —
 *  ENFORCED at compile time (2026-10-08): raw `+ - * / %` on two money
 *  values is an error (`+` concatenates decimal strings; i64 ops
 *  truncate). One-sided `+` stays legal ("total:" + amt).
 *  Return contract: `: Yocto` / `: Promise<Yocto>` must return a provable
 *  u128 value — await result, u128Add(...), deposit/balance read, decimal
 *  string. `"label:" + x` is concat (display text): annotate
 *  `: Promise<string>`, not `: Promise<Yocto>`.
 *  `const d: Yocto = ...` must also initialize from a provable u128
 *  value, and d itself carries the arithmetic rule.
 *  Money SOURCES: await results seed money only for balance/supply
 *  reads (ftBalanceRaw/ftBalanceOf/balanceOf/ftTotalSupply/ftSupplyFor)
 *  — an await of any other method binds a plain string, and returning
 *  it as `: Promise<Yocto>` is rejected. SINKS: transferU128's amount
 *  must be a provable u128 value — an unannotated string is rejected
 *  (`const s: Yocto = "5"` is the fix). Boundaries (greppable custody):
 *  `jsonGet`/`jsonGetStr` reads and `storageGet(k) ?? "0"` ledger reads
 *  may feed sinks but carry NO arithmetic seal (the reader can't know
 *  the domain) — assert the money domain with the annotation:
 *  `const amt: Yocto = ...`. Op closure: u128Add(...) etc. return
 *  money. */
type Yocto = string;

/** u128 raw amount in TOKEN DECIMALS (NEP-141 `amount`) as a DECIMAL
 *  STRING — ftBalanceRaw()/ftTransfer-scale values. Same ENFORCED rule:
 *  arithmetic via u128.* only — raw `+` on two money values is a compile
 *  error. */
type Amount = string;

/** @deprecated — use Yocto (NEAR-denominated) or Amount (FT raw).
 *  Same return contract as Yocto: annotated returns/consts are checked. */
type Money = string;

/** Fail the transaction with msg when cond is false (full state
 *  rollback — nothing the entry wrote before the assert survives). */
declare function assert(cond: boolean, msg: string): void;

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
