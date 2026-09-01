const REC0 = '{"sender":"","recipient":"","hashlock":"","amt":"0","tl":"0","state":""}';
export function t(recipient: string, secret: string, amount: bigint, timeoutSec: bigint): string {
  let who = near.signerAccountId();
  let rec = jsonSet(REC0, "sender", who);
  rec = jsonSet(rec, "recipient", recipient);
  rec = jsonSet(rec, "hashlock", near.sha256Hash(secret));
  rec = jsonSet(rec, "amt", amount);
  rec = jsonSet(rec, "tl", near.blockTimestamp() + timeoutSec * 1000000000n);
  rec = jsonSet(rec, "state", "PENDING");
  return rec;
}
