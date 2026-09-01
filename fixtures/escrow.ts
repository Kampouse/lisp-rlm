// HTLC escrow: hashlock + timelock state machine.
// e:<id> records via the v4 dot-access pattern (shape from the default).
// hashlock = sha256(secret) raw 32 bytes — never quoted, byte-compared.
// PENDING --claim(secret, before tl)--> CLAIMED
// PENDING --refund(after tl, sender)--> REFUNDED

const ZERO = 0n;
const NANOS = 1000000000n;
function deadlineFrom(now: bigint, sec: bigint): bigint {
  return now + sec * NANOS;
}

function abortIfPast(now: bigint, tl: bigint): bigint {
  if (now > tl) {
    near.abort("timed out");
  }
  return 0n;
}

function abortIfBefore(now: bigint, tl: bigint): bigint {
  if (now < tl) {
    near.abort("not yet timed out");
  }
  return 0n;
}

const REC0 = '{"sender":"","recipient":"","amt":"0","tl":"0","state":""}';

export function escrowNew(recipient: string, secret: string, amount: bigint, timeoutSec: bigint): string {
  let who = near.signerAccountId();
  let bal = near.storageGet("ft:" + who) ?? ZERO;
  if (bal < amount) {
    near.abort("insufficient balance");
  }
  let id = near.storageGet("e:count") ?? ZERO;
  id = id + 1n;
  let rec = jsonSet(REC0, "sender", who);
  rec = jsonSet(rec, "recipient", recipient);
  rec = jsonSet(rec, "amt", amount);
  rec = jsonSet(rec, "tl", deadlineFrom(near.blockTimestamp(), timeoutSec));
  rec = jsonSet(rec, "state", "PENDING");
  near.storageSet("e:" + id, rec);
  near.storageSet("eh:" + id, near.sha256Hash(secret));
  near.storageSet("e:count", id);
  near.storageSet("ft:" + who, bal - amount);
  return "escrow:" + id;
}

export function escrowClaim(id: bigint, secret: string): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("e:" + id) ?? REC0;
  if (rec.state != "PENDING") {
    near.abort("not pending");
  }
  if (who != rec.recipient) {
    near.abort("only the recipient may claim");
  }
  if (near.sha256Hash(secret) != (near.storageGet("eh:" + id) ?? "")) {
    near.abort("wrong secret");
  }
  abortIfPast(near.blockTimestamp(), rec.tl);
  let rBal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ft:" + who, rBal + rec.amt);
  near.storageSet("e:" + id, jsonSet(rec, "state", "CLAIMED"));
  return "claimed:" + rec.amt;
}

export function escrowRefund(id: bigint): string {
  let who = near.signerAccountId();
  let rec = near.storageGet("e:" + id) ?? REC0;
  if (rec.state != "PENDING") {
    near.abort("not pending");
  }
  if (who != rec.sender) {
    near.abort("only the sender may refund");
  }
  abortIfBefore(near.blockTimestamp(), rec.tl);
  let sBal = near.storageGet("ft:" + who) ?? ZERO;
  near.storageSet("ft:" + who, sBal + rec.amt);
  near.storageSet("e:" + id, jsonSet(rec, "state", "REFUNDED"));
  return "refunded:" + rec.amt;
}

export function escrowInfo(id: bigint): string {
  return near.storageGet("e:" + id) ?? "none";
}

export function ftBalanceOf(who: string): string {
  return near.storageGet("ft:" + who) ?? ZERO;
}

export function ftMint(to: string, amount: bigint): string {
  if ((near.storageGet("ft:own") ?? "") == "") {
    near.storageSet("ft:own", near.signerAccountId());
  }
  if (near.signerAccountId() != (near.storageGet("ft:own") ?? "")) {
    near.abort("only the owner may mint");
  }
  let bal = near.storageGet("ft:" + to) ?? ZERO;
  let supply = near.storageGet("ft:supply") ?? ZERO;
  near.storageSet("ft:" + to, bal + amount);
  near.storageSet("ft:supply", supply + amount);
  return "supply:" + (supply + amount);
}
