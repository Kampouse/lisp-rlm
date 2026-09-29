// Reference XYK implementation — independent of the contract, BigInt-exact.
// Contract semantics (projects/launchpad/pool.ts):
//   buy : gross = floor(pt * in / (pn + in));  fee = floor(gross * bps / 10000)
//         net = gross - fee → trader;  pn' = pn + in;  pt' = pt - gross
//   sell: gross = floor(pn * in / (pt + in));  fee = floor(gross * bps / 10000)
//         net = gross - fee → trader;  pt' = pt + in;  pn' = pn - gross
// BigInt division IS floor — matches the contract's u128 long division
// (bounds: reserves ≤ 1e30 → remainder*10 ≤ 1e31 ≪ u128::MAX, no drift).

export const BPS_DEFAULT = 100n; // pool default fee = 1%

export function xykGrossBuy(pt: bigint, pn: bigint, inAmt: bigint): bigint {
  return (pt * inAmt) / (pn + inAmt);
}
export function xykGrossSell(pn: bigint, pt: bigint, inAmt: bigint): bigint {
  return (pn * inAmt) / (pt + inAmt);
}
export function feeOn(amount: bigint, bps: bigint = BPS_DEFAULT): bigint {
  return (amount * bps) / 10000n;
}

export type PoolState = { near: bigint; tokens: bigint; fees_tok: bigint; fees_near: bigint };
export type PadState = { near: bigint; tokens: bigint };

export function applyBuy(s: PoolState, pad: PadState, inAmt: bigint, bps = BPS_DEFAULT) {
  const gross = xykGrossBuy(s.tokens, s.near, inAmt);
  const fee = feeOn(gross, bps);
  const net = gross - fee;
  return {
    gross, fee, net,
    pool: { near: s.near + inAmt, tokens: s.tokens - gross, fees_tok: s.fees_tok + fee, fees_near: s.fees_near },
    pad: { near: pad.near - inAmt, tokens: pad.tokens + net }, // attach-0 (gas-key) buy
    ftPad: { near: pad.near, tokens: pad.tokens },             // classic buy pays FT, not pad
  };
}

export function applySell(s: PoolState, pad: PadState, inAmt: bigint, bps = BPS_DEFAULT) {
  const gross = xykGrossSell(s.near, s.tokens, inAmt);
  const fee = feeOn(gross, bps);
  const net = gross - fee;
  return {
    gross, fee, net,
    pool: { near: s.near - gross, tokens: s.tokens + inAmt, fees_tok: s.fees_tok, fees_near: s.fees_near + fee },
    pad: { near: pad.near + net, tokens: pad.tokens - inAmt }, // internal sell credits the pad
  };
}
