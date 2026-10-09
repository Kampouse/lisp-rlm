// ─── twap.ts — TS twin of examples/twap.lisp (differential contract) ───
//
// Same storage layout (o:<id>:<field>), same policy numbers, same
// verdicts. Driven by tests/test_twap_ts.rs with the SAME scenario
// passes as tests/test_twap.rs — every storage expectation must match
// the lisp original exactly.
//
// Shape notes (mirrors the lisp landmine-compliance section):
//   no lambdas/closures; u128 math via u128.* calls (money taint makes
//   raw + - * / % on amounts a compile error); u128.lt/eq usable
//   directly as if-conditions; values bound before use; storage is
//   str → str only.
//
//   create    payable — escrow total_in (attached yocto), store order
//   tick      permissionless — execute ONE slice: due → near.callAwait
//             swap with on_slice receipt callback; expired → accrue rx
//   on_slice  receipt — fee split (min(FEE_BPS, seller cap)), protocol
//             cut → burn, credit fo, advance nx
//   cancel    seller-only pre-start → full refund
//   finalize  nx == ns → refund rx, verdict filled_ok|avg_missed|avg_skip
//   get_order view

const FEE_BPS = "100";      // market default executor fee, 1%
const PROTO_BPS = "50";     // protocol cut → burn, 0.5% ($CROSS thesis)
const BP_TOTAL = "10000";
const BURN_ADDR = "burn.near";
const TGAS = 30000000000000;
const MAX_SLICES = "1000";

// ── storage helpers (str → str) ──

function keyOf(id: string, f: string): string {
  return "o:" + id + ":" + f;
}

function readField(id: string, f: string, d: string): string {
  const raw = near.storageGet(keyOf(id, f));
  if (raw === "") {
    return d;
  }
  return raw;
}

function writeField(id: string, f: string, v: string): void {
  near.storageSet(keyOf(id, f), v);
}

// executor captured at tick time (receipt callbacks see THIS contract
// as predecessor, so the fee payee must be remembered)
function executorOf(id: string): string {
  return near.storageGet(id + "|ex") ?? "";
}

// ── create ──
// args: pool, token_out, recipient, n_slices, interval_h, start_h,
//       grace_h, min_out_per_slice, min_avg_out, fee_bps_cap
// funds: attached yocto = total_in (escrowed by this contract)

export function create(): string {
  const pool = near.jsonGetStr("pool") ?? "";
  const tokenOut = near.jsonGetStr("token_out") ?? "";
  const recipient = near.jsonGetStr("recipient") ?? "";
  const total = near.jsonGetStr("n_slices") ?? "0";
  const interval = near.jsonGetStr("interval_h") ?? "0";
  const start = near.jsonGetStr("start_h") ?? "0";
  const grace = near.jsonGetStr("grace_h") ?? "0";
  const minOut = near.jsonGetStr("min_out_per_slice") ?? "0";
  const minAvg = near.jsonGetStr("min_avg_out") ?? "0";
  const feeCap = near.jsonGetStr("fee_bps_cap") ?? FEE_BPS;
  const deposit = near.attachedDepositU128();
  const h = near.blockHeight();

  if (u128.eq(deposit, "0")) {
    near.abort("ERR_ZERO");
  }
  if (pool === "") {
    near.abort("ERR_ARGS");
  }
  if (u128.eq(total, "0")) {
    near.abort("ERR_ARGS");
  }
  if (u128.lt(MAX_SLICES, total)) {
    near.abort("ERR_ARGS");
  }
  if (u128.eq(interval, "0")) {
    near.abort("ERR_ARGS");
  }
  if (u128.eq(grace, "0")) {
    near.abort("ERR_ARGS");
  }
  // start_h defaults to now + interval
  const startAt = u128.eq(start, "0") ? u128.add(u128FromNum(h), interval) : start;

  const id = near.storageGet("seq") ?? "0";
  const newId = u128.add(id, "1");
  near.storageSet("seq", newId);
  const seller = near.predecessorAccountId();
  writeField(newId, "seller", seller);
  writeField(newId, "pool", pool);
  writeField(newId, "token", tokenOut);
  writeField(newId, "rcp", recipient);
  writeField(newId, "ns", total);
  writeField(newId, "iv", interval);
  writeField(newId, "sh", startAt);
  writeField(newId, "gh", grace);
  writeField(newId, "mo", minOut);
  writeField(newId, "ma", minAvg);
  writeField(newId, "fc", feeCap);
  writeField(newId, "nx", "0");
  writeField(newId, "fo", "0");
  writeField(newId, "rx", "0");
  writeField(newId, "xp", "0");
  writeField(newId, "st", "open");
  // slice_in = total_in / n_slices; the remainder rides the last slice
  const sliceIn = u128.div(deposit, total);
  const used = u128.mul(sliceIn, total);
  const leftover = u128.sub(deposit, used);
  writeField(newId, "si", sliceIn);
  writeField(newId, "le", leftover);
  return newId;
}

