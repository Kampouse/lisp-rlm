/// <reference path="../types/lisp-rlm.d.ts" />
// registrar — per-caller Nostr identity registry (local near-mock rehearsal
// of the contract → OutLayer → Nostr pipeline).
//
// Flow:
//   post(content)   predecessor = caller → yieldCreate(on_sign, {caller, content})
//   [off-chain worker signs the event, resumes with "id|sig"]
//   on_sign(args)   payload via promiseResult(0) = "id|sig"
//                   → re-serialize, sha256 → id must match
//                   → schnorrVerify (host, k256) must pass
//                   → store registry[caller] = {pk, id, sig} (pk set once)
//
// pk comes from the worker via the "pk" arg of the yield (first post) or is
// already registered. The worker derives sk = SHA256(root||0x1f||caller) —
// the contract NEVER sees sk, only pk + a self-verifying signature.

function die(m: string) {
  near.log(m);
  near.abort(m);
}

function getStr(k: string): string {
  return near.storageGet(k) ?? "";
}

// NIP-01 serialization (same shape the worker uses):
// [0,"<pk>",<created_at>,<kind>,[],"<content>"]
function eventSerialize(pk: string, cat: string, kind: string, content: string): string {
  return `[0,"${pk}",${cat},${kind},[],"${content}"]`;
}

// ── mutators ─────────────────────────────────────────────────────────────

// Called by a user. Attests the caller, hands (caller, content, pk) to the
// signing worker via yield/resume. pk is required on first post (from the
// worker's derive step) and must match the stored identity afterwards.
export function post(): string {
  const caller = near.predecessorAccountId();
  const content = near.jsonGetStr("content");
  if (content === null) die("ERR_NO_CONTENT");
  const pkIn = near.jsonGetStr("pk") ?? "";

  const stored = getStr(`pk:${caller}`);
  let pk = stored;
  if (strLength(stored) === 0) {
    if (strLength(pkIn) === 0) die("ERR_NO_PK");
    pk = pkIn;
  } else if (strLength(pkIn) !== 0 && pkIn !== stored) {
    die("ERR_PK_MISMATCH");
  }

  const args = `{"caller":"${caller}","content":${jsonQuote(content)},"pk":"${pk}"}`;
  const idx = near.yieldCreate("on_sign", args, 10000000000000, 1);
  near.log(`POST:${caller}:${idx}`);
  return toStr(idx);
}

// Driver entry: resume the yielded promise with the worker's "id|sig".
// yieldResume is idempotent-rejecting: once consumed, it returns 0.
export function resume(): string {
  const caller = near.predecessorAccountId();
  if (caller !== near.currentAccountId()) die("ERR_NOT_SELF");
  const idStr = near.jsonGetStr("id") ?? "0";
  const payload = near.jsonGetStr("payload") ?? "";
  const ok = near.yieldResume(idStr, payload);
  if (ok !== 1) die("ERR_RESUME_FAILED");
  return "resumed";
}

// Resume entry: the worker's output arrives via promiseResult(0) as "id|sig".
// Verify EVERYTHING before storing — the worker is untrusted.
export function on_sign(): string {
  const caller = near.jsonGetStr("caller");
  if (caller === null) die("ERR_NO_CALLER");
  const content = near.jsonGetStr("content");
  if (content === null) die("ERR_NO_CONTENT");
  const pk = near.jsonGetStr("pk");
  if (pk === null) die("ERR_NO_PK");

  const payload = near.promiseResult(0);
  if (strLength(payload) === 0) {
    // NotReady leg (yield executes once with no result before resume):
    // no state changes, the resumed run carries the payload.
    near.log("NOT_READY");
    return "pending";
  }
  const sep = strIndexOf(payload, "|");
  if (sep < 0) die("ERR_BAD_PAYLOAD");
  const id = strSlice(payload, 0, sep);
  const sig = strSlice(payload, sep + 1, strLength(payload));

  // 1) id must equal sha256 of the serialization WE compute
  const ser = eventSerialize(pk, "0", "1", content);
  const expect = sha256Hash(ser);
  if (id !== expect) die("ERR_ID_MISMATCH");

  // 2) signature must verify against (id, pk) — host schnorr, k256-backed
  // (builtin takes DECODED bytes, per nostr-gov convention)
  const ok = schnorrVerify(hexDecode(pk), hexDecode(sig), hexDecode(id));
  if (ok !== 1) die("ERR_SIG_INVALID");

  // 3) persist identity + event (pk set once, first post wins)
  if (strLength(getStr(`pk:${caller}`)) === 0) {
    near.storageSet(`pk:${caller}`, pk);
  }
  near.storageSet(`last:${caller}`, `{"id":${jsonQuote(id)},"sig":${jsonQuote(sig)},"content":${jsonQuote(content)}}`);
  near.log(`SIGNED:${caller}:${id}`);
  return id;
}

// ── views ────────────────────────────────────────────────────────────────

export function get_pk(): string {
  const caller = near.jsonGetStr("caller") ?? "";
  return getStr(`pk:${caller}`);
}

export function get_last(): string {
  const caller = near.jsonGetStr("caller") ?? "";
  return getStr(`last:${caller}`);
}
