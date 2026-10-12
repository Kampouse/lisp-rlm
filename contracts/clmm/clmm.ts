// clmm.ts — Concentrated Liquidity Market Maker, lisp-rlm TS dialect.
// FULL u128 port: no bigint lattice exists in this dialect. All money /
// sqrtPrice values are DECIMAL STRINGS in the u128 domain, manipulated via
// u128Add/Sub/Mul/MulDiv/Div + u128Lt/Gt/Eq. Ticks are small numbers used
// only as loop counters / keys; Q32 fixed point: sp = price^0.5 * 2^32.

type U128 = Yocto; // phantom money seal: only provable u128 sources mint it

const Q32: U128 = "4294967296"; // 2^32
const TICK_OFF: number = 50000;

const POOL0 = '{"sp":"3221225472","tick":"0","liq":"0","ts":"0","tc":"0","bal0":"0","bal1":"0"}';

// ── small integer helpers (lattice Num domain) ──
function num(x: any): number { return strToNum(x); }
function s(n: any): string { return toStr(n); }
function s128(n: number): U128 { return u128FromNum(n); }
// Explicit conversion (the only string->money door): validates the
// decimal parse via the u128 host (traps on junk) and returns an
// op result, which carries the seal.
function seal(v: string): U128 { return u128Add(v, "0"); }

// ── pool record ──
function poolRec(): any {
  return near.storageGet("clmm:pool") ?? POOL0;
}
function savePool(p: any) {
  near.storageSet("clmm:pool", p);
}

function tickKey(tick: number, field: number): string {
  return "clmm:t:" + s(tick + TICK_OFF) + ":" + s(field);
}
// signed net liquidity as sign-magnitude pair: field 0 = pos mag, field 2 = neg mag
function netPos(tick: number): U128 {
  return seal(near.storageGet(tickKey(tick, 0)) ?? "0");
}
function netNeg(tick: number): U128 {
  return seal(near.storageGet(tickKey(tick, 2)) ?? "0");
}
function netSet(tick: number, pos: U128, neg: U128): number {
  near.storageSet(tickKey(tick, 0), pos);
  near.storageSet(tickKey(tick, 2), neg);
}
// net += liq (signed)
function netAdd(tick: number, liq: U128): number {
  let pos: U128 = netPos(tick);
  let neg: U128 = netNeg(tick);
  if (u128IsZero(neg)) {
    netSet(tick, u128Add(pos, liq), "0");
  } else if (u128Lt(liq, neg) || u128Eq(liq, neg)) {
    netSet(tick, pos, u128Sub(neg, liq));
  } else {
    netSet(tick, u128Sub(liq, neg), "0");
  }
}
// net -= liq (signed)
function netSub(tick: number, liq: U128): number {
  let pos: U128 = netPos(tick);
  let neg: U128 = netNeg(tick);
  if (u128IsZero(pos)) {
    netSet(tick, "0", u128Add(neg, liq));
  } else if (u128Lt(liq, pos) || u128Eq(liq, pos)) {
    netSet(tick, u128Sub(pos, liq), neg);
  } else {
    netSet(tick, "0", u128Sub(liq, pos));
  }
}
// signed net value: liq + net (for swap crossing), aborts on negative result
function netApply(liq: U128, tick: number): U128 {
  let pos: U128 = netPos(tick);
  let neg: U128 = netNeg(tick);
  if (u128IsZero(neg)) {
    return u128Add(liq, pos);
  }
  if (u128Lt(liq, neg)) {
    near.abort("negative pool liq");
  }
  return u128Sub(liq, neg);
}
function tickNet(tick: number): string {
  // display form: pos - neg, "-" prefix when negative (NOT money —
  // the sign-prefixed shape is display text by design)
  let pos: U128 = netPos(tick);
  let neg: U128 = netNeg(tick);
  if (u128IsZero(neg)) {
    return pos;
  }
  if (u128Lt(pos, neg)) {
    return "-" + s(num(u128Sub(neg, pos)));
  }
  return u128Sub(pos, neg);
}
function tickGross(tick: number): U128 {
  return seal(near.storageGet(tickKey(tick, 1)) ?? "0");
}

