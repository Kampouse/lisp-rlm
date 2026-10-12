// clmm.ts — Concentrated Liquidity Market Maker, lisp-rlm TS dialect.
// FULL u128 port: no bigint lattice exists in this dialect. All money /
// sqrtPrice values are DECIMAL STRINGS in the u128 domain, manipulated via
// u128Add/Sub/Mul/MulDiv/Div + u128Lt/Gt/Eq. Ticks are small numbers used
// only as loop counters / keys; Q32 fixed point: sp = price^0.5 * 2^32.

type U = string; // u128 decimal string

const Q32: U = "4294967296"; // 2^32
const TICK_OFF: number = 50000;

const POOL0 = '{"sp":"3221225472","tick":"0","liq":"0","ts":"0","tc":"0"}';

// ── small integer helpers (lattice Num domain) ──
function num(x: any): number { return strToNum(x); }
function s(n: any): U { return toStr(n); }
function s128(n: number): U { return u128FromNum(n); }

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
function tickNet(tick: number): U {
  return near.storageGet(tickKey(tick, 0)) ?? "0";
}
function tickGross(tick: number): U {
  return near.storageGet(tickKey(tick, 1)) ?? "0";
}

// ── u128 arithmetic (a*b/c full precision; div0 aborts in host) ──
function muldiv(a: U, b: U, c: U): U {
  if (u128IsZero(c)) {
    near.abort("muldiv div0");
  }
  return u128MulDiv(a, b, c);
}

