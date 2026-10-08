// THE base example (2026-10-07 ergonomics pitch) — verbatim shape.
// @ts-nocheck — d.ts surface is loose (near.all typing, object args);
// the lisp-rlm compiler is the authority for these fixtures.
// Probes the pitch promises: array destructuring of near.all, object
// args auto-quoted, NEAR-native amount types (Yocto = yoctoNEAR u128
// decimal string; Amount = NEP-141 raw token amount).
type Yocto = string;
type Amount = string;
const TOKS = ["toka.v2.test.near", "tokb.v2.test.near"];
const GAS = 20000000000000;

// Return-contract check (2026-10-08): "total:" + x is CONCAT — display
// text, not an amount — so the honest annotation is Promise<string>.
// Promise<Amount> would be rejected: the u128 value must be returned raw.
export async function portfolioTotal(user: string): Promise<string> {
  const [a, b] = await near.all([
    near.call(TOKS[0], "ftBalanceRaw", { who: user }, GAS, "0"),
    near.call(TOKS[1], "ftBalanceRaw", { who: user }, GAS, "0"),
  ]);
  return "total:" + u128Add(a, b);
}

// Yocto side: deposit gate + transfer (yoctoNEAR domain). Non-async —
// deposit/transfer are synchronous; awaits are for cross-contract reads.
export function echoDeposit(): Yocto {
  const d: Yocto = near.attachedDepositU128();
  if (near.depositGte(1000000000000000000000000n)) {
    near.transferU128("carol.v2.test.near", d);
  }
  return d;
}