// ── tick — the permissionless engine ──
// One call advances the order by ONE slice (swap or expiry).

export function tick(): string {
  const id = near.jsonGetStr("order_id") ?? "";
  const h = near.blockHeight();
  const executor = near.predecessorAccountId();
  const seller = readField(id, "seller", "");
  if (seller === "") {
    near.abort("ERR_NO_ORDER");
  }
  const cursor = readField(id, "nx", "0");
  const total = readField(id, "ns", "0");
  const start = readField(id, "sh", "0");
  const interval = readField(id, "iv", "0");
  const grace = readField(id, "gh", "0");
  if (u128.eq(cursor, total)) {
    near.abort("ERR_DONE");
  }
  // due_h = sh + nx*iv ; exp_h = due_h + gh
  const dueAt = u128.add(start, u128.mul(cursor, interval));
  const expAt = u128.add(dueAt, grace);
  const now = u128FromNum(h);
  if (u128.lt(now, dueAt)) {
    near.abort("ERR_NOT_DUE");
  }
  if (u128.lt(expAt, now)) {
    return tickExpired(id, cursor);
  }
  return tickSwap(id, cursor, executor);
}

// slice past grace: unfilled, its input becomes refundable
function tickExpired(id: string, cursor: string): string {
  const sliceIn = readField(id, "si", "0");
  const total = readField(id, "ns", "0");
  const expired = readField(id, "xp", "0");
  const nextIdx = u128.add(cursor, "1");
  const expired1 = u128.add(expired, "1");
  writeField(id, "nx", nextIdx);
  writeField(id, "xp", expired1);
  // last slice carries the remainder
  if (u128.eq(nextIdx, total)) {
    const lastSliceIn = u128.add(sliceIn, readField(id, "le", "0"));
    const refund2 = u128.add(readField(id, "rx", "0"), lastSliceIn);
    writeField(id, "rx", refund2);
    return "expired";
  }
  const refund2 = u128.add(readField(id, "rx", "0"), sliceIn);
  writeField(id, "rx", refund2);
  return "expired";
}

// due slice: remember the executor, fire the single swap promise.
// The await consumed here is exactly one per call — the single-use
// gate's happy path.
function tickSwap(id: string, cursor: string, executor: string): string {
  near.storageSet(id + "|ex", executor);
  const pool = readField(id, "pool", "");
  const total = readField(id, "ns", "0");
  const sliceIn = readField(id, "si", "0");
  const nextIdx = u128.add(cursor, "1");
  // last slice: attach the escrow remainder too
  const amt = u128.eq(nextIdx, total) ? u128.add(sliceIn, readField(id, "le", "0")) : sliceIn;
  const cb = `{"order":"${id}","k":"${cursor}","amt":"${amt}"}`;
  near.callAwait(pool, "swap", `{"order":"${id}","k":"${cursor}"}`,
    TGAS, "on_slice", TGAS, cb);
  return "queued";
}

// ── on_slice — receipt callback ──
// cb args: order, k, amt. Pool returned slice_out on promise_result(0).
// Receipt fence: only the CURRENT slice may settle — the cursor advances
// on settle, so k == nx holds exactly once per slice. A duplicate
// receipt (k behind the cursor: the shape every double-tick replay
// produces) aborts with ERR_STALE before any money moves. Aborting (not
// discarding) is deliberate: state reverts atomically, the retry paths
// keep their meaning, and the only producer of a stale receipt on-chain
// is a double-ticking executor — its own receipt failing is desired.