// ── u128 arithmetic (a*b/c full precision; div0 aborts in host) ──
function muldiv(a: U128, b: U128, c: U128): U128 {
  if (u128IsZero(c)) {
    near.abort("muldiv div0");
  }
  return u128Div(u128Mul(a, b), c);
}

// pow32(base, exp): base^exp in Q32, exp may be negative (small |exp|).
// exp is a small number; the running value stays in u128.
function pow32(base: U128, exp: number): U128 {
  if (exp < 0) {
    return u128Div(u128Mul(Q32, Q32), pow32(base, -exp));
  }
  let r: U128 = Q32;
  let c: U128 = base;
  let n: number = exp;
  while (n > 0) {
    if (n % 2 == 1) {
      r = muldiv(r, c, Q32);
    }
    c = muldiv(c, c, Q32);
    n = n / 2;
  }
  return r;
}

// 1.0001 in Q32 = 4295397017.29 -> truncated 4295397017
function baseQ32(): U128 {
  return "4295397017";
}

// exact isqrt on u128 strings via Newton: y = (x + n/x) / 2
function isqrt(n: U128): U128 {
  if (u128Lt(n, "2")) return n;
  let x: U128 = n;
  let y: U128 = u128Div(u128Add(x, u128Div(n, x)), "2");
  while (u128Lt(y, x)) {
    x = y;
    y = u128Div(u128Add(x, u128Div(n, x)), "2");
  }
  return x;
}

// sqrtPrice at tick in Q32: sqrt(1.0001^t) * 2^32.
// isqrt(P) * 2^32 == isqrt(P * 2^32) exactly, so this stays exact.
function spAtTick(tick: number): U128 {
  return u128Mul(isqrt(pow32(baseQ32(), tick)), Q32);
}

// find t with spAtTick(t) <= sp < spAtTick(t+1), from hint t0
function tickFromSp(sp: U128, t0: number): number {
  let t: number = t0;
  while (u128Gt(spAtTick(t), sp)) {
    t = t - 1;
  }
  while (u128Gt(sp, spAtTick(t + 1))) {
    t = t + 1;
  }
  return t;
}

// dx to move sqrtP from sp down to spTgt at liquidity L:
// dx = L * Q32 * (sp - spTgt) / (sp * spTgt)
function dxForDrop(liq: U128, sp: U128, spTgt: U128): U128 {
  return muldiv(liq, muldiv(u128Sub(sp, spTgt), Q32, sp), spTgt);
}

// ── TWAP (biased cumulative: Σ(tick+BIAS)·dt), u128 throughout ──
function accCarry() {
  let p = poolRec();
  let now: U128 = near.blockTimestamp();
  let last: U128 = jsonGetU128("ts", p);
  if (u128Gt(now, last)) {
    // cum = tc + (tick+BIAS) * (now - last)
    let d: U128 = u128Sub(now, last);
    let rate: U128 = s128(num(jsonGet("tick", p)) + TICK_OFF);
    let cum: U128 = u128Add(jsonGetU128("tc", p), u128Mul(rate, d));
    savePool(jsonSet(jsonSet(p, "ts", now), "tc", cum));
  } else {
    savePool(jsonSet(p, "ts", now));
  }
}

// ── core ops ──
export function initialize(sp: U128, tick: number, t0: string, t1: string): string {
  if (near.storageGet("clmm:pool") != null) {
    near.abort("already initialized");
  }
  if (near.storageGet("clmm:pool") != null) {
    near.abort("ERR_ALREADY_INITIALIZED");
  }
  let p = jsonSet(jsonSet(jsonSet(jsonSet(POOL0, "sp", sp), "tick", s(tick)), "ts", near.blockTimestamp()), "tc", "0");
  savePool(jsonSet(jsonSet(p, "t0", t0), "t1", t1));
  return "ok";
}

