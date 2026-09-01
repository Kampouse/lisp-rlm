// AUCTION HOUSE — deposits, height deadlines, refund chains, fee split.
//   list(item, minBid, endHeight)  → id                (seller opens)
//   bid(id)                        → outbid REFUND via batch transfer
//     · reads THIS receipt's attached deposit (near.attachedDeposit,
//       the u128 decimal-string form)
//     · must beat current bid (≥ min, strictly greater)
//     · previous bidder refunded by a transfer batch
//   settle(id)   after blockIndex() ≥ end: pays seller (amt − 2.5% fee
//     via u128 math), storage.remove's the record, marks the ledger.
//   getAuction(id) → view
const GAS = 20000000000000;
const REC0 = '{"seller":"","item":"","min":"0","amt":"0","bidder":"","end":"0"}';

export function list(item: string, minBid: string, endHeight: string): string {
  let seller = near.signerAccountId();
  let id = near.storageGet("a:count") ?? "0";
  id = u128Add(id, "1");
  let rec = jsonSet(REC0, "seller", seller);
  rec = jsonSet(rec, "item", item);
  rec = jsonSet(rec, "min", minBid);
  rec = jsonSet(rec, "end", endHeight);
  near.storageSet("a:" + id, rec);
  near.storageSet("a:count", id);
  return "auction:" + id;
}

export function bid(id: string): string {
  let who = near.signerAccountId();
  let dep = near.attachedDepositU128();
  let rec = near.storageGet("a:" + id) ?? REC0;
  if (rec.min == "" && rec.amt == "") {
    near.abort("no such auction");
  }
  if (!u128Lt(toStr(near.blockIndex()), rec.end)) {
    near.abort("auction closed");
  }
  if (u128Lt(dep, rec.min)) {
    near.abort("below minimum");
  }
  if (rec.bidder != "") {
    if (!u128Gt(dep, rec.amt)) {
      near.abort("must outbid");
    }
    // ── refund the previous bidder: REAL value, batch transfer ──
    let r = near.promiseBatchCreate(rec.bidder);
    near.promiseBatchActionTransfer(r, rec.amt);
    near.promiseReturn(r);
  }
  rec = jsonSet(rec, "amt", dep);
  rec = jsonSet(rec, "bidder", who);
  near.storageSet("a:" + id, rec);
  return "bid:" + dep;
}

export function settle(id: string): string {
  let rec = near.storageGet("a:" + id) ?? REC0;
  if (rec.amt == "") {
    near.abort("no such auction");
  }
  if (u128Lt(toStr(near.blockIndex()), rec.end)) {
    near.abort("not closed yet");
  }
  if (rec.bidder == "") {
    // no bids: record a pass, remove the auction
    storage.del("a:" + id);
    near.storageSet("s:" + id, "PASSED");
    return "passed";
  }
  // 2.5% fee: amt*25/1000 — u128 decimal-string math
  let fee = u128Div(u128Mul(rec.amt, "25"), "1000");
  let net = u128Sub(rec.amt, fee);
  // stash for the resume (the airdrop pattern — args ride storage)
  near.storageSet("__st:id", id);
  near.storageSet("__st:net", net);
  near.storageSet("__st:fee", fee);
  let pay = near.promiseBatchCreate(rec.seller);
  near.promiseBatchActionTransfer(pay, net);
  let house = near.promiseBatchCreate(near.currentAccountId());
  near.promiseBatchActionTransfer(house, fee);
  let both = near.promiseAnd(pay, house);
  let cb = near.promiseBatchThen(both, near.currentAccountId());
  near.promiseBatchActionFunctionCall(cb, "settle__resume", "{}", "0", GAS);
  near.promiseReturn(cb);
  return "settling";
}

export function settle__resume(): string {
  let n = near.promiseResultsCount();
  if (n != 2) {
    near.abort("expected 2 payouts");
  }
  let i = 0;
  while (i < n) {
    if (near.promiseSucceeded(i) == 0) {
      near.abort("a payout failed");
    }
    i = i + 1;
  }
  let id = near.storageGet("__st:id") ?? "";
  let net = near.storageGet("__st:net") ?? "0";
  let fee = near.storageGet("__st:fee") ?? "0";
  storage.del("a:" + id);
  near.storageSet("s:" + id, "SOLD:" + net + ":" + fee);
  return "sold";
}

export function getAuction(id: string): string {
  return near.storageGet("a:" + id) ?? (near.storageGet("s:" + id) ?? "none");
}
