// FLASH POOL — the reentrancy-discipline pattern.
//   flashLoan(amount, borrower, callback) →
//     1. snapshot pool balance B0 (must cover amount)
//     2. transfer `amount` to the borrower (real value, batch transfer)
//     3. batch FUNCTION-CALL borrower.callback(amount)
//     4. then callback-settle on the pool: balance must be ≥ B0 + fee,
//        else ABORT — the WHOLE tx rolls back (atomicity: the transfer
//        and the borrow un-happen). State (fee pot) only survives on
//        honest repayment.
// The borrower (flashborrower.ts) accepts callbacks ONLY from the pool
// (predecessorAccountId guard) — the classic reentrancy firewall.
const GAS = 20000000000000;

export function deposit(): string {
  let d = near.attachedDepositU128();
  let bal = near.storageGet("bal") ?? "0";
  near.storageSet("bal", u128Add(bal, d));
  return "pool:" + u128Add(bal, d);
}

export function balance(): string {
  return near.storageGet("bal") ?? "0";
}

export function flashLoan(amount: string, borrower: string): string {
  let b0 = toStr(near.accountBalance());
  if (u128Lt(b0, amount)) {
    near.abort("insufficient pool");
  }
  let fee = u128Div(amount, "100"); // 1%
  let want = u128Add(amount, fee);
  near.storageSet("__fl:want", want);
  // 1. send the funds out
  let t = near.promiseBatchCreate(borrower);
  near.promiseBatchActionTransfer(t, amount);
  // 2. call the borrower's callback (it must repay `want` to us)
  let c = near.promiseBatchThen(t, borrower);
  near.promiseBatchActionFunctionCall(c, "onFlashLoan", "{\"amount\":\"" + amount + "\",\"fee\":\"" + fee + "\"}", "0", GAS);
  // 3. settle on ourselves — verifies the balance AFTER the callback
  let s = near.promiseBatchThen(c, near.currentAccountId());
  near.promiseBatchActionFunctionCall(s, "flashSettle", "{}", "0", GAS);
  near.promiseReturn(s);
  return "flashing";
}

export function flashSettle(): string {
  // only the pool itself may run the settle (it IS the callback target)
  if (near.predecessorAccountId() != near.currentAccountId()) {
    near.abort("settle: pool only");
  }
  let want = near.storageGet("__fl:want") ?? "0";
  let bal = toStr(near.accountBalance());
  if (u128Lt(bal, want)) {
    near.abort("flash loan not repaid");
  }
  near.storageSet("fees", u128Add(near.storageGet("fees") ?? "0", u128Sub(bal, want)));
  return "settled:" + bal;
}
