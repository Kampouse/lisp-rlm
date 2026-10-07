// lib/clmm_pool_v4.ts — CLMM pool v4.3 ported to the lisp-rlm TypeScript dialect.
// Byte-for-byte storage-compatible with lib/clmm_pool_v4.lisp (keys S0..S4, F,
// AB, PB, SHT, SH:<a>, TOKA, TOKB, DY, REM) — a drop-in code replacement for
// any deployed v4.x pool.
//
// Model (identical to the Lisp original):
//   - LPs deposit B via ft_on_transfer (predecessor == TOKB, msg ignored).
//     First deposit bootstraps the ladder 1000:2000:5000:3000:1500 at prices
//     0.98/0.99/1.00/1.01/1.02 B-per-A; later deposits proportional to fill.
//   - Swaps A→B via ft_on_transfer (predecessor == TOKA, msg "swap[:min]"):
//     ladder walk, fee 30bps of used, ALL received A accrues to AB.
//   - Withdraw burns shares, pays A then B through pay_b gate; failed payout
//     legs reverse the operation's exact deltas (v4.3 rollback).
//
// PORT-SPECIFIC HARDENING (beyond the Lisp original):
//   The Lisp build uses growing-limb math (arbitrary precision). The TS
//   dialect's u128 builtins are fixed-range, so this port enforces pool
//   capacity caps that keep every internal product ≤ 1e36 < 2^128 (~340×
//   margin): PB ≤ 1e18, AB ≤ 1e18, inputs ≤ 30 digits ASCII. Overflow by
//   construction is impossible even against tokens that lie about amounts.
/// <reference path="../ts/lisp-rlm.d.ts" />

// ── state helpers ──────────────────────────────────────────────────────

function bz(k: string): string {
  return near.storageGet(k) ?? "0";
}

function shKey(a: string): string {
  return strCat("SH:", a);
}

function selfCall(): boolean {
  return near.predecessorAccountId() === near.currentAccountId();
}

function slotGet(i: number): string {
  return near.storageGet(strCat("S", toStr(i))) ?? "0";
}

function slotSet(i: number, v: string): void {
  near.storageSet(strCat("S", toStr(i)), v);
}

function feeGet(): string {
  return bz("F");
}

// ASCII digits, length ≤ 30, non-empty. (Lisp gate: ≤40 digits, empty OK.)
function numOk(s: string): boolean {
  if (s === "") { return false; }
  if (strLength(s) > 30) { return false; }
  let i = 0;
  while (i < strLength(s)) {
    const d = strSlice(s, i, i + 1);
    if (strIndexOf("0123456789", d) < 0) { return false; } else { i = i + 1; }
  }
  return true;
}

// pool capacity caps (u128 product-safety, see header)
const CAP_TOTAL = "1000000000000000000"; // 1e18

// ── ladder core (unrolled 5 slots: 98/99/100/101/102 per 100, fee 30bps) ──

// one slot step: returns [rem, dyAdd, feeAdd, laLeft]
function stepGet(dx: string, la: string, pnum: string): LispArr<string> {
  let used = "0"; let rem = "0"; let left = "0";
  if (u128Lt(dx, la)) { used = dx; rem = "0"; left = u128Sub(la, dx); }
  else { used = la; rem = u128Sub(dx, la); left = "0"; }
  const out = ["", "", "", ""];
  out[0] = rem;
  out[1] = u128Add("0", u128Div(u128Mul(used, pnum), "100"));
  out[2] = u128Div(u128Mul(used, "30"), "10000");
  out[3] = left;
  return out;
}

function poolSwap(amt: string): string {
  let dx = amt;
  let dy = "0";
  let fe = "0";
  // slot 0 @ 98/100
  let st = stepGet(dx, slotGet(0), "98");
  dx = st[0]; dy = u128Add(dy, st[1]); fe = u128Add(fe, st[2]); slotSet(0, st[3]);
  // slot 1 @ 99/100
  st = stepGet(dx, slotGet(1), "99");
  dx = st[0]; dy = u128Add(dy, st[1]); fe = u128Add(fe, st[2]); slotSet(1, st[3]);
  // slot 2 @ 100/100
  st = stepGet(dx, slotGet(2), "100");
  dx = st[0]; dy = u128Add(dy, st[1]); fe = u128Add(fe, st[2]); slotSet(2, st[3]);
  // slot 3 @ 101/100
  st = stepGet(dx, slotGet(3), "101");
  dx = st[0]; dy = u128Add(dy, st[1]); fe = u128Add(fe, st[2]); slotSet(3, st[3]);
  // slot 4 @ 102/100
  st = stepGet(dx, slotGet(4), "102");
  dx = st[0]; dy = u128Add(dy, st[1]); fe = u128Add(fe, st[2]); slotSet(4, st[3]);
  near.storageSet("F", u128Add(feeGet(), fe));
  near.storageSet("DY", dy);
  near.storageSet("REM", dx);
  return strCat(dy, "/", dx);
}