function balAdd0(amt: U128) {
  let p = poolRec();
  savePool(jsonSet(p, "bal0", u128Add(jsonGetU128("bal0", p), amt)));
}
function balAdd1(amt: U128) {
  let p = poolRec();
  savePool(jsonSet(p, "bal1", u128Add(jsonGetU128("bal1", p), amt)));
}
function balSub0(amt: U128) {
  let p = poolRec();
  if (u128Lt(jsonGetU128("bal0", p), amt)) {
    near.abort("insufficient pool bal0");
  }
  savePool(jsonSet(p, "bal0", u128Sub(jsonGetU128("bal0", p), amt)));
}
function balSub1(amt: U128) {
  let p = poolRec();
  if (u128Lt(jsonGetU128("bal1", p), amt)) {
    near.abort("insufficient pool bal1");
  }
  savePool(jsonSet(p, "bal1", u128Sub(jsonGetU128("bal1", p), amt)));
}

function doAddLiquidity(lower: number, upper: number, liq: U128): string {
  accCarry();
  netSub(lower, liq);
  netAdd(upper, liq);
  near.storageSet(tickKey(lower, 1), u128Add(tickGross(lower), liq));
  near.storageSet(tickKey(upper, 1), u128Add(tickGross(upper), liq));
  let p = poolRec();
  let tick = num(jsonGet("tick", p));
  if (tick >= lower && tick < upper) {
    savePool(jsonSet(p, "liq", u128Add(jsonGetU128("liq", p), liq)));
  }
  return liq;
}

function doRemoveLiquidity(lower: number, upper: number, liq: U128): string {
  accCarry();
  netAdd(lower, liq);
  netSub(upper, liq);
  near.storageSet(tickKey(lower, 1), u128Sub(tickGross(lower), liq));
  near.storageSet(tickKey(upper, 1), u128Sub(tickGross(upper), liq));
  let p = poolRec();
  let tick = num(jsonGet("tick", p));
  if (tick >= lower && tick < upper) {
    if (u128Lt(jsonGetU128("liq", p), liq)) {
      near.abort("liq underflow");
    }
    savePool(jsonSet(p, "liq", u128Sub(jsonGetU128("liq", p), liq)));
  }
  return liq;
}

export function removeLiquidity(lower: number, upper: number, liq: U128): string {
  return doRemoveLiquidity(lower, upper, liq);
}

// one in-range fill of dx at current liquidity; updates sp, returns dy
function swap0Fill(dx: U128): U128 {
  let p = poolRec();
  let sp: U128 = jsonGetU128("sp", p);
  let liq: U128 = jsonGetU128("liq", p);
  // newSp = sp*L / (L + dx*sp/Q32)
  let denom: U128 = u128Add(liq, muldiv(dx, sp, Q32));
  let newSp: U128 = muldiv(sp, liq, denom);
  let dy: U128 = muldiv(liq, u128Sub(sp, newSp), Q32);
  savePool(jsonSet(p, "sp", newSp));
  return dy;
}

