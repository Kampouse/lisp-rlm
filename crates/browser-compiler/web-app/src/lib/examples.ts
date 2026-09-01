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
    source: `// Lending v4 — u128 + interest + LIQUIDATIONS.
// 10% APY accrues per-second. health < 10000 bp → anyone but the
// borrower can repay up to half the debt and seize collateral at a
// 5% bonus. 50% LTV, 5% fee (ceiled), floor interest.
// Args: { "amt": "10000000000000000000000000" } (10 NEAR)

const LTV_BP = 5000n;
const FEE_BP = 500n;
const APY_BP = 1000n;
const SCALE = 10000n;
const YEAR_SEC = 31536000n;
const ZERO = 0n;
const SEC = 1000000000n;
const LIQ_BONUS_BP = 500n;
const TWO = 2n;

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
  let raw = near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}';
  let a = accrue(raw, near.blockTimestamp());
  let next = jsonSet(a, "dep", a.dep + amt);
  if (raw.ts == ZERO) {
    next = jsonSet(next, "own", who);   // first deposit stamps the owner
  }
  near.storageSet("lv4:" + who, next);
  return next;
}

export function borrow(amt: bigint): string {
  let who = near.signerAccountId();
  let a = accrue(
    near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
  let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
  let bor = a.bor + add;
  if (a.dep * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  let next = jsonSet(a, "bor", bor);
  near.storageSet("lv4:" + who, next);
  return next;
}

export function withdraw(amt: bigint): string {
  let who = near.signerAccountId();
  let a = accrue(
    near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
  let dep = a.dep - amt;
  if (dep * LTV_BP < a.bor * SCALE) {
    near.abort("withdraw would undercollateralize");
  }
  let next = jsonSet(a, "dep", dep);
  near.storageSet("lv4:" + who, next);
  return next;
}

export function liquidate(victim: string, amt: bigint): string {
  let a = accrue(
    near.storageGet("lv4:" + victim) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
  if (a.bor == ZERO) { near.abort("nothing to liquidate"); }
  if (a.dep * LTV_BP >= a.bor * SCALE) { near.abort("account healthy"); }
  if (near.signerAccountId() == a.own) { near.abort("cannot liquidate yourself"); }
  if (amt * TWO > a.bor) { near.abort("close factor 50%"); }
  let seize = (amt * (SCALE + LIQ_BONUS_BP)) / SCALE;
  if (seize > a.dep) { near.abort("collateral exhausted"); }
  let next = jsonSet(a, "bor", a.bor - amt);
  next = jsonSet(next, "dep", a.dep - seize);
  near.storageSet("lv4:" + victim, next);
  return next;
}

export function health(): string {
  let who = near.signerAccountId();
  let a = accrue(
    near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
  if (a.bor == ZERO) { return "inf"; }
  return (a.dep * LTV_BP) / a.bor;
}`,
  },
  {
    name: 'Token (TS)',
    icon: '🪙',
    target: 'near',
    lang: 'ts',
    source: `// Fungible Token — NEP-141 subset, full u128 precision.
// Balances live as decimal strings under "ft:<account>"; first minter
// becomes owner. Args: { "to": "owner.test.near", "amount": "1000000000000000000000000" }

const ZERO = 0n;

export function ftMint(to: string, amount: bigint): string {
  if ((near.storageGet("ft:own") ?? "") == "") {
    near.storageSet("ft:own", near.signerAccountId());
  }
  if (near.signerAccountId() != (near.storageGet("ft:own") ?? "")) {
    near.abort("only the owner may mint");
  }
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function ftTransfer(to: string, amount: bigint): string {
  let who = near.signerAccountId();
  let from = near.storageGet("ft:" + who) ?? ZERO;
  if (from < amount) {
    near.abort("insufficient balance");
  }
  let toBal = near.storageGet("ft:" + to) ?? ZERO;
  near.storageSet("ft:" + who, from - amount);
  near.storageSet("ft:" + to, toBal + amount);
  return "ok";
}

export function ftBalanceOf(who: string): string {
  return near.storageGet("ft:" + who) ?? ZERO;
}

// ── Allowances (NEP-141 approve/transferFrom) ──
function allowanceKey(owner: string, spender: string): string {
  return "fta:" + owner + ":" + spender;
}

export function ftApprove(spender: string, amount: bigint): string {
  let who = near.signerAccountId();
  let key = allowanceKey(who, spender);
  let cur = near.storageGet(key) ?? ZERO;
  if (cur != ZERO && amount != ZERO) {
    near.abort("reset allowance to zero first");   // NEP-141 race rule
  }
  near.storageSet(key, amount);
  return "ok";
}

export function ftTransferFrom(from: string, to: string, amount: bigint): string {
  let who = near.signerAccountId();
  let aKey = allowanceKey(from, who);
  let allowed = near.storageGet(aKey) ?? ZERO;
  let bal = near.storageGet("ft:" + from) ?? ZERO;
  if (allowed < amount || bal < amount) {
    near.abort("allowance or balance too low");
  }
  let toBal = near.storageGet("ft:" + to) ?? ZERO;
  near.storageSet(aKey, allowed - amount);
  near.storageSet("ft:" + from, bal - amount);
  near.storageSet("ft:" + to, toBal + amount);
  return "ok";
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
  {
    name: 'FT-Lend (TS)',
    icon: '🏦',
    target: 'near',
    lang: 'ts',
    source: `// FT + lending desk: your FT balance is collateral, borrow the same
// token against it. Cap = locked * 5000 / 10000 (50% LTV, no price
// feed needed — same token both sides). Keys: ft:<who> balance,
// lt:<who> locked, ld:<who> debt.
// Try: ftMint {to,amount}, lendDeposit {amount}, lendBorrow {amount}…

const ZERO = 0n;
const LTV = 5000n;
const SCALE = 10000n;

export function ftMint(to: string, amount: bigint): string {
  if ((near.storageGet("ft:own") ?? "") == "") {
    near.storageSet("ft:own", near.signerAccountId());
  }
  if (near.signerAccountId() != (near.storageGet("ft:own") ?? "")) {
    near.abort("only the owner may mint");
  }
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function lendDeposit(amount: bigint): string {
  let who = near.signerAccountId();
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  if (bal < amount) { near.abort("insufficient balance"); }
  let locked = near.storageGet("lt:" + who) ?? ZERO;
  near.storageSet("ft:" + who, bal - amount);
  near.storageSet("lt:" + who, locked + amount);
  return "locked:" + (locked + amount);
}

export function lendBorrow(amount: bigint): string {
  let who = near.signerAccountId();
  let debt = near.storageGet("ld:" + who) ?? ZERO;
  let cap = (near.storageGet("lt:" + who) ?? ZERO) * LTV / SCALE;
  if (debt + amount > cap) { near.abort("would exceed borrow cap"); }
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ld:" + who, debt + amount);
  near.storageSet("ft:" + who, bal + amount);
  return "debt:" + (debt + amount);
}

export function lendWithdraw(amount: bigint): string {
  let who = near.signerAccountId();
  let locked = near.storageGet("lt:" + who) ?? ZERO;
  if (locked < amount) { near.abort("withdraw exceeds locked"); }
  let debt = near.storageGet("ld:" + who) ?? ZERO;
  if (debt > (locked - amount) * LTV / SCALE) {
    near.abort("would undercollateralize");
  }
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("lt:" + who, locked - amount);
  near.storageSet("ft:" + who, bal + amount);
  return "locked:" + (locked - amount);
}

export function lendHealth(): string {
  let who = near.signerAccountId();
  let cap = (near.storageGet("lt:" + who) ?? ZERO) * LTV / SCALE;
  return "cap:" + cap + " debt:" + (near.storageGet("ld:" + who) ?? ZERO);
}`,
  },
  {
    name: 'HTLC Escrow (TS)',
    icon: '🔐',
    target: 'near',
    lang: 'ts',
    source: `// Hash-timelock escrow: claim with the secret before the deadline,
// or the sender refunds after it. The hashlock is sha256(secret) as a
// 64-char hex digest — scanner-safe inside JSON records.
// Try: escrowNew {recipient, secret, amount, timeoutSec}, then
// escrowClaim {id, secret} as the recipient, escrowRefund {id} later.

const ZERO = 0n;
const NANOS = 1000000000n;
const REC0 = '{"sender":"","recipient":"","amt":"0","tl":"0","state":""}';

export function ftMint(to: string, amount: bigint): string {
  if ((near.storageGet("ft:own") ?? "") == "") {
    near.storageSet("ft:own", near.signerAccountId());
  }
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function escrowNew(recipient: string, secret: string, amount: bigint, timeoutSec: bigint): string {
  let who = near.signerAccountId();
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  if (bal < amount) { near.abort("insufficient balance"); }
  let id = near.storageGet("e:count") ?? ZERO;
  id = id + 1n;
  let rec = jsonSet(REC0, "sender", who);
  rec = jsonSet(rec, "recipient", recipient);
  rec = jsonSet(rec, "amt", amount);
  rec = jsonSet(rec, "tl", near.blockTimestamp() + timeoutSec * NANOS);
  rec = jsonSet(rec, "state", "PENDING");
  near.storageSet("e:" + id, rec);
  near.storageSet("e:count", id);
  near.storageSet("eh:" + id, near.sha256Hash(secret));
  near.storageSet("ft:" + who, bal - amount);
  return "escrow:" + id;
}

export function escrowClaim(id: bigint, secret: string): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("e:" + id) ?? REC0;
  if (rec.state != "PENDING") { near.abort("not pending"); }
  if (who != rec.recipient) { near.abort("only the recipient may claim"); }
  if (near.sha256Hash(secret) != (near.storageGet("eh:" + id) ?? "")) {
    near.abort("wrong secret");
  }
  if (near.blockTimestamp() > rec.tl) { near.abort("timed out"); }
  let rBal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ft:" + who, rBal + rec.amt);
  near.storageSet("e:" + id, jsonSet(rec, "state", "CLAIMED"));
  return "claimed:" + rec.amt;
}

export function escrowRefund(id: bigint): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("e:" + id) ?? REC0;
  if (rec.state != "PENDING") { near.abort("not pending"); }
  if (who != rec.sender) { near.abort("only the sender may refund"); }
  if (near.blockTimestamp() < rec.tl) { near.abort("not yet timed out"); }
  let sBal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ft:" + who, sBal + rec.amt);
  near.storageSet("e:" + id, jsonSet(rec, "state", "REFUNDED"));
  return "refunded:" + rec.amt;
}`,
  },
];
