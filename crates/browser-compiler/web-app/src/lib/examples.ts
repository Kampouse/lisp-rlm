export interface Example {
  name: string;
  icon: string;
  source: string;
  target: 'p1' | 'p2' | 'pure' | 'near';
  lang?: 'ts';
}

export const examples: Example[] = [
  {
    name: 'Fibonacci',
    icon: '🌀',
    target: 'pure',
    source: `(define (fib n)
  (if (<= n 1)
    n
    (+ (fib (- n 1)) (fib (- n 2)))))

(define (main)
  (fib 10))`,
  },
  {
    name: 'Factorial',
    icon: '❗',
    target: 'pure',
    source: `(define (fact n)
  (if (<= n 1)
    1
    (* n (fact (- n 1)))))

(define (main)
  (fact 12))`,
  },
  {
    name: 'Counter',
    icon: '🔢',
    target: 'p1',
    source: `(memory 1)
(define (get_counter) (near/load "c"))
(define (set_counter val) (near/store "c" val))
(define (new) (set_counter 0))
(define (increment) (set_counter (+ (get_counter) 1)))
(define (get) (near/return (get_counter)))
(export "new" new false)
(export "increment" increment false)
(export "get" get true)`,
  },
  {
    name: 'Counter TS',
    icon: '🟦',
    target: 'p1',
    lang: 'ts',
    source: `// TypeScript dialect — lowered to Lisp, compiled to NEAR wasm.
// Same counter contract as the Lisp example, written in TS.

export function new_(): void {
  near.storageSet("c", "0");
}

export function increment(): void {
  near.storageSet("c", toStr(getCounter() + 1));
}

export function get_value(): string {
  return toStr(getCounter());
}

function getCounter(): number {
  return strToNum(near.storageGet("c") ?? "0");
}

// Dialect notes:
// - near.storageGet/Set, near.log, near.abort (host bindings)
// - toStr / strToNum / strLength (stdlib bridge)
// - storageGet returns str-or-nil; x ?? fallback handles the nil
// - new_ exports as NEAR's "new" constructor (reserved word in TS)
// - get_* functions are views (auto value_return)
// - values stored as strings, numbers are i64`,
  },
  {
    name: 'NEAR API Tour (TS)',
    icon: '🗺️',
    target: 'p1',
    lang: 'ts',
    source: `// NEAR API tour — every spelling below compile-verified.
// No imports needed: near.* / storage.* map to builtins automatically.
//
// ── Storage (string KV — store numbers as strings) ─────────────
//   near.storageSet(k, v)      storage.set(k, v)   // both spellings
//   near.storageGet(k) ?? ""   → string (raw result is str-or-nil)
//   near.storageHas(k) → bool  near.storageDel(k)
//
// ── Transaction context ────────────────────────────────────────
//   near.blockIndex()          near.blockTimestamp()
//   near.currentAccountId()    near.signerAccountId()
//   near.attachedDeposit()     near.prepaidGas()
//
// ── Logging / panic ────────────────────────────────────────────
//   console.log(x)             → on-chain log (str, num, or array)
//   near.abort("reason")       → panic + revert
//
// ── Strings & numbers ──────────────────────────────────────────
//   toStr(123) → "123"         strToNum("42") → 42
//   strLength("abc") → 3
//
// ── Cross-contract (async fns only, one await, first stmt) ────
//   const r = await near.callAwait("acct", "method", argsJson, deposit)
//
// Exports: export function / export const f = arrow.
// get_* names become views; new_ exports as NEAR's "new".

export function tour(): string {
  near.storageSet("visits", toStr(strToNum(near.storageGet("visits") ?? "0") + 1));

  console.log("account:", near.currentAccountId());
  console.log("block:", near.blockIndex());
  console.log("visits:", near.storageGet("visits"));
  console.log("array logging works too:", [1, 2, 3]);

  return near.storageGet("visits") ?? "0";
}

export const get_visits = (): string => near.storageGet("visits") ?? "0";`,
  },
  {
    name: 'Objects (TS)',
    icon: '🧱',
    target: 'p1',
    lang: 'ts',
    source: `// M2 objects — JSON-string values, zero conversion for
// storage / returns / cross-contract args.
//
// ── Literals self-encode ──────────────────────────────────────
//   { name: "bob", votes: 42, active: true }
//   strings → quoted, numbers bare, booleans bare true/false
//
// ── Reads ─────────────────────────────────────────────────────
//   o.name          → string (nil when absent — ?? "" to default)
//   o.server.port   → nested folds inline
//   numeric value?  → strToNum(o.votes) (reads are strings)
//
// ── Rebuild (immutable) ───────────────────────────────────────
//   o = jsonSet(o, "key", encodedValue)
//   strings: jsonQuote(s) · numbers: toStr(n)
//
// ── Type the param as string ──────────────────────────────────
//   (LispObj alias in the d.ts = string)

export function makeProfile(name: string): string {
  return { name: name, votes: 0, active: true };
}

export function new_(): string {
  // exported fns read args from tx input, so build inline here
  // (makeProfile is the typed API surface for external callers)
  near.storageSet("profile", { name: "bob", votes: 0, active: true });
  return "ok";
}

export function vote(): string {
  let p = near.storageGet("profile") ?? "{}";
  let nv = strToNum(p.votes) + 1;
  near.storageSet("profile", jsonSet(p, "votes", toStr(nv)));
  return p.name + ": " + toStr(nv) + " votes";
}

export function get_profile(): string {
  return near.storageGet("profile") ?? "{}";
}

// Nested reads: args { "cfg": { "server": { "port": "80" } } }
export function get_port(cfg: string): string {
  return cfg.server.port;
}

// ── Typed object params ──────────────────────────────────────
// Inline shape or a type alias — numeric props AUTO-DECODE.
type Ballot = { title: string; votes: number };

export function cast(b: Ballot): string {
  let nv = b.votes + 1;             // strToNum handled by the lowering
  return b.title + ": " + toStr(nv);
}

export function tally(b: Ballot, stamp: number): string {
  return { title: b.title, votes: b.votes, stamped: stamp };
}`,
  },
  {
    name: 'Lending (TS)',
    icon: '🏦',
    target: 'near',
    lang: 'ts',
    source: `// Lending v3 — u128 + TIME-BASED INTEREST.
// bigint → u128/* string math; 10% APY accrues per-second on every
// call via near.blockTimestamp(). 50% LTV, 5% fee (ceiled).
// Args: { "amt": "10000000000000000000000000" } (10 NEAR)

const LTV_BP = 5000n;
const FEE_BP = 500n;
const APY_BP = 1000n;
const SCALE = 10000n;
const YEAR_SEC = 31536000n;
const ZERO = 0n;
const SEC = 1000000000n;

function accrue(acct: string, ts: bigint): string {
  let bor = acct.bor;
  let elapsed = (ts - acct.ts) / SEC;
  if (bor > ZERO && elapsed > ZERO) {
    bor = bor + (bor * APY_BP * elapsed) / (SCALE * YEAR_SEC);
  }
  let next = jsonSet(acct, "bor", bor);
  return jsonSet(next, "ts", ts);
}

export function deposit(amt: bigint): string {
  let who = near.signerAccountId();
  let raw = near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}';
  let acct = accrue(raw, near.blockTimestamp());
  if (raw.ts == ZERO) {
    acct = jsonSet(acct, "ts", near.blockTimestamp());
  }
  let next = jsonSet(acct, "dep", acct.dep + amt);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function borrow(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
  let bor = acct.bor + add;
  if (acct.dep * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  let next = jsonSet(acct, "bor", bor);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function health(): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  if (acct.bor == ZERO) {
    return "inf";
  }
  return (acct.dep * LTV_BP) / acct.bor;
}`,
  },
  {
    name: 'CC View',
    icon: '🔗',
    target: 'p1',
    source: `(memory 1)
;; Cross-contract view call — queries wrap.near wNEAR balance
;; Set the JSON args below to query any account
(define (query)
  (let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
    (near/promise_result 0)))
(export "query" query true)`,
  },
  {
    name: 'HTTP Fetch',
    icon: '🌐',
    target: 'p2',
    source: `(define (get-weather)
  (let ((url "https://api.open-meteo.com/v1/forecast?latitude=45.50&longitude=-73.57&current_weather=true"))
    (http-get url)))

(define (main)
  (get-weather))`,
  },
  {
    name: 'P2 Storage',
    icon: '💾',
    target: 'p2',
    source: `;; OutLayer P2 storage demo
;; Uses localStorage in browser, real OutLayer storage on NEAR
(define (main)
  (begin
    (storage-set "count" "42")
    (storage-get "count")))`,
  },
  {
    name: 'Tests',
    icon: '✓',
    target: 'pure',
    source: `;; Test system demo
;; Tests use assert-equal, assert-true, assert-false

(define (add a b) (+ a b))

(test "addition works"
  (assert-equal 5 (add 2 3)))

(test "handles zero"
  (assert-equal 0 (add 0 0))
  (assert-equal 5 (add 5 0)))

(test "negative numbers"
  (assert-equal 0 (add -1 1))
  (assert-equal -8 (add -5 -3)))
`,
  },
  {
    name: 'HTTP POST',
    icon: '📤',
    target: 'p2',
    source: `(define (main)
  (let ((url "https://httpbin.org/post")
        (body "{\\"hello\\": \\"world\\"}"))
    (http-post url body)))`,
  },
  {
    name: 'Wallet POST',
    icon: '💳',
    target: 'p2',
    source: `;; Wallet-style: POST balance data to API
(define (report-balance account)
  (let ((body (str-concat "{\\"account\\":\\"" account "\\"}")))
    (http-post "https://api.example.com/balance" body)))

(define (main)
  (report-balance "user.near"))`,
  },
];