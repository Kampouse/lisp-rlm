// Logical VALUE semantics (2026-10-04): `a || b` / `a && b` now return the
// OPERAND VALUE (JS semantics) instead of coerced 0/1 booleans. Each
// left operand is let-bound → evaluated AT MOST once; short-circuit
// preserved. Truthiness is exactly what `if (x)` uses today (tag-aware:
// false / nil / 0 falsy; strings — including "" — truthy, the documented
// M2 boundary that mirrors `if (s)`).

// 1. number fallback defaults — the motivating pattern
export function orDefault(a: number, b: number): number {
  return a || b;
}

// 2. && value semantics on numbers
export function andGuard(a: number, b: number): number {
  return a && b;
}

// 3. both operands bool → still boolean values (compat with bool surface)
function annBool(x: number): boolean { return x > 1; }
export function boolChain(x: number): string {
  return `${annBool(x) && true}|${annBool(x) || false}`;
}

// 4. string defaults: "" stays TRUTHY here — same behavior `if (s)` has
// today (string truthiness is the M2 boundary; do NOT expect JS falsy "").
// Result bracket-wrapped: a bare `""` return prints NO 📄 line in near-mock
// (empty value_return), so the empty-string win must be made visible.
export function strOr(s: string, d: string): string {
  return `[${s || d}]`;
}

// 5. left operand evaluated exactly ONCE (side effect via storage bump),
//    even though it's a call and the result is falsy
function bumpAndZero(): number {
  const cur = near.storageGet("__lv:c") ?? "0";
  near.storageSet("__lv:c", toStr(strToNum(cur) + 1));
  return 0; // falsy → right operand wins, bump must have run exactly once
}
export function onceLeft(): number {
  return bumpAndZero() || 42;
}

// 6. && short-circuit: right operand NOT evaluated when left is falsy
function bumpAndNine(): number {
  const cur = near.storageGet("__lv:c") ?? "0";
  near.storageSet("__lv:c", toStr(strToNum(cur) + 1));
  return 9;
}
export function onceRight(a: number): number {
  return a && bumpAndNine();
}

// counter reader for the single-eval assertions (exported: mock-callable)
export function readC(): number {
  return strToNum(near.storageGet("__lv:c") ?? "0");
}

// 7. logical values inside if-conditions keep boolean behavior
export function guardCall(x: number): string {
  if (x > 0 && x < 100) { return "in"; }
  return "out";
}

// 8. chains: a || b || c (left-assoc — value flows through)
export function orChain(a: number, b: number, c: number): number {
  return a || b || c;
}
export function andChain(a: number, b: number, c: number): number {
  return a && b && c;
}
