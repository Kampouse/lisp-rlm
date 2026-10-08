// ft_ergonomics.ts — THE ergonomics-v2 acceptance example (2026-10-07).
// ftMint/ftBurn/ftKeys are the user-named target surface, verbatim.
// init/ftGet exist so the test can drive ownership + balance reads.

type Money = string;

export function init(): string {
  near.db.put("owner", near.predecessorAccountId());
  return "ok";
}

export function ftMint(params: { to: string, amount: Money }): string {
  assert(near.predecessorAccountId() == near.db.key("owner"), "ERR_NOT_OWNER");
  const cur = near.db.key("ft:" + params.to) ?? "0";
  near.db.put("ft:" + params.to, u128Add(cur, params.amount));
  near.event("ft_mint", { to: params.to, amount: params.amount });
  return "ok";
}

export function ftBurn(params: { from: string }): string {
  assert(near.db.has("ft:" + params.from), "ERR_NO_ACCOUNT");
  near.db.del("ft:" + params.from); // probed: has 1→0, key() → nil after
  return "ok";
}

export function ftKeys(): string {
  let out = "";
  for (const k of near.db.keys("ft:")) {
    out = out + k + ",";
  }
  return out;
}

export function ftGet(to: string): string {
  return near.db.key("ft:" + to) ?? "none";
}
