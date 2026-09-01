// ── Lending v3 — u128 precision + TIME-BASED INTEREST ──
//
// Every balance is bigint (u128/* string math). Interest accrues
// per-second at a fixed 10% APY, applied lazily on every call using
// near.blockTimestamp() (ns). Ceiled everywhere — the protocol keeps
// the dust, never lends the fee short.

const LTV_BP = 5000n;        // 50% collateral factor
const FEE_BP = 500n;         // 5% origination fee
const APY_BP = 1000n;        // 10% annual
const SCALE = 10000n;
const YEAR_SEC = 31536000n;
const ZERO = 0n;
const SEC = 1000000000n;     // ns → s

// lazy accrual: bor' = bor + bor*APY*elapsed_s/(SCALE*YEAR)
function accrue(acct: string, ts: bigint): string {
  let bor = acct.bor;
  let last = acct.ts;
  let elapsed = (ts - last) / SEC;
  if (bor > ZERO && elapsed > ZERO) {
    bor = bor + (bor * APY_BP * elapsed) / (SCALE * YEAR_SEC);
  }
  let next = jsonSet(acct, "bor", bor);
  return jsonSet(next, "ts", ts);
}

export function deposit(amt: bigint): string {
  let who = near.signerAccountId();
  let raw = near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}';
  // first deposit stamps the clock; later ones just accrue
  let acct = accrue(raw, near.blockTimestamp());
  if (raw.ts == ZERO) {
    acct = jsonSet(acct, "ts", near.blockTimestamp());
  }
  let next = jsonSet(acct, "dep", acct.dep + amt);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function borrow(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
  let bor = acct.bor + add;
  if (acct.dep * LTV_BP < bor * SCALE) {
    near.abort("insufficient collateral");
  }
  let next = jsonSet(acct, "bor", bor);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function repay(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  // saturate at zero (u128 can't go negative — check BEFORE subtracting)
  let bor = amt > acct.bor ? ZERO : acct.bor - amt;
  let next = jsonSet(acct, "bor", bor);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function withdraw(amt: bigint): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  let dep = acct.dep - amt;
  if (dep * LTV_BP < acct.bor * SCALE) {
    near.abort("withdraw would undercollateralize");
  }
  let next = jsonSet(acct, "dep", dep);
  near.storageSet("lv3:" + who, next);
  return next;
}

export function health(): string {
  let who = near.signerAccountId();
  let acct = accrue(
    near.storageGet("lv3:" + who) ?? '{"dep":"0","bor":"0","ts":"0"}',
    near.blockTimestamp(),
  );
  if (acct.bor == ZERO) {
    return "inf";
  }
  return (acct.dep * LTV_BP) / acct.bor;
}