function poolState(): string {
  return strCat("s0:", slotGet(0), " s1:", slotGet(1), " s2:", slotGet(2),
                " s3:", slotGet(3), " s4:", slotGet(4), " F:", feeGet());
}

// ── init ───────────────────────────────────────────────────────────────

function poolInit4(): string {
  if (near.storageHas("TOKA")) { return "already-initialized"; }
  slotSet(0, "0"); slotSet(1, "0"); slotSet(2, "0"); slotSet(3, "0"); slotSet(4, "0");
  near.storageSet("F", "0");
  near.storageSet("AB", "0");
  near.storageSet("PB", "0");
  near.storageSet("SHT", "0");
  near.storageSet("TOKA", near.jsonGetStr("toka") ?? "");
  near.storageSet("TOKB", near.jsonGetStr("tokb") ?? "");
  return "ok";
}

// ── liquidity (B side) ────────────────────────────────────────────────

// slot i += (amt·base_i/tot)·100/pnum_i — base = shape (first) or slot (later)
function slotWiden(i: number, base: string, amt: string, tot: string, pnum: string): void {
  const part = u128Div(u128Mul(amt, base), tot);
  slotSet(i, u128Add(slotGet(i), u128Div(u128Mul(part, "100"), pnum)));
}

// returns the unused-B refund for ft_on_transfer: "0" on success.
// Validate BEFORE mutating: dust / zero-share / bad-amount / over-cap all
// refund untouched (sybil/DoS guard, no state writes).
function addBLiq(amt: string, sender: string): string {
  if (u128Lt(amt, "10")) { return amt; } // below min deposit: refund
  const pb = bz("PB");
  const sht = bz("SHT");
  if (u128Gt(u128Add(pb, amt), CAP_TOTAL)) { return amt; } // pool capacity guard
  let sh = "0";
  if (sht === "0") { sh = amt; }
  else { sh = u128Div(u128Mul(amt, sht), pb); }
  if (sh === "0") { return amt; } // dust floors to zero shares
  const tot = u128Add(u128Add(u128Add(slotGet(0), slotGet(1)), u128Add(slotGet(2), slotGet(3))), slotGet(4));
  if (tot === "0") {
    slotWiden(0, "1000", amt, "12500", "98");
    slotWiden(1, "2000", amt, "12500", "99");
    slotWiden(2, "5000", amt, "12500", "100");
    slotWiden(3, "3000", amt, "12500", "101");
    slotWiden(4, "1500", amt, "12500", "102");
  } else {
    slotWiden(0, slotGet(0), amt, tot, "98");
    slotWiden(1, slotGet(1), amt, tot, "99");
    slotWiden(2, slotGet(2), amt, tot, "100");
    slotWiden(3, slotGet(3), amt, tot, "101");
    slotWiden(4, slotGet(4), amt, tot, "102");
  }
  near.storageSet("SHT", u128Add(sht, sh));
  near.storageSet("PB", u128Add(pb, amt));
  near.storageSet(shKey(sender), u128Add(bz(shKey(sender)), sh));
  return "0";
}

// ── swap payout gate (internal only) ───────────────────────────────────

// result(0) = the awaited B payout. On failure: reverse THIS swap's deltas
// (slots + fee + AB + PB, all linear so concurrent swaps compose) and
// return the trader's full amount (the ft_on_transfer unused-amount refund).
export function pay_out(): string {
  if (!selfCall()) { return "forbidden"; }
  const res = near.promiseResult(0);
  if (strLength(res) < 1) {
    slotSet(0, u128Add(slotGet(0), near.jsonGetStr("c0") ?? "0"));
    slotSet(1, u128Add(slotGet(1), near.jsonGetStr("c1") ?? "0"));
    slotSet(2, u128Add(slotGet(2), near.jsonGetStr("c2") ?? "0"));
    slotSet(3, u128Add(slotGet(3), near.jsonGetStr("c3") ?? "0"));
    slotSet(4, u128Add(slotGet(4), near.jsonGetStr("c4") ?? "0"));
    near.storageSet("F", u128Sub(feeGet(), near.jsonGetStr("f") ?? "0"));
    near.storageSet("AB", u128Sub(bz("AB"), near.jsonGetStr("u") ?? "0"));
    near.storageSet("PB", u128Add(bz("PB"), near.jsonGetStr("d") ?? "0"));
    return near.jsonGetStr("a") ?? "0";
  }
  return near.jsonGetStr("r") ?? "0";
}

// ── withdraw payout gate (internal only) ───────────────────────────────

