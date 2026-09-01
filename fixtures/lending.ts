// ── Lending v1 — single-asset, collateralized, all-integer math ──
//
// Model: deposit the native token as collateral, borrow against it.
//   LTV 50% (500 bp) · 5% (500 bp) origination fee · balances in u64
// Interest accrual is per-action in v1 (time-based needs block_ts, v2).

const LTV_BP = 5000;  // 50% collateral factor (basis points)
const FEE_BP = 500;   // 5% origination fee on borrows
const SCALE = 10000;

export function deposit(amt: number): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv1:" + who) ?? '{"dep":0,"bor":0}';
  let dep = strToNum(acct.dep) + amt;
  let next = jsonSet(acct, "dep", toStr(dep));
  near.storageSet("lv1:" + who, next);
  return next;
}

export function withdraw(amt: number): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv1:" + who) ?? '{"dep":0,"bor":0}';
  let dep = strToNum(acct.dep) - amt;
  // remaining collateral must still cover the open borrow at LTV
  if (dep * LTV_BP < strToNum(acct.bor) * SCALE) {
    near.abort("withdraw would undercollateralize");
  }
  let next = jsonSet(acct, "dep", toStr(dep));
  near.storageSet("lv1:" + who, next);
  return next;
}

export function borrow(amt: number): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv1:" + who) ?? '{"dep":0,"bor":0}';
  // debt grows by amt + fee, all in integer basis points
  let add = (amt * (SCALE + FEE_BP)) / SCALE;
  let bor = strToNum(acct.bor) + add;
  if (strToNum(acct.dep) * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  let next = jsonSet(acct, "bor", toStr(bor));
  near.storageSet("lv1:" + who, next);
  return next;
}

export function repay(amt: number): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv1:" + who) ?? '{"dep":0,"bor":0}';
  let bor = strToNum(acct.bor) - amt;
  if (bor < 0) {
    bor = 0;
  }
  let next = jsonSet(acct, "bor", toStr(bor));
  near.storageSet("lv1:" + who, next);
  return next;
}

// health = collateral coverage, 10000 = exactly at LTV limit
export function health(): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv1:" + who) ?? '{"dep":0,"bor":0}';
  let bor = strToNum(acct.bor);
  if (bor == 0) {
    return "inf";
  }
  let h = (strToNum(acct.dep) * LTV_BP * SCALE) / (bor * SCALE);
  return toStr(h);
}