// pow32(base, exp): base^exp in Q32, exp may be negative (small |exp|).
// exp is a small number; the running value stays in u128.
function pow32(base: U, exp: number): U {
  if (exp < 0) {
    return u128Div(u128Mul(Q32, Q32), pow32(base, -exp));
  }
  let r: U = Q32;
  let c: U = base;
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
function baseQ32(): U {
  return "4295397017";
}

// exact isqrt on u128 strings via Newton: y = (x + n/x) / 2
function isqrt(n: U): U {
  if (u128Lt(n, "2")) return n;
  let x: U = n;
  let y: U = u128Div(u128Add(x, u128Div(n, x)), "2");
  while (u128Lt(y, x)) {
    x = y;
    y = u128Div(u128Add(x, u128Div(n, x)), "2");
  }
  return x;
}

// sqrtPrice at tick in Q32: sqrt(1.0001^t) * 2^32.
// isqrt(P) * 2^32 == isqrt(P * 2^32) exactly, so this stays exact.
function spAtTick(tick: number): U {
  return u128Mul(isqrt(pow32(baseQ32(), tick)), Q32);
}

// find t with spAtTick(t) <= sp < spAtTick(t+1), from hint t0
function tickFromSp(sp: U, t0: number): number {
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
function dxForDrop(liq: U, sp: U, spTgt: U): U {
  return muldiv(liq, muldiv(u128Sub(sp, spTgt), Q32, sp), spTgt);
}

// ── TWAP (biased cumulative: Σ(tick+BIAS)·dt), u128 throughout ──
function accCarry() {
  let p = poolRec();
  let now: U = near.blockTimestamp();
  let last: U = jsonGet("ts", p);
  if (u128Gt(now, last)) {
    // cum = tc + (tick+BIAS) * (now - last)
    let d: U = u128Sub(now, last);
    let rate: U = s128(num(jsonGet("tick", p)) + TICK_OFF);
    let cum: U = u128Add(jsonGet("tc", p), u128Mul(rate, d));
    savePool(jsonSet(jsonSet(p, "ts", now), "tc", cum));
  } else {
    savePool(jsonSet(p, "ts", now));
  }
}

// ── core ops ──
export function initialize(sp: U, tick: number, t0: string, t1: string): string {
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

function doAddLiquidity(lower: number, upper: number, liq: U): string {
  accCarry();
  near.storageSet(tickKey(lower, 0), u128Sub(tickNet(lower), liq));
  near.storageSet(tickKey(upper, 0), u128Add(tickNet(upper), liq));
  near.storageSet(tickKey(lower, 1), u128Add(tickGross(lower), liq));
  near.storageSet(tickKey(upper, 1), u128Add(tickGross(upper), liq));
  let p = poolRec();
  let tick = num(jsonGet("tick", p));
  if (tick >= lower && tick < upper) {
    savePool(jsonSet(p, "liq", u128Add(jsonGet("liq", p), liq)));
  }
  return liq;
}

function doRemoveLiquidity(lower: number, upper: number, liq: U): string {
  accCarry();
  near.storageSet(tickKey(lower, 0), u128Add(tickNet(lower), liq));
  near.storageSet(tickKey(upper, 0), u128Sub(tickNet(upper), liq));
  near.storageSet(tickKey(lower, 1), u128Sub(tickGross(lower), liq));
  near.storageSet(tickKey(upper, 1), u128Sub(tickGross(upper), liq));
  let p = poolRec();
  let tick = num(jsonGet("tick", p));
  if (tick >= lower && tick < upper) {
    if (u128Lt(jsonGet("liq", p), liq)) {
      near.abort("liq underflow");
    }
    savePool(jsonSet(p, "liq", u128Sub(jsonGet("liq", p), liq)));
  }
  return liq;
}

export function removeLiquidity(lower: number, upper: number, liq: U): string {
  return doRemoveLiquidity(lower, upper, liq);
}

// one in-range fill of dx at current liquidity; updates sp, returns dy
function swap0Fill(dx: U): U {
  let p = poolRec();
  let sp: U = jsonGet("sp", p);
  let liq: U = jsonGet("liq", p);
  // newSp = sp*L / (L + dx*sp/Q32)
  let denom: U = u128Add(liq, muldiv(dx, sp, Q32));
  let newSp: U = muldiv(sp, liq, denom);
  let dy: U = muldiv(liq, u128Sub(sp, newSp), Q32);
  savePool(jsonSet(p, "sp", newSp));
  return dy;
}

// swap0: sell token0, price down. Fill segments, cross ticks, repeat.
export function swap0(dx: U): string {
  accCarry();
  let out: U = "0";
  let remaining: U = dx;
  while (!u128IsZero(remaining)) {
    let p = poolRec();
    let liq: U = jsonGet("liq", p);
    if (u128IsZero(liq)) {
      break;
    }
    let sp: U = jsonGet("sp", p);
    let tick = num(jsonGet("tick", p));
    let spBelow: U = spAtTick(tick); // lower edge of current tick cell
    if (!u128Lt(spBelow, sp)) {
      // sp already at/below cell edge (edge case after cross): fill all
      out = u128Add(out, swap0Fill(remaining));
      remaining = "0";
    } else {
      let dxEdge: U = dxForDrop(liq, sp, spBelow);
      if (u128Lt(remaining, dxEdge) || u128Eq(remaining, dxEdge)) {
        out = u128Add(out, swap0Fill(remaining));
        remaining = "0";
      } else {
        // reach cell boundary, cross DOWN: apply net at tick, move to tick-1
        out = u128Add(out, muldiv(liq, u128Sub(sp, spBelow), Q32));
        let net: U = tickNet(tick);
        let newLiq: U = u128Add(liq, net);
        let p2 = jsonSet(poolRec(), "sp", spBelow);
        savePool(jsonSet(jsonSet(p2, "liq", newLiq), "tick", s(tick - 1)));
        remaining = u128Sub(remaining, dxEdge);
      }
    }
  }
  // recompute final tick from new sqrtP (fixes drift from per-cell stepping)
  let p = poolRec();
  savePool(jsonSet(p, "tick", s(tickFromSp(jsonGet("sp", p), num(jsonGet("tick", p))))));
  return out;
}

// swap1: sell token1, price up. Single fill + tick recompute (v1).
export function swap1(dyIn: U): string {
  accCarry();
  let p = poolRec();
  let sp: U = jsonGet("sp", p);
  let liq: U = jsonGet("liq", p);
  if (u128IsZero(liq)) {
    near.abort("no liquidity");
  }
  let newSp: U = u128Add(sp, muldiv(dyIn, Q32, liq));
  // dx = L * Q32 * (newSp - sp) / (sp * newSp)
  let dx: U = muldiv(liq, muldiv(u128Sub(newSp, sp), Q32, sp), newSp);
  savePool(jsonSet(jsonSet(p, "sp", newSp), "tick", s(tickFromSp(newSp, num(jsonGet("tick", p))))));
  return dx;
}

export function addLiquidity(lower: number, upper: number, liq: U): string {
  return doAddLiquidity(lower, upper, liq);
}

// ── Position NFTs ──
export function mintPosition(lower: number, upper: number, liq: U): string {
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
  let freed = doRemoveLiquidity(num(jsonGet("lo", rec)), num(jsonGet("up", rec)), jsonGet("lq", rec));
  near.storageSet("clmm:p:" + s(id), "");
  near.storageSet("clmm:po:" + s(id), "");
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
  near.storageSet("clmm:snap_ts", jsonGet("ts", p));
  near.storageSet("clmm:snap_tc", jsonGet("tc", p));
  return jsonGet("tc", p);
}

// arithmetic-mean biased tick since last snap; subtract BIAS for true mean.
// view-safe: no storage writes (pure carry computed inline).
export function twapTick(): string {
  let now: U = near.blockTimestamp();
  let snapTs: U = near.storageGet("clmm:snap_ts") ?? "0";
  let snapTc: U = near.storageGet("clmm:snap_tc") ?? "0";
  let dt: U = u128Sub(now, snapTs);
  if (u128IsZero(dt)) {
    return jsonGet("tick", poolRec());
  }
  let p = poolRec();
  // pure = tc + (tick+BIAS) * (now - ts)
  let rate: U = s128(num(jsonGet("tick", p)) + TICK_OFF);
  let pure: U = u128Add(jsonGet("tc", p), u128Mul(rate, u128Sub(now, jsonGet("ts", p))));
  return s(num(u128Sub(u128Div(u128Sub(pure, snapTc), dt), s128(TICK_OFF))));
}

// ── views ──
export function getPrice(): string { return jsonGet("sp", poolRec()); }
export function getTick(): string { return jsonGet("tick", poolRec()); }
export function getLiq(): string { return jsonGet("liq", poolRec()); }
export function viewSpAtTick(tick: number): string { return spAtTick(tick); }
export function viewTickNet(tick: number): string { return tickNet(tick); }
