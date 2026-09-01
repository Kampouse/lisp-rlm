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

// ── Allowances (NEP-141 approve/transferFrom) ──
// Key "fta:<owner>:<spender>". NEP-141 race rule: changing a nonzero
// allowance to another nonzero value must pass through zero.

function allowanceKey(owner: string, spender: string): string {
  return "fta:" + owner + ":" + spender;
}

export function ftApprove(spender: string, amount: bigint): string {
  let who = near.signerAccountId();
  let key = allowanceKey(who, spender);
  let cur = near.storageGet(key) ?? 0n;
  if (cur != 0n && amount != 0n) {
    near.abort("reset allowance to zero first");
  }
  near.storageSet(key, amount);
  return "ok";
}

export function ftAllowance(owner: string, spender: string): string {
  return near.storageGet(allowanceKey(owner, spender)) ?? 0n;
}

export function ftTransferFrom(from: string, to: string, amount: bigint): string {
  let who = near.signerAccountId();
  let aKey = allowanceKey(from, who);
  let allowed = near.storageGet(aKey) ?? 0n;
  let bal = near.storageGet("ft:" + from) ?? 0n;
  if (allowed < amount || bal < amount) {
    near.abort("allowance or balance too low");
  }
  let toBal = near.storageGet("ft:" + to) ?? 0n;
  near.storageSet(aKey, allowed - amount);
  near.storageSet("ft:" + from, bal - amount);
  near.storageSet("ft:" + to, toBal + amount);
  return "ok";
}
