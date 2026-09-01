// @ts-nocheck — dialect async (V1 resume-continuations) is not Promise-based
// Vault — cross-contract depositor. deposit() fires an async
// ftTransferFrom on the token (the vault is the spender; the USER's
// allowance is debited), then the auto-generated continuation reads
// the promise result and credits the deposit. V1 async: await must be
// the first statement.
const ZERO = 0n;
const TOKEN = "token.cc.test.near";

export async function deposit(user: string, amount: bigint): string {
  const res = await near.call(
    TOKEN,
    "ftTransferFrom",
    "{\"from\":\"" + user + "\",\"to\":\"" + near.currentAccountId() + "\",\"amount\":" + amount + "}",
    20000000000000,
    0
  );
  if (res == "") {
    near.abort("token transfer failed");
  }
  let cur = near.storageGet("vd:" + user) ?? ZERO;
  near.storageSet("vd:" + user, cur + amount);
  return "deposited:" + (cur + amount);
}

export function getTotalDeposits(who: string): string {
  return "deposits:" + (near.storageGet("vd:" + who) ?? ZERO);
}
