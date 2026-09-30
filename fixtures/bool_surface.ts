// Bool-first-class surface tour (2026-09-30): TS boolean literals lower as
// real Bool values, `: boolean` annotations map to :: bool, and ternaries
// mixing bool expressions with literals unify. Every construct here FAILED
// or was impossible before the change (bool literals were Num 0/1).

// 1. the exact construct that motivated the change: ternary mixing a
//    .includes() (bool-typed) with a boolean literal branch
export function ternaryIncludes(hay: string, needle: string): string {
  const ok: boolean = hay.length > 0 ? hay.includes(needle) : false;
  return `${ok}`;
}

// 2. const boolean literal + truthiness
export function flagTruth(): string {
  const FLAG = true;
  const OFF = false;
  if (FLAG) {
    if (!OFF) { return "yes"; }
  }
  return "no";
}

// 3. annotated-boolean function returning literals, used in if / && / ||
//    (helper, not exported: exported fns take args from the tx input JSON
//    in NEAR mode, so in-contract calls go to unexported helpers)
function annBool(x: number): boolean {
  return x > 1;
}

export function annBoolRender(x: number): string {
  return `${annBool(x)}`;
}

export function annBoolUse(x: number): string {
  if (annBool(x) && true) { return "gt"; }
  if (annBool(x) || false) { return "still-gt"; }
  return "le";
}

// 4. boolean-to-boolean equality with literals (strict semantics)
export function eqBool(x: number): string {
  const t = annBool(x);
  if (t === true) { return "eq-true"; }
  if (t === false) { return "eq-false"; }
  return "none";
}

// 5. logic chains mixing literals, comparisons and string predicates
export function logicMix(hay: string): string {
  const a = hay.includes("a");
  if (a && true) { return "and"; }
  if (false || !a) { return "or"; }
  return "none";
}

// 6. boolean rendering in template interpolation (to-string of Bool)
export function boolRender(flag: boolean): string {
  return `flag=${flag}`;
}

// 7. comparison-driven ternary with literal branches
export function compareTern(a: number, b: number): string {
  const gt: boolean = a > b ? true : false;
  return `${gt}`;
}
