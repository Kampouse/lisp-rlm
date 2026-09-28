// ─── On-chain arbitrage executor + bandit tuner (fixed-point, integer IR) ───
// Two internal constant-product pools with drifted prices. A cycle is
// A→p0→B→p1→A. executeCycle is FLASH-shaped: quote first, apply writes only
// if profit ≥ minProfit, else leave ALL state untouched (atomic revert on
// chain; here via check-effects pattern, which is the honest skeleton).
// arbStep(eps) = ε-greedy bandit over input-size bands, reward = realized
// profit, Q persisted in storage — same learner as examples/qlearn.ts.
// NOTE: helpers are non-exported — exports are entry points (near-mock ABI),
// internal calls only resolve to module-level fns.

// ---- pools (integer "micro" token units, fee in basis points) ----
export function initPools(): number {
  near.storageSet("p0:rA", "1000000");
  near.storageSet("p0:rB", "1000000");
  near.storageSet("p0:fee", "30"); // 0.30%
  near.storageSet("p1:rA", "1000000");
  near.storageSet("p1:rB", "900000"); // drifted: B cheap on p1 → cycle live
  near.storageSet("p1:fee", "30");
  near.storageSet("treasury", "1000000"); // flash capital
  near.storageSet("fees", "0");
  near.storageSet("profit", "0");
  near.storageSet("rng", "1");
  return 10;
}

// External shock: reseeds BOTH pools to the canonical drifted state
// (simulates the market re-opening the cycle; treasury/history untouched)
export function drift(): number {
  near.storageSet("p0:rA", "1000000");
  near.storageSet("p0:rB", "1000000");
  near.storageSet("p1:rA", "1400000");
  near.storageSet("p1:rB", "800000");
  return 1;
}

// constant-product out given in (A→B on p0, or B→A on p1).
// NOTE (2026-09-17): pool id is a STRING prefix ("p0"/"p1") — numeric ids
// made storage keys "0:rA" while init wrote "p0:rA" (silent all-zero reads).
function getAmountOut(pk: string, ain: number): number {
  const rA = strToNum(near.storageGet(pk + ":rA") ?? "0");
  const rB = strToNum(near.storageGet(pk + ":rB") ?? "0");
  const fee = strToNum(near.storageGet(pk + ":fee") ?? "30");
  let rin = rA;
  let rout = rB;
  if (pk == "p1") {
    rin = rB;
    rout = rA;
  }
  const ainFee = ain - (ain * fee) / 10000;
  return (ainFee * rout) / (rin + ainFee);
}

// Full cycle quote: A →(p0) B →(p1) A, net of flash fee (10 bps)
function quoteCycle(ain: number): number {
  const bOut = getAmountOut("p0", ain);
  const aOut = getAmountOut("p1", bOut);
  const repay = ain + (ain * 10) / 10000;
  return aOut - repay;
}

// The state-mutating leg: apply both swaps + book profits. Caller MUST
// have validated profit first (check-effects discipline).
function applyCycle(ain: number): number {
  const bOut = getAmountOut("p0", ain);
  const aOut = getAmountOut("p1", bOut);
  const profit = quoteCycle(ain);
  const flashFee = (ain * 10) / 10000;
  near.storageSet("p0:rA", toStr(strToNum(near.storageGet("p0:rA") ?? "0") + ain));
  near.storageSet("p0:rB", toStr(strToNum(near.storageGet("p0:rB") ?? "0") - bOut));
  near.storageSet("p1:rB", toStr(strToNum(near.storageGet("p1:rB") ?? "0") + bOut));
  near.storageSet("p1:rA", toStr(strToNum(near.storageGet("p1:rA") ?? "0") - aOut));
  near.storageSet("treasury", toStr(strToNum(near.storageGet("treasury") ?? "0") + profit));
  near.storageSet("fees", toStr(strToNum(near.storageGet("fees") ?? "0") + flashFee));
  near.storageSet("profit", toStr(strToNum(near.storageGet("profit") ?? "0") + profit));
  return profit;
}

// Atomic execute: quote → guard → apply. On guard fail: zero writes.
export function executeCycle(ain: number, minProfit: number): string {
  const profit = quoteCycle(ain);
  if (profit < minProfit) {
    return "REVERT:" + toStr(profit);
  }
  return "OK:" + toStr(applyCycle(ain));
}

// ---- bandit: arms = input-size bands ----
// Q["arm:i"] in centi-reward (reward = profit / 10)
export function arbStep(eps: number): string {
  const sizes: number[] = [1000, 10000, 40000];
  // greedy arm
  let best = 0;
  let bv = -1;
  let i = 0;
  while (i < 3) {
    const v = strToNum(near.storageGet("arm:" + i) ?? "0");
    if (v > bv) {
      bv = v;
      best = i;
    }
    i = i + 1;
  }
  // ε-greedy via RNG counter
  const n = strToNum(near.storageGet("rng") ?? "0") + 1;
  near.storageSet("rng", toStr(n));
  let arm = best;
  if ((n * 7) % 100 < eps) {
    arm = n % 3;
  }
  const ain = sizes[arm];
  const profit = quoteCycle(ain);
  let r = 0;
  if (profit > 0) {
    applyCycle(ain);
    r = profit / 10; // centi-reward
  }
  // Q-update (skipped opportunities leave Q unchanged)
  const k = "arm:" + arm;
  const q = strToNum(near.storageGet(k) ?? "0");
  near.storageSet(k, toStr((q * 3 + r) / 4)); // α=0.25; decays to 0, never stalls
  return "arm:" + toStr(arm) + " ain:" + toStr(ain) + " r:" + toStr(r);
}

export function stats(): string {
  let out = "treasury:" + (near.storageGet("treasury") ?? "?");
  out = out + " profit:" + (near.storageGet("profit") ?? "?");
  out = out + " fees:" + (near.storageGet("fees") ?? "?");
  let i = 0;
  while (i < 3) {
    const k = "arm:" + i;
    out = out + " Q[" + toStr(i) + "]:" + (near.storageGet(k) ?? "0");
    i = i + 1;
  }
  return out;
}
