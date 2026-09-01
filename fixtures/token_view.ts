// Token + raw balance view (for cross-contract aggregation — returns a
// plain decimal string, no prefix, so u128 arithmetic can consume it).
const ZERO = 0n;

export function ftMint(to: string, amount: bigint): string {
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function ftBalanceRaw(who: string): string {
  return (near.storageGet("ft:" + who) ?? ZERO) + "";
}
