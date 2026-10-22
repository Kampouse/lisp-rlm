// CLMM v5 core math — compilable-TS port of the lisp p5-dy/dx family.
// Contract mirror (up-move conventions, S2 ≥ S assumed — same as lisp):
//   dy  = L·(S2−S) >> 64
//   dx  = L·(S2−S) // (S·S2)
//   dyi/dxo = the down-move twins (args swapped by the caller).
// All grid values S(i) = isqrt(1.0001^i·2^128) fit u128 for |i| ≤ 800000
// (max S ≈ 4.3e36 < 2^128), so the subtraction is exact in the u128
// family and both u256/mul operands stay in domain.
//
// Differential gate: scripts/clmm_v5_core_ts_parity.py (byte-exact vs
// the lisp kernel across randomized grid pairs, |i| ≤ 800000).
//
// NOT ported here (needs more surface):
//   G(i)  — the 1.0001^i pow chain squares values ≥ 2^128; u256/mul is
//           u128×u128 only. Needs a wide-mul op (256×256→512) or lisp-side
//           grid feeding.
//   t*    — needs u256 add/shl (2^64·(dy+1) − 1): no u256 add/sub/shl yet.

export function tsdy(L: string, S: string, S2: string): string {
  const d: string = u128Sub(S2, S);
  return u256Shr(u256Mul(L, d), 64);
}

export function tsdyi(L: string, S: string, S2: string): string {
  const d: string = u128Sub(S, S2);
  return u256Shr(u256Mul(L, d), 64);
}

export function tsdx(L: string, S: string, S2: string): string {
  const d: string = u128Sub(S2, S);
  const ss: string = u256Mul(S, S2);
  // price-bridge renormalization (v5-dx!: ×2^64 before the divide). Use
  // u256Shl, not a second mul: u256Mul is u128×u128 only — the product
  // L·d can exceed u128 and would truncate.
  return u256Div(u256Shl(u256Mul(L, d), 64), ss);
}

// t* — the exact dy-solver constant: ((dy+1)·2^64 − 1) // L.
// Uses the identity (dy+1)·2^64 − 1 = (dy << 64) + (2^64 − 1), folding the
// subtraction into an additive constant — no u256 sub needed.

export function tstar(dy: string, L: string): string {
  const num: string = u256Add(u256Shl(dy, 64), "18446744073709551615");
  return u256Div(num, L);
}

export function tsdxo(L: string, S: string, S2: string): string {
  const d: string = u128Sub(S, S2);
  const ss: string = u256Mul(S, S2);
  // price-bridge renormalization (same shl-64 as tsdx)
  return u256Div(u256Shl(u256Mul(L, d), 64), ss);
}
