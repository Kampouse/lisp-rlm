// FT + lending desk in one contract. The FT module owns "ft:<who>"
// balances + "ft:supply". The lending module locks collateral into
// "lt:<who>" and tracks debt under "ld:<who>" — same token both sides,
// so no price feed needed: borrow power = locked * 5000 / 10000 (50%).

const ZERO = 0n;
const LTV = 5000n;      // 50% of locked value
const SCALE = 10000n;

export function ftMint(to: string, amount: bigint): string {
  if ((near.storageGet("ft:own") ?? "") == "") {
    near.storageSet("ft:own", near.signerAccountId());
  }
  if (near.signerAccountId() != (near.storageGet("ft:own") ?? "")) {
    near.abort("only the owner may mint");
  }
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function ftBalanceOf(who: string): string {
  return near.storageGet("ft:" + who) ?? ZERO;
}

// ── Lending desk ──
function collateralOf(who: string): bigint {
  return near.storageGet("lt:" + who) ?? ZERO;
}
function debtOf(who: string): bigint {
  return near.storageGet("ld:" + who) ?? ZERO;
}
function maxBorrowOf(who: string): bigint {
  return collateralOf(who) * LTV / SCALE;
}

export function lendDeposit(amount: bigint): string {
  let who = near.signerAccountId();
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  if (bal < amount) {
    near.abort("insufficient balance");
  }
  let locked = collateralOf(who);
  near.storageSet("ft:" + who, bal - amount);
  near.storageSet("lt:" + who, locked + amount);
  return "locked:" + (locked + amount);
}

export function lendBorrow(amount: bigint): string {
  let who = near.signerAccountId();
  let debt = debtOf(who);
  let cap = maxBorrowOf(who);
  if (debt + amount > cap) {
    near.abort("would exceed borrow cap");
  }
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ld:" + who, debt + amount);
  near.storageSet("ft:" + who, bal + amount);
  return "debt:" + (debt + amount);
}

export function lendRepay(amount: bigint): string {
  let who = near.signerAccountId();
  let debt = debtOf(who);
  if (debt < amount) {
    near.abort("repay exceeds debt");
  }
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  if (bal < amount) {
    near.abort("insufficient balance");
  }
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ld:" + who, debt - amount);
  near.storageSet("ft:" + who, bal - amount);
  near.storageSet("ft:supply", supply - amount);
  return "debt:" + (debt - amount);
}

export function lendWithdraw(amount: bigint): string {
  let who = near.signerAccountId();
  let locked = collateralOf(who);
  if (locked < amount) {
    near.abort("withdraw exceeds locked");
  }
  let debt = debtOf(who);
  let cap = (locked - amount) * LTV / SCALE;
  if (debt > cap) {
    near.abort("would undercollateralize");
  }
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("lt:" + who, locked - amount);
  near.storageSet("ft:" + who, bal + amount);
  return "locked:" + (locked - amount);
}

export function lendHealth(): string {
  let who = near.signerAccountId();
  let cap = maxBorrowOf(who);
  let debt = debtOf(who);
  return "cap:" + cap + " debt:" + debt;
}