// swap0: sell token0, price down. Fill segments, cross ticks, repeat.
function swap0Core(dx: U128): string {
  accCarry();
  let out: U128 = "0";
  let remaining: U128 = dx;
  while (!u128IsZero(remaining)) {
    let p = poolRec();
    let liq: U128 = jsonGetU128("liq", p);
    if (u128IsZero(liq)) {
      break;
    }
    let sp: U128 = jsonGetU128("sp", p);
    let tick = num(jsonGet("tick", p));
    let spBelow: U128 = spAtTick(tick); // lower edge of current tick cell
    if (!u128Lt(spBelow, sp)) {
      // sp already at/below cell edge (edge case after cross): fill all
      out = u128Add(out, swap0Fill(remaining));
      remaining = "0";
    } else {
      let dxEdge: U128 = dxForDrop(liq, sp, spBelow);
      if (u128Lt(remaining, dxEdge) || u128Eq(remaining, dxEdge)) {
        out = u128Add(out, swap0Fill(remaining));
        remaining = "0";
      } else {
        // reach cell boundary, cross DOWN: apply net at tick, move to tick-1
        out = u128Add(out, muldiv(liq, u128Sub(sp, spBelow), Q32));
        let newLiq: U128 = netApply(liq, tick);
        let p2 = jsonSet(poolRec(), "sp", spBelow);
        savePool(jsonSet(jsonSet(p2, "liq", newLiq), "tick", s(tick - 1)));
        remaining = u128Sub(remaining, dxEdge);
      }
    }
  }
  // recompute final tick from new sqrtP (fixes drift from per-cell stepping)
  let p = poolRec();
  savePool(jsonSet(p, "tick", s(tickFromSp(jsonGetU128("sp", p), num(jsonGet("tick", p))))));
  return out;
}

// swap1: sell token1, price up. Single fill + tick recompute (v1).
function swap1Core(dyIn: U128): string {
  accCarry();
  let p = poolRec();
  let sp: U128 = jsonGetU128("sp", p);
  let liq: U128 = jsonGetU128("liq", p);
  if (u128IsZero(liq)) {
    near.abort("no liquidity");
  }
  let newSp: U128 = u128Add(sp, muldiv(dyIn, Q32, liq));
  // dx = L * Q32 * (newSp - sp) / (sp * newSp)
  let dx: U128 = muldiv(liq, muldiv(u128Sub(newSp, sp), Q32, sp), newSp);
  savePool(jsonSet(jsonSet(p, "sp", newSp), "tick", s(tickFromSp(newSp, num(jsonGet("tick", p))))));
  return dx;
}

export function swap0(dx: U128): string { return swap0Core(dx); }
export function swap1(dyIn: U128): string { return swap1Core(dyIn); }

export function addLiquidity(lower: number, upper: number, liq: U128): string {
  return doAddLiquidity(lower, upper, liq);
}

// ── Position NFTs ──
export function mintPosition(lower: number, upper: number, liq: U128): string {
  doAddLiquidity(lower, upper, liq);
  let id: number = num(near.storageGet("clmm:p_next") ?? "0");
  let rec = '{"lo":' + s(lower) + ',"up":' + s(upper) + ',"lq":"' + liq + '"}';
  near.storageSet("clmm:p:" + s(id), rec);
  near.storageSet("clmm:po:" + s(id), near.predecessorAccountId());
  near.storageSet("clmm:p_next", s(id + 1));
  return s(id);
}

export function removePosition(id: number): string {
  let owner = near.storageGet("clmm:po:" + s(id)) ?? "";
  if (owner == "") {
    near.abort("no such position");
  }
  if (owner != near.predecessorAccountId()) {
    near.abort("not owner");
  }
  let rec: any = near.storageGet("clmm:p:" + s(id)) ?? "";
  if (rec == "") {
    near.abort("no such position");
  }
  let freed = doRemoveLiquidity(num(jsonGetStr("lo", rec)), num(jsonGetStr("up", rec)), jsonGetU128("lq", rec));
  let a0: U128 = jsonGetU128("a0", rec);
  let a1: U128 = jsonGetU128("a1", rec);
  balSub0(a0);
  balSub1(a1);
  near.storageSet("clmm:p:" + s(id), "");
  near.storageSet("clmm:po:" + s(id), "");
  payOut(jsonGetStr("t0", poolRec()), owner, a0);
  payOut(jsonGetStr("t1", poolRec()), owner, a1);
  return freed;
}

export function position(id: number): string {
  let owner = near.storageGet("clmm:po:" + s(id)) ?? "";
  let rec = near.storageGet("clmm:p:" + s(id)) ?? "{}";
  return owner + " " + rec;
}

