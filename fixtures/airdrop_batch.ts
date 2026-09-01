// NEAR airdrop — BATCH PROMISES + REAL TRANSFERS:
//   airdrop(r1, r2, r3, amt) builds one transfer batch PER receiver
//   (a batch = one receipt to one account), joins with promise_and,
//   attaches ONE resume that verifies the action-result count and
//   records the drop. Transfers carry REAL value: the airdropper's
//   NEAR balance (funded by the receipt's attached deposit) is debited.
const GAS = 20000000000000;

export function airdrop(r1: string, r2: string, r3: string, amt: string): string {
  near.storageSet("__ad:amt", amt);
  let b1 = near.promiseBatchCreate(r1);
  near.promiseBatchActionTransfer(b1, amt);
  let b2 = near.promiseBatchCreate(r2);
  near.promiseBatchActionTransfer(b2, amt);
  let b3 = near.promiseBatchCreate(r3);
  near.promiseBatchActionTransfer(b3, amt);
  let all = near.promiseAnd(b1, b2, b3);
  let cb = near.promiseBatchThen(all, near.currentAccountId());
  near.promiseBatchActionFunctionCall(cb, "airdrop__resume", "{}", "0", GAS);
  near.promiseReturn(cb);
  return "fired";
}

export function airdrop__resume(): string {
  let n = near.promiseResultsCount();
  if (n != 3) {
    near.abort("expected 3 receipts");
  }
  // fail-closed via the STATUS probe: Successful(empty) transfers are
  // real NEAR shape; promiseSucceeded distinguishes them from Failed.
  // Any failure aborts — this resume's writes revert while the successful
  // sibling transfers KEEP their committed value (receipt atomicity).
  let i = 0;
  while (i < n) {
    if (near.promiseSucceeded(i) == 0) {
      near.abort("a transfer receipt failed");
    }
    i = i + 1;
  }
  let amt = near.storageGet("__ad:amt") ?? "0";
  let total = u128Mul(amt, "3");
  near.storageSet("ad:total", total);
  return "dropped:" + total;
}

// — mixed batch: 2 function_calls in ONE batch to the same account —
export function mintTwice(minter: string, to: string, amt: string): string {
  let b = near.promiseBatchCreate(minter);
  near.promiseBatchActionFunctionCall(b, "ftMint", "{\"to\":\"" + to + "\",\"amount\":" + "\"" + amt + "\"}", "0", GAS);
  near.promiseBatchActionFunctionCall(b, "ftMint", "{\"to\":\"" + to + "\",\"amount\":" + "\"" + amt + "\"}", "0", GAS);
  let cb = near.promiseBatchThen(b, near.currentAccountId());
  near.promiseBatchActionFunctionCall(cb, "mintTwice__resume", "{}", "0", GAS);
  near.promiseReturn(cb);
  return "fired";
}

export function mintTwice__resume(): string {
  let n = near.promiseResultsCount();
  if (n != 2) {
    near.abort("expected 2 receipts");
  }
  return "mints:" + toStr(n);
}