// result(0) = the awaited payout:
//   g=1 two-leg: leg1 (A) — on fail reverse all deltas, no B paid;
//                on success pay leg2 (B).
//   g=0 single-leg (out-A was 0): B already paid by the awaited promise —
//         check only, never re-pay.
export function pay_b(): string {
  if (!selfCall()) { return "forbidden"; }
  const res = near.promiseResult(0);
  if (strLength(res) < 1) {
    slotSet(0, u128Add(slotGet(0), near.jsonGetStr("x0") ?? "0"));
    slotSet(1, u128Add(slotGet(1), near.jsonGetStr("x1") ?? "0"));
    slotSet(2, u128Add(slotGet(2), near.jsonGetStr("x2") ?? "0"));
    slotSet(3, u128Add(slotGet(3), near.jsonGetStr("x3") ?? "0"));
    slotSet(4, u128Add(slotGet(4), near.jsonGetStr("x4") ?? "0"));
    near.storageSet("SHT", u128Add(bz("SHT"), near.jsonGetStr("s") ?? "0"));
    const w = near.jsonGetStr("w") ?? "";
    near.storageSet(shKey(w), u128Add(bz(shKey(w)), near.jsonGetStr("s") ?? "0"));
    near.storageSet("AB", u128Add(bz("AB"), near.jsonGetStr("a") ?? "0"));
    near.storageSet("PB", u128Add(bz("PB"), near.jsonGetStr("b") ?? "0"));
    return "wd-failed";
  }
  if ((near.jsonGetStr("g") ?? "0") === "1") {
    const p = near.promiseBatchCreate(near.storageGet("TOKB") ?? "");
    near.promiseBatchActionFunctionCall(p, "ft_transfer",
      jsonSet(jsonSet("{}", "receiver_id", jsonQuote(near.jsonGetStr("r") ?? "")),
              "amount", jsonQuote(near.jsonGetStr("b") ?? "0")),
      "0", 40000000000000);
    near.promiseReturn(p);
    return "wd-queued";
  }
  return "wd-ok";
}

// ── withdraw ───────────────────────────────────────────────────────────

function withdraw4(): string {
  const who = near.predecessorAccountId();
  const sh = near.jsonGetStr("sh") ?? "";
  if (!numOk(sh)) { return "bad-sh"; }
  const have = bz(shKey(who));
  if (u128Lt(have, sh)) { return strCat("insufficient-shares-have:", have); }
  const ab = bz("AB");
  const pb = bz("PB");
  const sht = bz("SHT");
  const outA = u128Div(u128Mul(sh, ab), sht);
  const outB = u128Div(u128Mul(sh, pb), sht);
  const o0 = slotGet(0); const o1 = slotGet(1); const o2 = slotGet(2);
  const o3 = slotGet(3); const o4 = slotGet(4);
  near.storageSet("SHT", u128Sub(sht, sh));
  near.storageSet(shKey(who), u128Sub(have, sh));
  near.storageSet("AB", u128Sub(ab, outA));
  near.storageSet("PB", u128Sub(pb, outB));
  slotSet(0, u128Div(u128Mul(o0, bz("PB")), pb));
  slotSet(1, u128Div(u128Mul(o1, bz("PB")), pb));
  slotSet(2, u128Div(u128Mul(o2, bz("PB")), pb));
  slotSet(3, u128Div(u128Mul(o3, bz("PB")), pb));
  slotSet(4, u128Div(u128Mul(o4, bz("PB")), pb));
  let args = "{}";
  args = jsonSet(args, "r", jsonQuote(who));
  args = jsonSet(args, "b", jsonQuote(outB));
  args = jsonSet(args, "w", jsonQuote(who));
  args = jsonSet(args, "s", jsonQuote(sh));
  args = jsonSet(args, "a", jsonQuote(outA));
  args = jsonSet(args, "g", jsonQuote("1"));
  args = jsonSet(args, "x0", jsonQuote(u128Sub(o0, slotGet(0))));
  args = jsonSet(args, "x1", jsonQuote(u128Sub(o1, slotGet(1))));
  args = jsonSet(args, "x2", jsonQuote(u128Sub(o2, slotGet(2))));
  args = jsonSet(args, "x3", jsonQuote(u128Sub(o3, slotGet(3))));
  args = jsonSet(args, "x4", jsonQuote(u128Sub(o4, slotGet(4))));
  if (outA === "0") {
    const p = near.promiseBatchCreate(near.storageGet("TOKB") ?? "");
    near.promiseBatchActionFunctionCall(p, "ft_transfer",
      jsonSet(jsonSet("{}", "receiver_id", jsonQuote(who)), "amount", jsonQuote(outB)),
      "0", 40000000000000);
    near.promiseReturn(near.promiseThen(p, near.currentAccountId(), "pay_b",
      jsonSet(args, "g", jsonQuote("0")), "0", 60000000000000));
    return "wd-queued";
  }
  const p = near.promiseBatchCreate(near.storageGet("TOKA") ?? "");
  near.promiseBatchActionFunctionCall(p, "ft_transfer",
    jsonSet(jsonSet("{}", "receiver_id", jsonQuote(who)), "amount", jsonQuote(outA)),
    "0", 40000000000000);
  near.promiseReturn(near.promiseThen(p, near.currentAccountId(), "pay_b",
    args, "0", 60000000000000));
  return "wd-queued";
}