// ── TWAP ──
export function twapSnap(): string {
  accCarry();
  let p = poolRec();
  near.storageSet("clmm:snap_ts", jsonGetU128("ts", p));
  near.storageSet("clmm:snap_tc", jsonGetU128("tc", p));
  return jsonGetU128("tc", p);
}

// arithmetic-mean biased tick since last snap; subtract BIAS for true mean.
// view-safe: no storage writes (pure carry computed inline).
export function twapTick(): string {
  let now: U128 = near.blockTimestamp();
  let snapTs: U128 = near.storageGet("clmm:snap_ts") ?? "0";
  let snapTc: U128 = near.storageGet("clmm:snap_tc") ?? "0";
  let dt: U128 = u128Sub(now, snapTs);
  if (u128IsZero(dt)) {
    return jsonGet("tick", poolRec());
  }
  let p = poolRec();
  // pure = tc + (tick+BIAS) * (now - ts)
  let rate: U128 = s128(num(jsonGet("tick", p)) + TICK_OFF);
  let pure: U128 = u128Add(jsonGetU128("tc", p), u128Mul(rate, u128Sub(now, jsonGetU128("ts", p))));
  return s(num(u128Sub(u128Div(u128Sub(pure, snapTc), dt), s128(TICK_OFF))));
}


// ── token flow (NEP-141) ──
// t0/t1 are the FT contract accounts recorded by initialize. Inflow arrives
// via ft_transfer_call -> ft_on_transfer; outflow leaves via ft_transfer
// promise batches (1 yocto attached, standard NEP-141 pull).
function isT0(acct: string): boolean { return acct == jsonGetStr("t0", poolRec()); }

function payOut(tokenAcct: string, to: string, amt: U128) {
  if (u128IsZero(amt)) { return; }
  let pi = near.promiseBatchCreate(tokenAcct);
  near.promiseBatchActionFunctionCall(pi, "ft_transfer",
    '{"receiver_id":' + jsonQuote(to) + ',"amount":"' + amt + '","memo":"clmm"}',
    "1", 10000000000000);
}

// mint a position on behalf of sender with `amt` of the token that arrived
function mintFor(sender: string, lower: number, upper: number, amt: U128, is0: number): string {
  doAddLiquidity(lower, upper, amt);
  let id: number = num(near.storageGet("clmm:p_next") ?? "0");
  let a0: U128 = "0";
  let a1: U128 = "0";
  if (is0 == 1) { a0 = amt; } else { a1 = amt; }
  let rec = '{"lo":' + s(lower) + ',"up":' + s(upper) + ',"lq":"' + amt + '","a0":"' + a0 + '","a1":"' + a1 + '"}';
  near.storageSet("clmm:p:" + s(id), rec);
  near.storageSet("clmm:po:" + s(id), sender);
  near.storageSet("clmm:p_next", s(id + 1));
  if (is0 == 1) { balAdd0(amt); } else { balAdd1(amt); }
  return s(id);
}

// split "a:b:c" at first colon; returns [head, rest]. rest="" if none.
function msgSplit1(msg: string): string[] {
  let i = 0;
  while (i < strLength(msg)) {
    if (strSlice(msg, i, i + 1) == ":") {
      return [strSlice(msg, 0, i), strSlice(msg, i + 1, strLength(msg))];
    }
    i = i + 1;
  }
  return [msg, ""];
}

