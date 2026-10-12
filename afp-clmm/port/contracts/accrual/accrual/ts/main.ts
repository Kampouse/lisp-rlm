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
function debit(u: string, a: string): void {
  if (u128.lt(bal(u), a)) fail("ERR_FUNDS");
  sput("a:bal:" + u, u128.sub(bal(u), a));
}
function u128MulDiv(a: string, b: string, d: string): string {
  // (a * b) / d, truncating; d != 0
  if (d === "0") fail("ERR_DIV0");
  const A = BigInt(a), B = BigInt(b), D = BigInt(d);
  return ((A * B) / D).toString();
}
function u128Shr64(a: string): string {
  // a div 2^64  (exact transcription of `div 2^64` in accruef)
  return (BigInt(a) >> 64n).toString();
}

// owed fold: checkpoint-threaded sum of (g - c) * Lf g div 2^64.
// gs: per-op growth deltas (charged book entries, Aggregate_Book);
// ls: per-op liquidity Lf; c: starting checkpoint (<= every g, else ERR_ORDER).
// Returns owed as string; checkpoint advances to last g.
function accrue(gs: string[], ls: string[], c: string): string {
  let ck = BigInt(c);
  let acc = 0n;
  for (let i = 0; i < gs.length; i++) {
    const g = BigInt(gs[i]);
    if (g < ck) fail("ERR_ORDER");
    acc += BigInt(divFloor64((g - ck).toString(), ls[i]));
    ck = g;
  }
  return acc.toString();
}

// entry: accrue { gs: "g1,g2,...", ls: "l1,l2,...", c: "ck", user: "alice" }
// charges owed into the caller's cell; preserves owed <= charged.
export function accrue_entry(arg: string): string {
  const o = JSON.parse(arg);
  const gs: string[] = String(o.gs).split(",");
  const ls: string[] = String(o.ls).split(",");
  if (gs.length !== ls.length) fail("ERR_LEN");
  const u = String(o.user);
  const owed = accrue(gs, ls, String(o.c));
  credit(u, owed);
  sput("a:charged:" + u, u128.add(charged(u), owed));
  return owed;
}
