// Stub Ref Finance router — JSON-only surface used by launchpad.ts
// graduation (2026-09-27). Records pools + deposits in storage so the
// mock test can assert the graduation actually landed.

export function storage_deposit(account_id: string, registration_only: boolean): string {
  near.storageSet(strCat("reg:", account_id), "1");
  return "registered";
}

export function create_pool(token_a: string, token_b: string, fee: number): string {
  let cur = near.storageGet("pool_count") ?? "0";
  let pid = toStr(strToNum(cur) + 1);
  near.storageSet("pool_count", pid);
  let pool = strCat(
    "{\"id\":\"", pid,
    "\",\"a\":", jsonQuote(token_a),
    ",\"b\":\"", token_b,
    "\",\"fee\":", toStr(fee), "}",
  );
  near.storageSet(strCat("pool:", pid), pool);
  return pid;
}

export function register_tokens(tokens: string): string {
  near.storageSet("last_registered", tokens);
  return "ok";
}

export function deposit(token: string, amount: string): string {
  let who = near.predecessorAccountId();
  let k = strCat("res:", who, ":", token);
  near.storageSet(k, u128Add(near.storageGet(k) ?? "0", amount));
  return "deposited";
}

export function get_pool(id: string): string {
  return near.storageGet(strCat("pool:", id)) ?? "";
}
