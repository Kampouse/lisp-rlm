// Token — FT with allowances. ftTransferFrom is the NEP-141 pattern:
// spender (= predecessor, e.g. a vault contract calling cross-contract)
// moves `amount` from `from` to `to`, debiting allowance[from][spender].
const ZERO = 0n;

export function ftMint(to: string, amount: bigint): string {
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function ftBalanceOf(who: string): string {
  return "bal:" + (near.storageGet("ft:" + who) ?? ZERO);
}

export function ftIncreaseAllowance(spender: string, amount: bigint): string {
  let owner = near.predecessorAccountId();
  let key = "al:" + owner + ":" + spender;
  let cur = near.storageGet(key) ?? ZERO;
  near.storageSet(key, cur + amount);
  return "allowance:" + (cur + amount);
}

export function ftTransferFrom(from: string, to: string, amount: bigint): string {
  let spender = near.predecessorAccountId();
  if (spender == from) {
    near.abort("use ftTransfer for self-transfers");
  }
  let key = "al:" + from + ":" + spender;
  let allowed = near.storageGet(key) ?? ZERO;
  if (allowed < amount) {
    near.abort("insufficient allowance");
  }
  let balFrom = near.storageGet("ft:" + from) ?? ZERO;
  if (balFrom < amount) {
    near.abort("insufficient balance");
  }
  let balTo = near.storageGet("ft:" + to) ?? ZERO;
  near.storageSet(key, allowed - amount);
  near.storageSet("ft:" + from, balFrom - amount);
  near.storageSet("ft:" + to, balTo + amount);
  return "ok:" + amount;
}
