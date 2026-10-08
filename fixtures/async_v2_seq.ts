// async v2 — TWO awaits in sequence, pre-await statement, per-resume binding.
// depositTwice views the user's balance on tok2 AND the contract's own
// balance, joining them in the last resume. The "pre:" marker written
// BEFORE the first await proves pre-await statements run in the entry.
const TOKEN = "tok2.v2.test.near";

export async function depositTwice(user: string): string {
  near.storageSet("v2:mark", "pre:" + user);
  const r1 = await near.call(TOKEN, "ftBalanceRaw", "{\"who\":\"" + user + "\"}", 20000000000000, 0);
  const r2 = await near.call(TOKEN, "ftBalanceRaw", "{\"who\":\"" + TOKEN + "\"}", 20000000000000, 0);
  if (r1 == "" || r2 == "") {
    near.abort("view failed");
  }
  let mark = near.storageGet("v2:mark") ?? "";
  return "twice:" + u128Add(r1, r2) + ":" + mark;
}
