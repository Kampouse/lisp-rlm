// Atomic swap: Alice locks asset-A against H=sha256(secret); Bob locks
// asset-B against the same H; Alice reveals the secret to take B; Bob
// uses the revealed secret to take A. Bob's claim window ends at the
// MIDPOINT of the remaining time, so Alice can never refund her A while
// Bob's B is still claimable (the classic swap safety property).
//
// Keys: s:<id> record, sh:<id> hashlock, sr:<id> revealed secret,
//       fa:<who> / fb:<who> token balances (two namespaces).
// States: A → BOTH → B_CLAIMED → DONE, with refund side-exits.

const ZERO = 0n;
const NANOS = 1000000000n;
function geTs(now: bigint, tl: bigint): bigint {
  if (now >= tl) { return 1n; }
  return 0n;
}

function ltTs(now: bigint, tl: bigint): bigint {
  if (now < tl) { return 0n; }
  return 1n;
}

function midTl(now: bigint, tlA: bigint): bigint {
  return now + (tlA - now) / 2n;
}

const REC0 = '{"init":"","resp":"","amtA":"0","amtB":"0","tlA":"0","tlB":"0","state":""}';

function mintTo(ns: string, to: string, amount: bigint): string {
  let bal = near.storageGet(ns + ":" + to) ?? ZERO;
  let supply = near.storageGet(ns + ":supply") ?? ZERO;
  near.storageSet(ns + ":" + to, bal + amount);
  near.storageSet(ns + ":supply", supply + amount);
  return "supply:" + (supply + amount);
}

export function faMint(to: string, amount: bigint): string {
  return mintTo("fa", to, amount);
}

export function fbMint(to: string, amount: bigint): string {
  return mintTo("fb", to, amount);
}

export function faBalanceOf(who: string): string {
  return near.storageGet("fa:" + who) ?? ZERO;
}

export function fbBalanceOf(who: string): string {
  return near.storageGet("fb:" + who) ?? ZERO;
}

export function swapNew(amountA: bigint, amountB: bigint, timeoutSec: bigint, secret: string): string {
  let init = near.signerAccountId();
  let bal = near.storageGet("fa:" + init) ?? ZERO;
  if (bal < amountA) {
    near.abort("insufficient A balance");
  }
  let id = near.storageGet("s:count") ?? ZERO;
  id = id + 1n;
  let tlA = near.blockTimestamp() + timeoutSec * NANOS;
  let rec = jsonSet(REC0, "init", init);
  rec = jsonSet(rec, "amtA", amountA);
  rec = jsonSet(rec, "amtB", amountB);
  rec = jsonSet(rec, "tlA", tlA);
  rec = jsonSet(rec, "state", "A");
  near.storageSet("s:" + id, rec);
  near.storageSet("sh:" + id, near.sha256Hash(secret));
  near.storageSet("s:count", id);
  near.storageSet("fa:" + init, bal - amountA);
  return "swap:" + id;
}

// Responder (Bob) locks his B. Only while state A. His claim deadline is
// the midpoint between now and Alice's tlA.
export function swapLockB(id: bigint): string {
  let resp = near.signerAccountId();
  let rec = near.storageGet("s:" + id) ?? REC0;
  if (rec.state != "A") {
    near.abort("not in state A");
  }
  if (geTs(near.blockTimestamp(), rec.tlA) != ZERO) {
    near.abort("timed out");
  }
  let bal = near.storageGet("fb:" + resp) ?? ZERO;
  if (bal < rec.amtB) {
    near.abort("insufficient B balance");
  }
  let tlB = midTl(near.blockTimestamp(), rec.tlA);
  let next = jsonSet(rec, "resp", resp);
  next = jsonSet(next, "tlB", tlB);
  next = jsonSet(next, "state", "BOTH");
  near.storageSet("s:" + id, next);
  near.storageSet("fb:" + resp, bal - rec.amtB);
  return "locked:" + tlB;
}

// Initiator (Alice) reveals the secret and takes B — before tlB.
export function swapClaimB(id: bigint, secret: string): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("s:" + id) ?? REC0;
  if (rec.state != "BOTH") {
    near.abort("not in state BOTH");
  }
  if (who != rec.init) {
    near.abort("only the initiator may claim B");
  }
  if (near.sha256Hash(secret) != (near.storageGet("sh:" + id) ?? "")) {
    near.abort("wrong secret");
  }
  if (geTs(near.blockTimestamp(), rec.tlB) != ZERO) {
    near.abort("B window closed");
  }
  let bal = near.storageGet("fb:" + who) ?? ZERO;
  near.storageSet("fb:" + who, bal + rec.amtB);
  near.storageSet("sr:" + id, secret);
  near.storageSet("s:" + id, jsonSet(rec, "state", "B_CLAIMED"));
  return "claimedB:" + rec.amtB;
}

// Responder (Bob) takes A with the revealed secret — before tlA.
export function swapClaimA(id: bigint, secret: string): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("s:" + id) ?? REC0;
  if (rec.state != "B_CLAIMED") {
    near.abort("not in state B_CLAIMED");
  }
  if (who != rec.resp) {
    near.abort("only the responder may claim A");
  }
  if (near.sha256Hash(secret) != (near.storageGet("sh:" + id) ?? "")) {
    near.abort("wrong secret");
  }
  if (geTs(near.blockTimestamp(), rec.tlA) != ZERO) {
    near.abort("A window closed");
  }
  let bal = near.storageGet("fa:" + who) ?? ZERO;
  near.storageSet("fa:" + who, bal + rec.amtA);
  near.storageSet("s:" + id, jsonSet(rec, "state", "DONE"));
  return "claimedA:" + rec.amtA;
}

// Initiator refund: only if Bob never locked (state A), after tlA.
export function swapRefundA(id: bigint): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("s:" + id) ?? REC0;
  if (rec.state != "A") {
    near.abort("not in state A");
  }
  if (who != rec.init) {
    near.abort("only the initiator may refund");
  }
  if (ltTs(near.blockTimestamp(), rec.tlA) == ZERO) {
    near.abort("not yet timed out");
  }
  let bal = near.storageGet("fa:" + who) ?? ZERO;
  near.storageSet("fa:" + who, bal + rec.amtA);
  near.storageSet("s:" + id, jsonSet(rec, "state", "DONE"));
  return "refundedA:" + rec.amtA;
}

// Responder refund: after tlB if Alice never claimed.
export function swapRefundB(id: bigint): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("s:" + id) ?? REC0;
  if (rec.state != "BOTH") {
    near.abort("not in state BOTH");
  }
  if (who != rec.resp) {
    near.abort("only the responder may refund");
  }
  if (ltTs(near.blockTimestamp(), rec.tlB) == ZERO) {
    near.abort("not yet timed out");
  }
  let bal = near.storageGet("fb:" + who) ?? ZERO;
  near.storageSet("fb:" + who, bal + rec.amtB);
  near.storageSet("s:" + id, jsonSet(rec, "state", "A"));
  return "refundedB:" + rec.amtB;
}
