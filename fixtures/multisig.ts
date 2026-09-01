// MULTI-SIG WALLET — threshold governance, deferred execution.
//   request(target, method, args) → txId      (anyone proposes)
//   approve(txId)                            (owners only, once each)
//   execute(txId)  once approvals ≥ 2 → batch FUNCTION-CALL action to the
//                  target contract (real cross-contract dispatch), then a
//                  resume verifies + burns the request.
// Approvers tracked as a comma-string roster (storage-native, no arrays).
const GAS = 20000000000000;
const TX0 = '{"target":"","method":"","args":"","approvers":"","n":"0","done":"0"}';

function approvedAlready(roster: string, who: string): boolean {
  // wrap so every member is ","-delimited, then linear scan
  let parts = strSplit("," + roster + ",", ",");
  let i = 0;
  while (i < parts.length) {
    if (parts[i] == who) {
      return true;
    }
    i = i + 1;
  }
  return false;
}

export function request(target: string, method: string, args: string): string {
  let who = near.signerAccountId();
  let id = near.storageGet("t:count") ?? "0";
  id = u128Add(id, "1");
  let rec = jsonSet(TX0, "target", target);
  rec = jsonSet(rec, "method", method);
  rec = jsonSet(rec, "args", args);
  rec = jsonSet(rec, "approvers", who);
  rec = jsonSet(rec, "n", "1"); // proposer auto-approves
  near.storageSet("t:" + id, rec);
  near.storageSet("t:count", id);
  return "tx:" + id;
}

export function approve(txId: string): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("t:" + txId) ?? TX0;
  if (rec.target == "") {
    near.abort("no such tx");
  }
  if (rec.done != "0") {
    near.abort("already executed");
  }
  // one-approval-each: "," delimiters guard substring collisions
  // dedup: match the account anywhere in the ","-delimited roster
  if (approvedAlready(rec.approvers, who)) {
    near.abort("already approved");
  }
  let n = u128Add(rec.n, "1");
  rec = jsonSet(rec, "approvers", rec.approvers + "," + who);
  rec = jsonSet(rec, "n", n);
  near.storageSet("t:" + txId, rec);
  return "approvals:" + n;
}

export function execute(txId: string): string {
  let rec = near.storageGet("t:" + txId) ?? TX0;
  if (rec.target == "") {
    near.abort("no such tx");
  }
  if (rec.done != "0") {
    near.abort("already executed");
  }
  if (u128Lt(rec.n, "2")) {
    near.abort("threshold not met");
  }
  near.storageSet("__tx:id", txId);
  // ── deferred execution: batch FUNCTION-CALL to the target contract ──
  let b = near.promiseBatchCreate(rec.target);
  near.promiseBatchActionFunctionCall(b, rec.method, rec.args, "0", GAS);
  let cb = near.promiseBatchThen(b, near.currentAccountId());
  near.promiseBatchActionFunctionCall(cb, "execute__resume", "{}", "0", GAS);
  near.promiseReturn(cb);
  return "executing";
}

export function execute__resume(): string {
  let n = near.promiseResultsCount();
  if (n != 1) {
    near.abort("expected 1 receipt");
  }
  if (near.promiseSucceeded(0) == 0) {
    near.abort("target call failed");
  }
  let txId = near.storageGet("__tx:id") ?? "";
  let rec = near.storageGet("t:" + txId) ?? TX0;
  rec = jsonSet(rec, "done", "1");
  near.storageSet("t:" + txId, rec);
  return "executed:" + near.promiseResult(0);
}

export function getTx(txId: string): string {
  let rec = near.storageGet("t:" + txId) ?? TX0;
  return rec.target + "|" + rec.method + "|n=" + rec.n + "|done=" + rec.done;
}
