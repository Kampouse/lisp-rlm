// ─── arb_live.ts — on-chain arb brain: guarded cross-contract cycle ───
// One entry call fires a promise chain: swap A→B→…→A across DEX pools
// (examples/dex.ts), each leg carrying its own min_out slippage guard. The
// caller (off-chain scout) pushes expected per-leg rates at configure time;
// the contract verifies the expected cycle edge ITSELF before firing — a
// fabricated/moved market makes a guard trap and no value moves beyond the
// failed leg. Callbacks correlate via storage (leg1out/leg2out).
//
// Cycle shapes:
//   nlegs=3  triangle A→B→C→A      callbacks: onLeg1 → onLeg2 → onLeg3
//   nlegs=2  twin-pool A→B→A       callbacks: onLeg1 → onLeg2b (closer)
//
// NOTE: the demo dex moves pool reserves only (no per-account custody), so a
// mid-chain failure here strands nothing. With real token custody, a failed
// leg N leaves legs 1..N-1 executed (inventory held) — production needs an
// unwind/retry receipt on the fail path. Per-leg min_out bounds that loss.
//
// Frontend lowering constraint (found by probe): never nest an `if { return }`
// inside a block that continues after it — flat early-return ifs are fine.
//
// NOTE: exports are entry points, not callables — helpers are module-level.
// All amounts: fixed-point integer micro-units (10^-6). No floats.
//
// Exports:
//   configure({dex, c0,d0,r0, c1,d1,r1, c2,d2,r2, cap, tol_bps,
//              min_edge_bps, nlegs})   (r2 ignored when nlegs=2)
//   run()            → "FIRED:..." | "REFUSED:..."  (starts the chain)
//   onLeg1/onLeg2/onLeg2b/onLeg3   receipt callbacks (exported = valid entries)
//   status()         → bookkeeping view

const GAS: number = 20000000000000;
const CBGAS: number = 20000000000000;

function num(key: string): number {
  return strToNum(near.storageGet(key) ?? "0");
}

function legJson(pool: string, side: string, amount: number, minOut: number): string {
  return '{"key":"' + pool + '","side":"' + side + '","amount":' + toStr(amount) +
    ',"min_out":' + toStr(minOut) + "}";
}

export function configure(
  dex: string,
  c0: string, d0: string, r0: number,
  c1: string, d1: string, r1: number,
  c2: string, d2: string, r2: number,
  cap: number,
  tol_bps: number,
  min_edge_bps: number,
  nlegs: number,
): string {
  near.storageSet("dex", dex);
  near.storageSet("c0", c0); near.storageSet("d0", d0); near.storageSet("r0", toStr(r0));
  near.storageSet("c1", c1); near.storageSet("d1", d1); near.storageSet("r1", toStr(r1));
  near.storageSet("c2", c2); near.storageSet("d2", d2); near.storageSet("r2", toStr(r2));
  near.storageSet("cap", toStr(cap));
  near.storageSet("tol", toStr(tol_bps));
  near.storageSet("minedge", toStr(min_edge_bps));
  // 2-leg (twin-pool) or 3-leg (triangle) cycle
  near.storageSet("nlegs", toStr(nlegs));
  if (num("pos") == 0) {
    near.storageSet("pos", "0");
    near.storageSet("profit", "0");
    near.storageSet("runs", "0");
    near.storageSet("fails", "0");
  }
  return "configured:" + dex;
}

// expected leg out = in × (10⁶ - tol·100)/10⁶; rates are out×10⁶/in.
// Divide amount×rate FIRST — amount×rate×10⁶ overflows tagged ints (~2⁶⁰).
function guardedMin(amount: number, rate: number): number {
  const tol = num("tol");
  const expected = amount * rate / 1000000;
  return expected * (1000000 - tol * 100) / 1000000;
}

