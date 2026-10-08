// THE base example (2026-10-07 ergonomics pitch) — verbatim shape.
// Probes the three pitch promises: array destructuring of near.all,
// object args auto-quoted, Promise<Money> annotation.
type Money = string;
const TOKS = ["toka.v2.test.near", "tokb.v2.test.near"];
const GAS = 20000000000000;

export async function portfolioTotal(user: string): Promise<Money> {
  const [a, b] = await near.all([
    near.call(TOKS[0], "ftBalanceRaw", { who: user }, GAS, "0"),
    near.call(TOKS[1], "ftBalanceRaw", { who: user }, GAS, "0"),
  ]);
  return "total:" + u128Add(a, b);
}