// NEP-141 receiver. msg grammar:
//   "mint:<lower>:<upper>"  -> open position for sender with full amount
//   "swap0"                 -> sell token0 (this deposit), pay out token1
//   "swap1"                 -> sell token1 (this deposit), pay out token0
//   "swap0:<minOut>:<deadline>"   -> same, abort if out < minOut or now > deadline
//   "swap1:<minOut>:<deadline>"   -> same, other direction
export function ft_on_transfer(): string {
  let sender = near.jsonGetStr("sender_id") ?? "";
  let amt: U128 = near.jsonGetU128("amount") ?? "0";
  let msg = near.jsonGetStr("msg") ?? "";
  let from = near.predecessorAccountId();
  near.log("DBG from=[" + from + "] t0=[" + jsonGetStr("t0", poolRec()) + "] t1=[" + jsonGetStr("t1", poolRec()) + "]");
  let is0 = isT0(from);
  let is1 = from == jsonGetStr("t1", poolRec());
  if (!is0 && !is1) {
    near.abort("ERR_UNKNOWN_TOKEN");
  }
  if (u128IsZero(amt)) {
    near.abort("ERR_ZERO");
  }
  // guarded swaps: "swap0:<minOut>:<deadline>" / "swap1:<minOut>:<deadline>"
  let swapKind: string = "";
  if (strLength(msg) >= 5 && strSlice(msg, 0, 5) == "swap0") { swapKind = "0"; }
  if (strLength(msg) >= 5 && strSlice(msg, 0, 5) == "swap1") { swapKind = "1"; }
  if (swapKind != "") {
    // parse <minOut>:<deadline> after "swapN"
    let rest = strSlice(msg, 5, strLength(msg));
    let colon = -1;
    let i = 0;
    while (i < strLength(rest)) {
      if (strSlice(rest, i, i + 1) == ":") { colon = i; }
      i = i + 1;
    }
    if (colon < 1) { near.abort("ERR_MSG"); }
    // memo text parsed at runtime — strSlice is NOT a provable source;
    // seal() is the explicit conversion (traps on junk via the u128 parse)
    let minOut: U128 = seal(strSlice(rest, 0, colon));
    let deadline: U128 = seal(strSlice(rest, colon + 1, strLength(rest)));
    if (u128Gt(near.blockTimestamp(), deadline)) {
      near.abort("ERR_DEADLINE");
    }
    let out: U128 = "0";
    if (swapKind == "0") {
      if (!is0) { near.abort("ERR_WRONG_TOKEN"); }
      balAdd0(amt);
      out = swap0Core(amt);
      balSub1(out);
      payOut(jsonGetStr("t1", poolRec()), sender, out);
    } else {
      if (!is1) { near.abort("ERR_WRONG_TOKEN"); }
      balAdd1(amt);
      out = swap1Core(amt);
      balSub0(out);
      payOut(jsonGetStr("t0", poolRec()), sender, out);
    }
    if (u128Lt(out, minOut)) {
      near.abort("ERR_MIN_OUT");
    }
  } else if (strLength(msg) >= 5 && strSlice(msg, 0, 5) == "mint:") {
    let rest = strSlice(msg, 5, strLength(msg));
    let colon = -1;
    let i = 0;
    while (i < strLength(rest)) {
      if (strSlice(rest, i, i + 1) == ":") { colon = i; }
      i = i + 1;
    }
    if (colon < 1) { near.abort("ERR_MSG"); }
    let isZeroTok: number = 0;
    if (is0) { isZeroTok = 1; }
    let id = mintFor(sender, num(strSlice(rest, 0, colon)), num(strSlice(rest, colon + 1, strLength(rest))), amt, isZeroTok);
    near.log("EVENT_JSON:" + '{"standard":"clmm","version":"1.0.0","event":"position_minted","data":{"id":"' + id + '","owner":' + jsonQuote(sender) + '}}');
  } else {
    near.abort("ERR_MSG");
  }
  // full amount consumed (refunds handled by aborts above)
  return "0";
}

// ── views ──
export function getPrice(): string { return jsonGetU128("sp", poolRec()); }
export function getTick(): string { return jsonGet("tick", poolRec()); }
export function getLiq(): string { return jsonGetU128("liq", poolRec()); }
export function viewSpAtTick(tick: number): string { return spAtTick(tick); }
export function viewTickNet(tick: number): string { return tickNet(tick); }
