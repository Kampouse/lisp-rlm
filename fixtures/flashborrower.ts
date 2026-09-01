// FLASH BORROWER — takes flash loans, does "arb", repays with fee.
// The predecessor guard accepts callbacks ONLY from the pool. A `stiff`
// mode refuses to repay — the pool's settle aborts and the WHOLE tx
// (transfer out + borrow) rolls back atomically.
export function deposit(): string {
  let d = near.attachedDepositU128();
  let bal = near.storageGet("bal") ?? "0";
  near.storageSet("bal", u128Add(bal, d));
  return "borrower:" + u128Add(bal, d);
}

export function onFlashLoan(amount: string, fee: string): string {
  if (near.predecessorAccountId() != "pool.c.test.near") {
    near.abort("callback: pool only");
  }
  if ((near.storageGet("stiff") ?? "0") == "1") {
    // rogue mode: keep the money, never repay — settle must kill the tx
    return "stiffed";
  }
  // "arbitrage profit" — just marker state + repay amount+fee
  near.storageSet("last-borrow", amount);
  let repay = u128Add(amount, fee);
  let t = near.promiseBatchCreate("pool.c.test.near");
  near.promiseBatchActionTransfer(t, repay);
  near.promiseReturn(t);
  return "repaid:" + repay;
}

export function goStiff(): string {
  near.storageSet("stiff", "1");
  return "stiff-on";
}

export function honest(): string {
  near.storageSet("stiff", "0");
  return "stiff-off";
}

export function lastBorrow(): string {
  return near.storageGet("last-borrow") ?? "none";
}
