// MSIG TARGET — a tiny counter the multisig governs. Records WHO called
// it (predecessorAccount): direct users are rejected; only the multisig
// contract may bump — governance-enforced state.
export function bump(step: string): string {
  let caller = near.predecessorAccountId();
  if (caller != "msig.b.test.near") {
    near.abort("only the multisig may bump");
  }
  let c = near.storageGet("count") ?? "0";
  c = u128Add(c, step);
  near.storageSet("count", c);
  near.storageSet("last-caller", caller);
  return "count:" + c;
}

export function getCount(): string {
  return near.storageGet("count") ?? "0";
}
