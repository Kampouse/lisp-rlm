// ─── clmm.ts — TS twin of examples/clmm.lisp (differential contract) ───
//
// Concentrated Liquidity Market Maker v1: Q32 fixed-point sqrt-price
// with tick granularity. Same storage layout (near/store_num tagged-i64
// keys), same policy numbers, same formulas as the lisp original:
//
//   key 0: sqrt_price_x32   key 1: current_tick   key 2: active_liquidity
//   tick state: ((tick + 50000) << 4) | field
//     field 0: liquidity_net    field 1: liquidity_gross
//
//   swap0: newP = L*P/(L + dx*P/Q32); dy = L*(P - newP)/Q32
//   swap1: newP = P + dy*Q32/L;     dx = L*(newP-P)*Q32/(P*newP)
//
// Driven by tests/test_clmm_ts.rs — the SAME scenario passes run against
// the lisp-compiled wasm; any divergence is a TS-frontend bug, not a
// contract bug.
//
// Shape notes (dialect compliance): no lambdas/closures; money-style
// u128 discipline N/A here (i64 Q32 math via muldiv/isqrt intrinsics,
// numbers auto-tagged like lisp literals); views via get_* export
// prefix (lisp's tick_net/tick_gross/sp_at_tick views export here as
// get_* twins — mapped by the test).

// ── constants ──

function q32(): number {
  return 1 << 32;
}

function tickOff(): number {
  return 50000;
}

// 1.0001 in Q32 = 10001 * Q32 / 10000
function baseQ32(): number {
  return mulDiv(10001, q32(), 10000);
}

// ── storage helpers ──

function tickKey(tick: number, field: number): number {
  return ((tick + tickOff()) << 4) | field;
}

function tickNetRaw(tick: number): number {
  return near.loadNum(tickKey(tick, 0));
}

function tickGrossRaw(tick: number): number {
  return near.loadNum(tickKey(tick, 1));
}

function priceRaw(): number {
  return near.loadNum(0);
}

function tickRaw(): number {
  return near.loadNum(1);
}

function liqRaw(): number {
  return near.loadNum(2);
}

// ── Q32 binary exponentiation: base^exp via repeated squaring ──

function pow32(base: number, exp: number): number {
  let r = q32();
  let c = base;
  let n = exp;
  while (n !== 0) {
    // n odd → r = r * c / Q32
    if ((n | 1) === n) {
      r = mulDiv(r, c, q32());
    }
    c = mulDiv(c, c, q32());
    n = n >> 1;
  }
  return r;
}

// sqrtPrice(tick) ≈ intSqrt(pow32) << 16 — exact at tick 0; ≤1ulp Q32 view
// precision (the exact form needs a ~2^64 intermediate the tagged range
// cannot hold — see the lisp twin's note; v5 does exact u128 tick math).
function spAtTickRaw(tick: number): number {
  return intSqrt(pow32(baseQ32(), tick)) << 16;
}

// ── pool initialization ──

export function initialize(): number {
  const sp = near.jsonGetInt("sp") ?? 0;
  const tick = near.jsonGetInt("tick") ?? 0;
  near.storeNum(0, sp);
  near.storeNum(1, tick);
  near.storeNum(2, 0);
  return 0;
}

// ── add liquidity: range [lower, upper) ──

export function add_liquidity(): number {
  const lower = near.jsonGetInt("lower") ?? 0;
  const upper = near.jsonGetInt("upper") ?? 0;
  const liq = near.jsonGetInt("liq") ?? 0;
  // lower: crossing DOWN removes → net -= liq ; upper: net += liq
  near.storeNum(tickKey(lower, 0), tickNetRaw(lower) + (0 - liq));
  near.storeNum(tickKey(upper, 0), tickNetRaw(upper) + liq);
  near.storeNum(tickKey(lower, 1), tickGrossRaw(lower) + liq);
  near.storeNum(tickKey(upper, 1), tickGrossRaw(upper) + liq);
  if (tickRaw() >= lower) {
    if (tickRaw() < upper) {
      near.storeNum(2, liqRaw() + liq);
    }
  }
  return liq;
}

// ── remove liquidity ──

export function remove_liquidity(): number {
  const lower = near.jsonGetInt("lower") ?? 0;
  const upper = near.jsonGetInt("upper") ?? 0;
  const liq = near.jsonGetInt("liq") ?? 0;
  near.storeNum(tickKey(lower, 0), tickNetRaw(lower) + liq);
  near.storeNum(tickKey(upper, 0), tickNetRaw(upper) + (0 - liq));
  near.storeNum(tickKey(lower, 1), tickGrossRaw(lower) + (0 - liq));
  near.storeNum(tickKey(upper, 1), tickGrossRaw(upper) + (0 - liq));
  if (tickRaw() >= lower) {
    if (tickRaw() < upper) {
      near.storeNum(2, liqRaw() + (0 - liq));
    }
  }
  return liq;
}

// ── swap token0 → token1 (price decreases) ──

export function swap0(): number {
  const dx = near.jsonGetInt("dx") ?? 0;
  const sp = priceRaw();
  const liq = liqRaw();
  if (liq === 0) {
    return 0;
  }
  const denom = liq + mulDiv(dx, sp, q32());
  const newSp = mulDiv(sp, liq, denom);
  const dy = mulDiv(liq, sp - newSp, q32());
  near.storeNum(0, newSp);
  return dy;
}

// ── swap token1 → token0 (price increases) ──

export function swap1(): number {
  const dyIn = near.jsonGetInt("dy_in") ?? 0;
  const sp = priceRaw();
  const liq = liqRaw();
  if (liq === 0) {
    return 0;
  }
  const dp = mulDiv(dyIn, q32(), liq);
  const newSp = sp + dp;
  const dx = mulDiv(liq, mulDiv(dp, q32(), sp), newSp);
  near.storeNum(0, newSp);
  return dx;
}

// ── views (get_* → view exports) ──

export function get_price(): number {
  return priceRaw();
}

export function get_tick(): number {
  return tickRaw();
}

export function get_liq(): number {
  return liqRaw();
}

// lisp exports tick_net/tick_gross/sp_at_tick as views — TS twins
// carry the get_ prefix (auto-view); the differential test maps names.
export function get_tick_net(): number {
  return tickNetRaw(near.jsonGetInt("tick") ?? 0);
}

export function get_tick_gross(): number {
  return tickGrossRaw(near.jsonGetInt("tick") ?? 0);
}

export function get_sp_at_tick(): number {
  return spAtTickRaw(near.jsonGetInt("tick") ?? 0);
}
