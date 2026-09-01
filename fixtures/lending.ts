// ── Lending v2 — u128 precision (yoctoNEAR, 10^24) ──
//
// All balance math is bigint: operators lower to the u128/* string family
// (limb math on both runtimes), so amounts never touch i64.
//   LTV 50% · 5% origination fee (ceiled — the protocol keeps the dust)

const LTV_BP = 5000n;   // 50% collateral factor
const FEE_BP = 500n;    // 5% origination fee
const SCALE = 10000n;
const ZERO = 0n;

export function deposit(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv2:" + who) ?? '{"dep":"0","bor":"0"}';
  let dep = acct.dep + amt;
  let next = jsonSet(acct, "dep", dep);
  near.storageSet("lv2:" + who, next);
  return next;
}

export function withdraw(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv2:" + who) ?? '{"dep":"0","bor":"0"}';
  let dep = acct.dep - amt;
  // remaining collateral must still cover the open borrow at LTV
  if (dep * LTV_BP < acct.bor * SCALE) {
    near.abort("withdraw would undercollateralize");
  }
  let next = jsonSet(acct, "dep", dep);
  near.storageSet("lv2:" + who, next);
  return next;
}

export function borrow(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv2:" + who) ?? '{"dep":"0","bor":"0"}';
  // debt = amt * (1 + fee), rounded UP — never lend the fee short
  let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
  let bor = acct.bor + add;
  if (acct.dep * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  let next = jsonSet(acct, "bor", bor);
  near.storageSet("lv2:" + who, next);
  return next;
}

export function repay(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv2:" + who) ?? '{"dep":"0","bor":"0"}';
  let bor = acct.bor - amt;
  if (bor < ZERO) {
    bor = ZERO;
  }
  let next = jsonSet(acct, "bor", bor);
  near.storageSet("lv2:" + who, next);
  return next;
}

// coverage in basis points: 10000+ = safe, <10000 = liquidatable
export function health(): string {
  let who = near.signerAccountId();
  let acct = near.storageGet("lv2:" + who) ?? '{"dep":"0","bor":"0"}';
  if (acct.bor == ZERO) {
    return "inf";
  }
  return (acct.dep * LTV_BP) / acct.bor;
}