// ── NEP-141 entry ─────────────────────────────────────────────────────

// "swap" → "0"; "swap:NNN" → NNN
function msgMin(msg: string): string {
  if (strLength(msg) < 6) { return "0"; }
  return strSlice(msg, 5, strLength(msg));
}

export function ft_on_transfer(): string {
  const sender = near.jsonGetStr("sender_id") ?? "";
  const amt = near.jsonGetStr("amount") ?? "";
  if (!numOk(amt)) { return amt; }
  const pred = near.predecessorAccountId();
  if (pred === (near.storageGet("TOKB") ?? "")) {
    return addBLiq(amt, sender); // B side: liquidity (refund = unused)
  }
  if (pred === (near.storageGet("TOKA") ?? "")) {
    return swapLeg(sender, amt);
  }
  return amt; // wrong token: refund everything
}

function swapLeg(sender: string, amt: string): string {
  const min = msgMin(near.jsonGetStr("msg") ?? "");
  if (!numOk(min)) { return amt; }
    const f0 = feeGet();
    const s0v = slotGet(0); const s1v = slotGet(1); const s2v = slotGet(2);
    const s3v = slotGet(3); const s4v = slotGet(4);
    poolSwap(amt);
    const dy = bz("DY");
    const rem = bz("REM");
    if (u128Lt(dy, min)) {
      // slippage guard: restore pre-walk state, refund all A
      slotSet(0, s0v); slotSet(1, s1v); slotSet(2, s2v);
      slotSet(3, s3v); slotSet(4, s4v);
      near.storageSet("F", f0);
      return amt;
    }
    const usedA = u128Sub(amt, rem);
    if (u128Gt(u128Add(bz("AB"), usedA), CAP_TOTAL)) {
      // capacity guard: restore + refund (kept behind the slippage check
      // so state is only written once per rejected swap)
      slotSet(0, s0v); slotSet(1, s1v); slotSet(2, s2v);
      slotSet(3, s3v); slotSet(4, s4v);
      near.storageSet("F", f0);
      return amt;
    }
    near.storageSet("AB", u128Add(bz("AB"), usedA));
    near.storageSet("PB", u128Sub(bz("PB"), dy));
    if (dy === "0") { return rem; }
    const p = near.promiseBatchCreate(near.storageGet("TOKB") ?? "");
    let args = "{}";
    args = jsonSet(args, "r", jsonQuote(rem));
    args = jsonSet(args, "a", jsonQuote(amt));
    args = jsonSet(args, "u", jsonQuote(usedA));
    args = jsonSet(args, "d", jsonQuote(dy));
    args = jsonSet(args, "f", jsonQuote(u128Sub(feeGet(), f0)));
    args = jsonSet(args, "c0", jsonQuote(u128Sub(s0v, slotGet(0))));
    args = jsonSet(args, "c1", jsonQuote(u128Sub(s1v, slotGet(1))));
    args = jsonSet(args, "c2", jsonQuote(u128Sub(s2v, slotGet(2))));
    args = jsonSet(args, "c3", jsonQuote(u128Sub(s3v, slotGet(3))));
    args = jsonSet(args, "c4", jsonQuote(u128Sub(s4v, slotGet(4))));
    near.promiseBatchActionFunctionCall(p, "ft_transfer",
      jsonSet(jsonSet("{}", "receiver_id", jsonQuote(sender)), "amount", jsonQuote(dy)),
      "0", 40000000000000);
    near.promiseReturn(near.promiseThen(p, near.currentAccountId(), "pay_out",
      args, "0", 5000000000000));
    return "0";
}

// ── admin/dispatch entry (parity with the Lisp _run) ──────────────────

export function _run(): string {
  const op = near.jsonGetStr("op") ?? "";
  if (op === "init") { return poolInit4(); }
  if (op === "state") { return poolState(); }
  if (op === "lp") {
    const who = near.jsonGetStr("who") ?? "";
    return strCat("AB:", bz("AB"), " PB:", bz("PB"), " SHT:", bz("SHT"),
                  " SH:", bz(shKey(who)));
  }
  if (op === "withdraw") { return withdraw4(); }
  if (op === "swap") {
    const amt = near.jsonGetStr("amt") ?? "";
    if (!numOk(amt)) { return "bad-amt"; } else { return poolSwap(amt); }
  } else {
    return strCat("unknown-op:", op);
  }
}
