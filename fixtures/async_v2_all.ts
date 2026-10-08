// async v2 — near.all fanout: two PARALLEL views, ONE resume, joined in
// dependency order. Lowering target shape (identical to the hand-written
// portfolio_fanout.ts DAG): promise_create ×2 → promise_and → ONE
// promise_then → promise_return; resume reads promise_result(0)/(1).
const TOKEN_A = "toka.v2.test.near";
const TOKEN_B = "tokb.v2.test.near";

export async function portfolioBoth(user: string): string {
  const [a, b] = await near.all([
    near.call(TOKEN_A, "ftBalanceRaw", "{\"who\":\"" + user + "\"}", 20000000000000, 0),
    near.call(TOKEN_B, "ftBalanceRaw", "{\"who\":\"" + user + "\"}", 20000000000000, 0),
  ]);
  if (a == "" || b == "") {
    near.abort("a view failed");
  }
  return "both:" + u128Add(a, b);
}
