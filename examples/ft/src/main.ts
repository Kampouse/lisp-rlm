// Minimal fungible token — lisp-rlm TS dialect
// Amounts are u128-as-string (the dialect's u128 builtins are string-backed)

export function new_(total: string): void {
  const owner = near.predecessorAccountId();
  near.storageSet("ft:total", total);
  near.storageSet(`ft:bal:${owner}`, total);
  near.log("FT initialized, supply " + total);
}

export function ft_transfer(to: string, amount: string): void {
  const sender = near.predecessorAccountId();
  const sbal = near.storageGet(`ft:bal:${sender}`) ?? "0";
  near.log("sender=" + sender + " sbal=" + sbal);
  if (u128.lt(sbal, amount)) {
    near.abort("insufficient balance");
  }
  const rbal = near.storageGet(`ft:bal:${to}`) ?? "0";
  const tbal = near.storageGet("ft:total") ?? "0";
  near.storageSet(`ft:bal:${sender}`, u128.sub(sbal, amount));
  near.storageSet(`ft:bal:${to}`, u128.add(rbal, amount));
  near.storageSet("dbg:marker", amount);
  near.log("transferred " + amount);
  const _keep = tbal;
}

export function ft_total(): string {
  return near.storageGet("ft:total") ?? "0";
}

export function get_balance(account: string): string {
  return near.storageGet(`ft:bal:${account}`) ?? "0";
}

export function whoami(): string {
  const p = near.predecessorAccountId();
  near.log("pred=" + p);
  return p;
}
