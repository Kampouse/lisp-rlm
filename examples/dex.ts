// ─── dex.ts — minimal two-token constant-product DEX mock (near-mock demo) ───
// A Ref classic pool is exactly this shape (get_pool → reserves + fee_bps).
// The arb brain (arb_live.ts) swaps on it via near/call-await with per-leg
// min_out guards. All math fixed-point integers (micro-units 10^-6); no floats.
//
// Storage:
//   r:<key>:base / r:<key>:quote   raw integer reserves (micro-units)
//   s:<key>                        "base|quote|fee_bps"
//   n:<key>                        "inBase,outQuote" cumulative flows (asserts)
//   fees                           cumulative fee income (micro-units)
//
// Exports (entry points; args arrive as named params bound from JSON):
//   init(key, base, quote, fee_bps)      create/replace pool, reserves 0
//   fund(key, side, amount)              add reserves (side: base|quote)
//   quote(key, side, amount) → str       view: out for in (no state change)
//   swap(key, side, amount, min_out)     guarded swap; traps if out < min_out
//
// side "base" = sell base for quote; side "quote" = sell quote for base.

// ---- helpers (non-exported: exports are entry points, not callables) ----

function outFor(ain: number, rIn: number, rOut: number, feeBps: number): number {
  const ainFee = ain - (ain * feeBps) / 10000;
  return (ainFee * rOut) / (rIn + ainFee);
}

function reserves(key: string, side: string): number {
  return strToNum(near.storageGet("r:" + key + ":" + side) ?? "0");
}

function feeOf(key: string): number {
  const spec = near.storageGet("s:" + key) ?? "||30";
  const parts = strSplit(spec, "|");
  return strToNum(parts[2] ?? "30");
}

function addFlow(key: string, inAmt: number, outAmt: number): void {
  const cur = near.storageGet("n:" + key) ?? "0,0";
  const parts = strSplit(cur, ",");
  const prevIn = strToNum(parts[0] ?? "0");
  const prevOut = strToNum(parts[1] ?? "0");
  near.storageSet("n:" + key, toStr(prevIn + inAmt) + "," + toStr(prevOut + outAmt));
}

// ---- entry points ----

export function init(key: string, base: string, quote: string, fee_bps: number): number {
  near.storageSet("s:" + key, base + "|" + quote + "|" + toStr(fee_bps));
  near.storageSet("r:" + key + ":base", "0");
  near.storageSet("r:" + key + ":quote", "0");
  near.storageSet("n:" + key, "0,0");
  return 1;
}

export function fund(key: string, side: string, amount: number): number {
  const k = "r:" + key + ":" + side;
  const cur = strToNum(near.storageGet(k) ?? "0");
  near.storageSet(k, toStr(cur + amount));
  return cur + amount;
}

export function quote(key: string, side: string, amount: number): string {
  let rIn = reserves(key, "base");
  let rOut = reserves(key, "quote");
  if (side == "quote") {
    rIn = reserves(key, "quote");
    rOut = reserves(key, "base");
  }
  return toStr(outFor(amount, rIn, rOut, feeOf(key)));
}

export function swap(key: string, side: string, amount: number, min_out: number): string {
  let rIn = reserves(key, "base");
  let rOut = reserves(key, "quote");
  if (side == "quote") {
    rIn = reserves(key, "quote");
    rOut = reserves(key, "base");
  }
  const out = outFor(amount, rIn, rOut, feeOf(key));
  if (out < min_out) {
    near.panic("slippage: out < min_out");
  }
  near.storageSet("r:" + key + ":base", toStr(rIn + amount));
  near.storageSet("r:" + key + ":quote", toStr(rOut - out));
  const fee = (amount * feeOf(key)) / 10000;
  near.storageSet("fees", toStr(strToNum(near.storageGet("fees") ?? "0") + fee));
  addFlow(key, amount, out);
  // STRING return: promise results cross contracts as bytes — a bare i64
  // arrives little-endian and strToNum reads 0. Digits survive the hop.
  return toStr(out);
}
