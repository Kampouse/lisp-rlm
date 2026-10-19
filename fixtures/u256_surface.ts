// u256 decimal-string surface — differential fixture for test_ts_u256.rs.
// All expected values are computed independently (Python BigInt oracle in
// the commit message / test assertions below).

export function mulBig(a: string, b: string): string {
  return u256Mul(a, b);
}

export function divBig(a: string, b: string): string {
  return u256Div(a, b);
}

export function shr64(a: string): string {
  return u256Shr(a, 64);
}

export function shr1(a: string): string {
  return u256Shr(a, 1);
}

export function shrK(a: string, k: number): string {
  return u256Shr(a, k);
}

export function isqrt(a: string): string {
  return u256Isqrt(a);
}

export function cmpKind(a: string, b: string): string {
  if (u256Lt(a, b)) {
    return "lt";
  }
  if (u256Gt(a, b)) {
    return "gt";
  }
  return "eq";
}

export function eqZero(a: string): string {
  return u256Eq(a, "0") ? "zero" : "nonzero";
}

// The CLMM walk step that motivated the whole module: L·Δ >> 64 where
// L·Δ overflows u128. L = 1e26, Δ = 18441593515559200.
export function walkStep(): string {
  const L: string = u128Mul("100000000000000000000000000", "1");
  const d: string = u128Sub(
    "18448588474025873616",
    "18446744073709551616"
  );
  const prod: string = u256Mul(L, d);
  return u256Shr(prod, 64);
}

// add: exercise every limb-boundary carry (t* uses add+shl).
export function add2(a: string, b: string): string {
  return u256Add(a, b);
}

export function shlK(a: string, k: number): string {
  return u256Shl(a, k);
}
