// ── Fungible Token (NEP-141-ish subset) — TS surface ──
// Storage: "ft:<account>" → u128 decimal string, "ft:supply" → string,
// "ft:own" → deployer. First caller becomes owner (ownable-lite).
// No NEP-141 receiver callback semantics — plain balance ledger.

const ZERO = 0n;

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

export function ftTransfer(to: string, amount: bigint): string {
  let who = near.signerAccountId();
  let from = near.storageGet("ft:" + who) ?? ZERO;
  if (from < amount) {
    near.abort("insufficient balance");
  }
  let toBal = near.storageGet("ft:" + to) ?? ZERO;
  near.storageSet("ft:" + who, from - amount);
  near.storageSet("ft:" + to, toBal + amount);
  return "ok";
}

export function ftBalanceOf(who: string): string {
  return near.storageGet("ft:" + who) ?? ZERO;
}

export function ftBurn(amount: bigint): string {
  let who = near.signerAccountId();
  let from = near.storageGet("ft:" + who) ?? ZERO;
  if (from < amount) {
    near.abort("insufficient balance");
  }
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + who, from - amount);
  near.storageSet("ft:supply", supply - amount);
  return "supply:" + (supply - amount);
}