export function on_slice(): string {
  const id = near.jsonGetStr("order") ?? "";
  const k = near.jsonGetStr("k") ?? "0";
  const cursorNow = readField(id, "nx", "0");
  if (k !== cursorNow) {
    near.abort("ERR_STALE");
  }
  const sliceOut = near.promiseResult(0);
  const minOut = readField(id, "mo", "0");
  if (sliceOut === "") {
    // swap receipt failed → slice stays due for a retry tick
    near.log("SLICE_RETRY");
    return "retry";
  }
  if (u128.lt(sliceOut, minOut)) {
    // slippage guard → stays due, retry until grace (strict retry)
    near.log("SLICE_RETRY");
    return "retry";
  }
  const feeCap = readField(id, "fc", FEE_BPS);
  const cursor = readField(id, "nx", "0");
  // fee_bps = min(market default, seller cap)
  const feeBps = u128.lt(FEE_BPS, feeCap) ? FEE_BPS : feeCap;
  // `: Yocto` at birth — the money-taint sink check demands provable u128
  // values in transferU128's amount slot; these are u128-arith results,
  // which the annotation accepts (either spelling, nesting fine).
  const fee: Yocto = u128.div(u128.mul(sliceOut, feeBps), BP_TOTAL);
  const cut: Yocto = u128.div(u128.mul(sliceOut, PROTO_BPS), BP_TOTAL);
  const afterFee: Yocto = u128.sub(sliceOut, fee);
  const payout: Yocto = u128.sub(afterFee, cut);
  const filled2 = u128.add(readField(id, "fo", "0"), payout);
  const nextIdx = u128.add(cursor, "1");
  writeField(id, "fo", filled2);
  writeField(id, "nx", nextIdx);
  if (!u128.eq(fee, "0")) {
    near.transferU128(executorOf(id), fee);
  }
  if (!u128.eq(cut, "0")) {
    near.transferU128(BURN_ADDR, cut);
  }
  near.log("filled:" + k);
  return payout;
}

// ── cancel — seller only, before the first slice is due ──

export function cancel(): string {
  const id = near.jsonGetStr("order_id") ?? "";
  const who = near.predecessorAccountId();
  const h = near.blockHeight();
  const seller = readField(id, "seller", "");
  if (seller === "") {
    near.abort("ERR_NO_ORDER");
  }
  if (seller !== who) {
    near.abort("ERR_PERM");
  }
  const start = readField(id, "sh", "0");
  const cursor = readField(id, "nx", "0");
  const now = u128FromNum(h);
  if (!u128.lt(now, start)) {
    near.abort("ERR_LATE");
  }
  if (!u128.eq(cursor, "0")) {
    near.abort("ERR_LATE");
  }
  // total = si*ns + le — recompute from parts (u128-arith → provable)
  const totalEscrow: Yocto = u128.add(u128.mul(readField(id, "si", "0"), readField(id, "ns", "0")), readField(id, "le", "0"));
  writeField(id, "st", "cancelled");
  near.transferU128(seller, totalEscrow);
  return "cancelled";
}

// ── finalize — all slices filled or expired ──
// refunds accrued expired slice_in to the seller; status carries the
// min_avg_out verdict. Callable once nx == ns.

export function finalize(): string {
  const id = near.jsonGetStr("order_id") ?? "";
  const seller = readField(id, "seller", "");
  if (seller === "") {
    near.abort("ERR_NO_ORDER");
  }
  const cursor = readField(id, "nx", "0");
  const total = readField(id, "ns", "0");
  if (!u128.eq(cursor, total)) {
    near.abort("ERR_OPEN");
  }
  const filledOut = readField(id, "fo", "0");
  const refund = readField(id, "rx", "0");
  const expired = readField(id, "xp", "0");
  const minAvg = readField(id, "ma", "0");
  const filled = u128.sub(cursor, expired);
  const verdict = u128.eq(filled, "0") ? "avg_skip"
    : (u128.lt(filledOut, u128.mul(minAvg, filled)) ? "avg_missed" : "filled_ok");
  writeField(id, "st", verdict);
  if (!u128.eq(refund, "0")) {
    // ledger read inline at the sink — coalesce form (bare storageGet
    // types (opt str); `?? ""` is the sanctioned ledger boundary)
    near.transferU128(seller, near.storageGet(keyOf(id, "rx")) ?? "");
  }
  return verdict;
}

// ── view ──

export function get_order(): string {
  const id = near.jsonGetStr("order_id") ?? "";
  const seller = readField(id, "seller", "");
  if (seller === "") {
    return "null";
  }
  let j = "{}";
  j = jsonSet(j, "seller", jsonQuote(seller));
  j = jsonSet(j, "pool", jsonQuote(readField(id, "pool", "")));
  j = jsonSet(j, "ns", jsonQuote(readField(id, "ns", "0")));
  j = jsonSet(j, "nx", jsonQuote(readField(id, "nx", "0")));
  j = jsonSet(j, "fo", jsonQuote(readField(id, "fo", "0")));
  j = jsonSet(j, "rx", jsonQuote(readField(id, "rx", "0")));
  j = jsonSet(j, "xp", jsonQuote(readField(id, "xp", "0")));
  j = jsonSet(j, "si", jsonQuote(readField(id, "si", "0")));
  j = jsonSet(j, "st", jsonQuote(readField(id, "st", "open")));
  return j;
}
