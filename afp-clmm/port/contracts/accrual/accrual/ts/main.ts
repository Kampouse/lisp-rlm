// Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
// accruef Lf c []       = 0
// accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
// Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).
function sget(k: string): string {
  return near.storageGet(k) ?? "0";
}
function sput(k: string, v: string): void {
  near.storageSet(k, v);
}
function fail(m: string): void {
  near.log(m);
  near.abort(m);
}
function bal(u: string): string {
  return sget("a:bal:" + u);
}
function charged(u: string): string {
  return sget("a:charged:" + u);
}
function credit(u: string, a: string): void {
  sput("a:bal:" + u, u128.add(bal(u), a));
}
// divFloor64 a b = (a * b) div 2^64, truncating (floor on nat).
// Mirrors Isabelle div_floor64: owed side rounds DOWN, fail-closed.
// Same 2^64 literal as the LISP twin and the .thy.
function divFloor64(a: string, b: string): string {
  return u128MulDiv(a, b, "18446744073709551616");
}

// Cons step of the owed fold: owed(g, c) = (g - c) * Lf g div 2^64.
// Checkpoint a:ck:<u> carries the fold's c across calls (monotone;
// g < c aborts ERR_ORDER = accruef's precondition). Scenario steps are
// the list gs. String layer only (u128.*), like the LISP twin.
export function accrueStep(): string {
  const g = near.jsonGetStr("g") ?? "0";
  const l = near.jsonGetStr("l") ?? "0";
  const u = near.predecessorAccountId();
  const ckKey = "a:ck:" + u;
  const c = sget(ckKey);
  if (u128.lt(g, c)) fail("ERR_ORDER");
  const d = u128.sub(g, c);
  const term = divFloor64(d, l);
  credit(u, term);
  sput("a:charged:" + u, u128.add(charged(u), term));
  sput(ckKey, g);
  return term;
}
