// token.ts — the pre-deployed global FT contract (minimal, ~8 KB)
//
// Deployed ONCE as a global contract; launches adopt it via
// use_global_contract, then initialize with new_().
// Storage: u128 money ledger (near/storage), plain strings.

function balKey(a: string): string { return "b:" + a; }

// init: mints total_supply to owner
export function new_(): number {
  if ((near.storageGet("ok") ?? "") != "") { near.abort("ERR_INIT"); return 0; }
  const ownerId = near.jsonGetStr("owner_id") ?? "";
  const totalSupply = near.jsonGetStr("total_supply") ?? "0";
  if (strLength(ownerId) == 0 || u128IsZero(totalSupply)) { near.abort("ERR_ARGS"); return 0; }
  near.storageSet("supply", totalSupply);
  near.storageSet(balKey(ownerId), totalSupply);
  near.storageSet("ok", "1");
  return 0;
}

export function ftTransfer(): number {
  const to = near.jsonGetStr("receiver_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const from = near.predecessorAccountId();
  const fromBal = near.storageGet(balKey(from)) ?? "0";
  if (u128Lt(fromBal, amount)) { near.abort("ERR_INSUFFICIENT"); return 0; }
  near.storageSet(balKey(from), u128Sub(fromBal, amount));
  near.storageSet(balKey(to), u128Add(near.storageGet(balKey(to)) ?? "0", amount));
  return 0;
}

export function storageDeposit(): number {
  const who = near.jsonGetStr("account_id") ?? near.predecessorAccountId();
  if ((near.storageGet(balKey(who)) ?? "") == "") { near.storageSet(balKey(who), "0"); }
  return 0;
}

export function ftBalanceOf(): string {
  return near.storageGet(balKey(near.jsonGetStr("account_id") ?? "")) ?? "0";
}
export function ftTotalSupply(): string { return near.storageGet("supply") ?? "0"; }