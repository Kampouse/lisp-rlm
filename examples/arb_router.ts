// ─── arb_router.ts — custody arb brain over an atomic router ───
// Same in-contract economics gate as arb_live.ts (verify-before-fire),
// but the fire is ONE callAwait into exchange.swap_router: all hops settle
// in a single receipt against the brain's own ledger balance — panic on any
// hop reverts everything (real-custody atomicity; the demo dex chain could
// strand forward-settled legs). The callback only books the result.
//
// Rates: raw out×10⁶/in (millionths) at executed size, scouted off-chain.
// Fixed-point integers only; products bounded ≤ 10¹⁸ (tagged-int range).
//
// configure(dex, c0,d0,r0, c1,d1,r1, c2,d2,r2, cap, tol_bps, min_edge_bps, nlegs)
//   c2/d2/r2 ignored in 2-leg mode (bridge sends dummies). nlegs = 2|3.
// run() → gate → exchange.swap_router(...) → onLeg2 (books, guards again).

// expected final-hop out = in×rate/10⁶, tolerance shaved; divide FIRST —
// amount×rate×10⁶ overflows tagged ints (~2⁶⁰).
function guardedMin(amount: number, rate: number): number {
  const tol = num("tol");
  const exp = amount * rate / 1000000;
  return exp * (10000 - tol) / 10000;
}

function num(key: string): number {
  return strToNum(near.storageGet(key) ?? "0");
}

export function configure(
  dex: string,
  c0: string, d0: string, r0: number,
  c1: string, d1: string, r1: number,
  c2: string, d2: string, r2: number,
  cap: number, tol_bps: number, min_edge_bps: number, nlegs: number
): string {
  near.storageSet("dex", dex);
  near.storageSet("c0", c0);
  near.storageSet("d0", d0);
  near.storageSet("r0", toStr(r0));
  near.storageSet("c1", c1);
  near.storageSet("d1", d1);
  near.storageSet("r1", toStr(r1));
  near.storageSet("c2", c2);
  near.storageSet("d2", d2);
  near.storageSet("r2", toStr(r2));
  near.storageSet("cap", toStr(cap));
  near.storageSet("tol", toStr(tol_bps));
  near.storageSet("minedge", toStr(min_edge_bps));
  near.storageSet("nlegs", toStr(nlegs));
  return "configured:" + dex;
}

export function run(): string {
  const cap = num("cap");
  const SC = 1000000;
  const r0 = num("r0");
  const r1 = num("r1");
  const r2 = num("r2");
  // overflow guards: bound every product ≤ 10¹⁸ (tagged ints cap near 2⁶⁰)
  let s0 = r0;
  if (s0 == 0) {
    s0 = 1;
  }
  let s1 = r1;
  if (s1 > 1000000000000000000 / s0) {
    s1 = 1000000000000000000 / s0;
  }
  const half = s0 * s1 / SC;
  let sh = half;
  if (sh == 0) {
    sh = 1;
  }
  let exp = half;
  if (num("nlegs") != 2) {
    let s2 = r2;
    if (s2 > 1000000000000000000 / sh) {
      s2 = 1000000000000000000 / sh;
    }
    exp = sh * s2 / SC;
  }
  const threshold = 1000000 + num("minedge") * 100;
  if (exp < threshold) {
    near.storageSet("runs", toStr(num("runs") + 1));
    return "REFUSED:edge:" + toStr(exp);
  }
  // per-hop min_amount_out: each hop's expected OUT shaved by tolerance,
  // final hop pinned at cap (profit guard). min_total = cap.
  const tol = num("tol");
  const e1 = cap * s0 / SC;
  const e2 = e1 * s1 / SC;
  const m0 = e1 * (10000 - tol) / 10000;
  let m1 = e2 * (10000 - tol) / 10000;
  if (num("nlegs") == 2) {
    m1 = cap;
  }
  const m2 = cap;
  const dex = near.storageGet("dex") ?? "";
  // string params (ids, sides) come straight out of storage as strings
  const C0 = near.storageGet("c0") ?? "";
  const D0 = near.storageGet("d0") ?? "";
  const C1 = near.storageGet("c1") ?? "";
  const D1 = near.storageGet("d1") ?? "";
  const C2 = near.storageGet("c2") ?? "";
  const D2 = near.storageGet("d2") ?? "";
  near.storageSet("runs", toStr(num("runs") + 1));
  if (num("nlegs") == 2) {
    near.callAwait(dex, "swap_router", "{\"c0\":\"" + C0 + "\",\"d0\":\"" + D0 + "\",\"a0\":" + toStr(cap) + ",\"m0\":" + toStr(m0)
      + ",\"c1\":\"" + C1 + "\",\"d1\":\"" + D1 + "\",\"a1\":" + toStr(e1) + ",\"m1\":" + toStr(cap)
      + ",\"c2\":\"\",\"d2\":\"\",\"a2\":0,\"m2\":0,\"min_total\":" + toStr(cap) + "}",
      20000000000000, "onLeg2b", 20000000000000, "{}");
  } else {
    near.callAwait(dex, "swap_router", "{\"c0\":\"" + C0 + "\",\"d0\":\"" + D0 + "\",\"a0\":" + toStr(cap) + ",\"m0\":" + toStr(m0)
      + ",\"c1\":\"" + C1 + "\",\"d1\":\"" + D1 + "\",\"a1\":" + toStr(e1) + ",\"m1\":" + toStr(m1)
      + ",\"c2\":\"" + C2 + "\",\"d2\":\"" + D2 + "\",\"a2\":" + toStr(e2) + ",\"m2\":" + toStr(cap)
      + ",\"min_total\":" + toStr(cap) + "}",
      20000000000000, "onLeg2", 20000000000000, "{}");
  }
  return "FIRED";
}

// book: the router already enforced per-hop mins; here we only ask whether
// the cycle cleared capital. promiseResult is fail-closed "" → strToNum
// reads 0 → LOSS path.
export function onLeg2(): string {
  const res = near.promiseResult(0);
  const amt = strToNum(res);
  const cap = num("cap");
  if (amt < cap) {
    near.storageSet("fails", toStr(num("fails") + 1));
    near.storageSet("pos", toStr(amt));
    return "LOSS:" + res;
  }
  const profit = amt - cap;
  near.storageSet("pos", toStr(amt));
  near.storageSet("profit", toStr(num("profit") + profit));
  return "OK:" + toStr(profit);
}

// 2-leg booking (same body, distinct literal callback name)
export function onLeg2b(): string {
  const res = near.promiseResult(0);
  const amt = strToNum(res);
  const cap = num("cap");
  if (amt < cap) {
    near.storageSet("fails", toStr(num("fails") + 1));
    near.storageSet("pos", toStr(amt));
    return "LOSS:" + res;
  }
  const profit = amt - cap;
  near.storageSet("pos", toStr(amt));
  near.storageSet("profit", toStr(num("profit") + profit));
  return "OK:" + toStr(profit);
}

export function status(): string {
  return "pos:" + toStr(num("pos")) + " profit:" + toStr(num("profit"))
    + " runs:" + toStr(num("runs")) + " fails:" + toStr(num("fails"));
}