export function run(): string {
  const r0 = num("r0");
  const r1 = num("r1");
  // overflow guard (tagged ints cap near 2⁶⁰; refuse products beyond 10¹⁸)
  let safe0 = r0;
  if (safe0 == 0) {
    safe0 = 1;
  }
  if (r1 > 1000000000000000000 / safe0) {
    return "REFUSED:overflow";
  }
  const SC = 1000000;
  const r2 = num("r2");
  const half = r0 * r1 / SC;
  // 3-leg second product guard; 2-leg skips r2 entirely (bridge sends dummies)
  let sh = half;
  if (sh == 0) {
    sh = 1;
  }
  if (num("nlegs") != 2) {
    if (r2 > 1000000000000000000 / sh) {
      return "REFUSED:overflow";
    }
  }
  // in-contract economics gate: combined edge in millionths. 2-leg: r0·r1/SC;
  // 3-leg: ·r2/SC more. Integer-exact, no floats.
  let exp = half;
  if (num("nlegs") != 2) {
    exp = half * r2 / SC;
  }
  if (exp < SC + num("minedge") * 100) {
    return "REFUSED:edge:" + toStr(exp);
  }
  const cap = num("cap");
  const dex = near.storageGet("dex") ?? "";
  const min0 = guardedMin(cap, r0);
  near.storageSet("runs", toStr(num("runs") + 1));
  near.callAwait(dex, "swap", legJson(near.storageGet("c0") ?? "", near.storageGet("d0") ?? "", cap, min0),
    GAS, "onLeg1", CBGAS, "{}");
  return "FIRED:leg1";
}

export function onLeg1(): string {
  const res = near.promiseResult(0);
  if (res == "") {
    near.storageSet("fails", toStr(num("fails") + 1));
    return "FAIL:leg1";
  }
  near.storageSet("leg1out", toStr(strToNum(res)));
  const dex = near.storageGet("dex") ?? "";
  const in1 = strToNum(res);
  // 2-leg mode: this is the closing leg → profit guard (≥ cap) + onLeg2b;
  // 3-leg mode: tolerance guard + onLeg2. (Callback names must be literals.)
  let min1 = guardedMin(in1, num("r1"));
  if (num("nlegs") == 2) {
    min1 = num("cap");
    near.callAwait(dex, "swap", legJson(near.storageGet("c1") ?? "", near.storageGet("d1") ?? "", in1, min1),
      GAS, "onLeg2b", CBGAS, "{}");
    return "FIRED:onLeg2b";
  }
  near.callAwait(dex, "swap", legJson(near.storageGet("c1") ?? "", near.storageGet("d1") ?? "", in1, min1),
    GAS, "onLeg2", CBGAS, "{}");
  return "FIRED:leg2";
}

export function onLeg2(): string {
  const res = near.promiseResult(0);
  if (res == "") {
    near.storageSet("fails", toStr(num("fails") + 1));
    return "FAIL:leg2";
  }
  near.storageSet("leg2out", toStr(strToNum(res)));
  const dex = near.storageGet("dex") ?? "";
  const in2 = strToNum(res);
  const cap = num("cap");
  // final leg carries the profit guard: must return at least capital
  near.callAwait(dex, "swap", legJson(near.storageGet("c2") ?? "", near.storageGet("d2") ?? "", in2, cap),
    GAS, "onLeg3", CBGAS, "{}");
  return "FIRED:leg3";
}

// 2-leg closer: the swap already ran with min = cap, so a success is profit
export function onLeg2b(): string {
  const res = near.promiseResult(0);
  if (res == "") {
    near.storageSet("fails", toStr(num("fails") + 1));
    return "FAIL:leg2";
  }
  const in2 = strToNum(res);
  near.storageSet("leg2out", toStr(in2));
  const cap = num("cap");
  const profit = in2 - cap;
  if (profit >= 0) {
    near.storageSet("pos", toStr(in2));
    near.storageSet("profit", toStr(num("profit") + profit));
    return "OK:" + toStr(profit);
  }
  near.storageSet("fails", toStr(num("fails") + 1));
  near.storageSet("pos", toStr(in2));
  return "LOSS:" + toStr(profit);
}

export function onLeg3(): string {
  const res = near.promiseResult(0);
  if (res == "") {
    near.storageSet("fails", toStr(num("fails") + 1));
    return "FAIL:leg3";
  }
  const out3 = strToNum(res);
  const cap = num("cap");
  const profit = out3 - cap;
  if (profit < 0) {
    near.storageSet("fails", toStr(num("fails") + 1));
    near.storageSet("pos", toStr(out3));
    return "LOSS:" + toStr(profit);
  }
  near.storageSet("pos", toStr(out3));
  near.storageSet("profit", toStr(num("profit") + profit));
  return "OK:" + toStr(profit);
}

export function status(): string {
  return "pos:" + (near.storageGet("pos") ?? "0") +
    " profit:" + (near.storageGet("profit") ?? "0") +
    " runs:" + (near.storageGet("runs") ?? "0") +
    " fails:" + (near.storageGet("fails") ?? "0") +
    " leg1:" + (near.storageGet("leg1out") ?? "-") +
    " leg2:" + (near.storageGet("leg2out") ?? "-");
}
