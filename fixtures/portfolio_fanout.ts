// Portfolio aggregator — PARALLEL cross-contract fan-out:
//   portfolioTotal(user) fires ftBalanceRaw on BOTH token contracts via
//   promise_and, attaches one resume callback, promise_returns the DAG.
//   The resume reads promise_result(0) + promise_result(1) (flattened in
//   dep order), u128-adds them, stores + returns the total.
const TOK_A = "toka.pf.test.near";
const TOK_B = "tokb.pf.test.near";
const GAS = 20000000000000;
const ZERO = 0n;

export function portfolioTotal(user: string): string {
  near.storageSet("__pf:user", user);
  let pa = near.promiseCreate(TOK_A, "ftBalanceRaw", "{\"who\":\"" + user + "\"}", GAS, 0);
  let pb = near.promiseCreate(TOK_B, "ftBalanceRaw", "{\"who\":\"" + user + "\"}", GAS, 0);
  let both = near.promiseAnd(pa, pb);
  let cb = near.promiseThen(both, near.currentAccountId(), "portfolioTotal__resume", "{}", GAS, 0);
  near.promiseReturn(cb);
  return "fired";
}

export function portfolioTotal__resume(): string {
  let user = near.storageGet("__pf:user") ?? "";
  let ra = near.promiseResult(0);
  let rb = near.promiseResult(1);
  if (ra == "" || rb == "") {
    near.abort("a token view failed");
  }
  let total = u128Add(ra, rb);
  near.storageSet("pf:" + user, total);
  return "total:" + total;
}
