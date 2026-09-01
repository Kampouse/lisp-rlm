// ── Lending v4 — u128 + interest + LIQUIDATIONS ──
//
// Single-asset, full-precision (yocto). 10% APY accrues lazily on every
// call. When health < LIQ_LINE (10000 bp), anyone EXCEPT the borrower
// can repay up to half the debt and seize collateral at LIQ_BONUS_BP
// (5%) over par. Close factor 50%. All math integer u128.

const LTV_BP = 5000n;        // 50% collateral factor
const FEE_BP = 500n;         // 5% origination fee
const APY_BP = 1000n;        // 10% annual
const SCALE = 10000n;
const YEAR_SEC = 31536000n;
const ZERO = 0n;
const SEC = 1000000000n;
const LIQ_LINE = 10000n;     // health below this = liquidatable
const LIQ_BONUS_BP = 500n;   // liquidator pays 1.00, receives 1.05
const TWO = 2n;

function accrue(acct: string, ts: bigint): string {
  let bor = acct.bor;
  let elapsed = (ts - acct.ts) / SEC;
  if (bor > ZERO && elapsed > ZERO) {
    bor = bor + (bor * APY_BP * elapsed) / (SCALE * YEAR_SEC);
  }
  let next = jsonSet(acct, "bor", bor);
  return jsonSet(next, "ts", ts);
}

function acct(): string {
  let who = near.signerAccountId();
  return accrue(
    near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
}

function save(acct: string): string {
  let who = near.signerAccountId();
  near.storageSet("lv4:" + who, acct);
  return acct;
}

export function deposit(amt: bigint): string {
  let who = near.signerAccountId();
  let raw = near.storageGet("lv4:" + who) ?? '{"dep":"0","bor":"0","ts":"0","own":""}';
  let a = accrue(raw, near.blockTimestamp());
  let next = jsonSet(a, "dep", a.dep + amt);
  if (raw.ts == ZERO) {
    // first deposit: stamp the owner (ts already stamped by accrue —
    // checking the POST-accrue ts made this branch unreachable)
    next = jsonSet(next, "own", who);
  }
  near.storageSet("lv4:" + who, next);
  return next;
}

export function borrow(amt: bigint): string {
  let a = acct();
  let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
  let bor = a.bor + add;
  if (a.dep * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  return save(jsonSet(a, "bor", bor));
}

export function repay(amt: bigint): string {
  let a = acct();
  let bor = amt > a.bor ? ZERO : a.bor - amt;   // saturate BEFORE subtract
  return save(jsonSet(a, "bor", bor));
}

// liquidator (signer) repays `amt` of `victim`'s debt and seizes
// collateral at a 5% bonus. Victim's account key, liquidator's tx.
export function liquidate(victim: string, amt: bigint): string {
  let a = accrue(
    near.storageGet("lv4:" + victim) ?? '{"dep":"0","bor":"0","ts":"0","own":""}',
    near.blockTimestamp(),
  );
  if (a.bor == ZERO) {
    near.abort("nothing to liquidate");
  }
  if (a.dep * LTV_BP >= a.bor * SCALE) {
    near.abort("account healthy");
  }
  if (near.signerAccountId() == a.own) {
    near.abort("cannot liquidate yourself");
  }
  if (amt * TWO > a.bor) {
    near.abort("close factor 50%");
  }
  let seize = (amt * (SCALE + LIQ_BONUS_BP)) / SCALE;
  if (seize > a.dep) {
    near.abort("collateral exhausted");
  }
  let next = jsonSet(a, "bor", a.bor - amt);
  next = jsonSet(next, "dep", a.dep - seize);
  near.storageSet("lv4:" + victim, next);
  return next;
}

export function health(): string {
  let a = acct();
  if (a.bor == ZERO) {
    return "inf";
  }
  return (a.dep * LTV_BP) / a.bor;
}
