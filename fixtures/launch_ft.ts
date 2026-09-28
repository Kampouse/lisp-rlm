// Stub NEP-141 FT — launched per token by launchpad.ts (2026-09-27).
// deploy(codeHash) stands in for global-contract adoption; mint credits
// the launchpad; transfers use bigint lattice math (lending.ts pattern).
// Balances are decimal strings in storage under "bal:<acct>".

const ZERO: bigint = 0n;

export function deploy(codeHash: string): string {
  near.log(strCat("ft: adopted code ", codeHash));
  return codeHash;
}

export function mint(to: string, amt: bigint): void {
  let k = strCat("bal:", to);
  near.storageSet(k, (near.storageGet(k) ?? ZERO) + amt);
  near.storageSet("sup", (near.storageGet("sup") ?? ZERO) + amt);
}

export function ft_balance_of(account_id: string): string {
  return near.storageGet(strCat("bal:", account_id)) ?? "0";
}

export function ft_total_supply(): string {
  return near.storageGet("sup") ?? "0";
}

export function ft_transfer(receiver_id: string, amount: bigint): void {
  let sender = near.predecessorAccountId();
  let k = strCat("bal:", sender);
  let cur = near.storageGet(k) ?? ZERO;
  if (amount > cur) {
    near.abort("insufficient balance");
  }
  near.storageSet(k, cur - amount);
  let rk = strCat("bal:", receiver_id);
  near.storageSet(rk, (near.storageGet(rk) ?? ZERO) + amount);
}

export function ft_transfer_call(receiver_id: string, amount: bigint, msg: string): void {
  let sender = near.predecessorAccountId();
  let k = strCat("bal:", sender);
  let cur = near.storageGet(k) ?? ZERO;
  if (amount > cur) {
    near.abort("insufficient balance");
  }
  near.storageSet(k, cur - amount);
  // JSON msg → Ref swap instruction (passthrough; mainnet Ref parses same)
  near.log(strCat("ft: forwarded to ", receiver_id, " msg=", msg));
}
