// TypeScript identity agent — the same non-spoofable hybrid identity flow
// as tests_p2/nostr_identity_agent.lisp, written in the TS dialect.
// Ops: attest / challenge {caller,[ts,kind,content]} / derive {caller,tx} /
//      sign {caller,tx,ts,kind,content}.
//   sk   = SHA256(root ‖ 0x1f   salt   0x1f   caller), salt = VRF at registration
//   ch   = SHA256(root ‖ caller ‖ ctr   purpose) — event-bound, single use
function sep(): string { return hexDecode("1f"); }
function deriveSk(root: string, salt: string, caller: string): string {
  return hexDecode(sha256Hash(strCat(hexDecode(root),
    strCat(sep(), strCat(hexDecode(salt), strCat(sep(), caller))))));
}
function pkOf(sk: string): string { return hexEncode(schnorrPubkey(sk)); }
function evSer(pk: string, ts: string, kind: string, content: string): string {
  const q = hexDecode("22");
  return strCat("[0," + q + pk + q + "," + ts + "," + kind + ",[]," + q + content + q + "]");
}
function bumpStore(key: string): string {
  const cur = outlayer.storageGet(key);
  const n = cur ? strToNum(cur) : 0;
  outlayer.storageSet(key, toStr(n + 1));
  return toStr(n);
}
function purposeOf(input: string): string {
  const ts = jsonGetStr("ts", input);
  if (ts && strLength(ts) > 0) {
    return "sign|" + ts + "|" + jsonGetStr("kind", input) + "|" + jsonGetStr("content", input);
  }
  return "derive";
}
function issueChallenge(root: string, caller: string, purpose: string): string {
  const ctr = bumpStore("ctr:identity");
  const ch = sha256Hash(strCat(hexDecode(root),
    strCat(sep(), strCat(caller, strCat(sep(), strCat(ctr, strCat(sep(), purpose)))))));
  outlayer.storageSet("ch:" + caller, ch);
  outlayer.storageSet("chp:" + caller, purpose);
  return ch;
}
function contains(hay: string, needle: string): number {
  return strIndexOf(hay, needle) > 0 ? 1 : 0;
}
function verifyAttestation(caller: string, tx: string, purpose: string) {
  const ch = outlayer.storageGet("ch:" + caller);
  const stp = outlayer.storageGet("chp:" + caller);
  const res = ch ? outlayer.raw("tx", "[\"" + tx + "\",\"" + caller + "\"]") : "no-challenge";
  const okSigner = ch ? contains(res, "\"signer_id\":\"" + caller + "\"") : 0;
  const okCh = ch ? contains(res, ch) : 0;
  const okPurpose = stp ? (stp == purpose ? 1 : 0) : 0;
  if (okSigner > 0 && okCh > 0 && okPurpose > 0) {
    outlayer.storageDelete("ch:" + caller);
    outlayer.storageDelete("chp:" + caller);
    return true;
  }
  return false;
}
function ensureSalt(caller: string): string {
  const s = outlayer.storageGet("salt:" + caller);
  if (s) { return s; }
  // NB: near:vrf host rejects ':' in seeds — keep colon-free
  const fresh = vrfGenerate("nostr-salt-" + caller);
  outlayer.storageSet("salt:" + caller, fresh);
  return fresh;
}
function opDerive(root: string, caller: string, tx: string): string {
  if (!verifyAttestation(caller, tx, "derive")) { return "{\"error\":\"attestation-failed\"}"; }
  const salt = ensureSalt(caller);
  const pk = pkOf(deriveSk(root, salt, caller));
  const isDemo = root == "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
  return "{\"pk\":\"" + pk + "\",\"caller\":\"" + caller + "\",\"root\":\"" +
    (isDemo ? "demo" : "real") + "\",\"salt\":\"" + (salt ? salt : "?") + "\"}";
}
function opSign(root: string, caller: string, tx: string, ts: string, kind: string, content: string): string {
  const purpose = "sign|" + ts + "|" + kind + "|" + content;
  if (!verifyAttestation(caller, tx, purpose)) { return "{\"error\":\"attestation-failed\"}"; }
  const salt = ensureSalt(caller);
  const sk = deriveSk(root, salt, caller);
  const pk = pkOf(sk);
  const idh = sha256Hash(evSer(pk, ts, kind, content));
  const sig = schnorrSign(sk, hexDecode(idh), hexDecode("0000000000000000000000000000000000000000000000000000000000000000"));
  return "{\"id\":\"" + idh + "\",\"sig\":\"" + hexEncode(sig) +
    "\",\"pk\":\"" + pk + "\",\"caller\":\"" + caller + "\"}";
}
export function run(input: string): string {
  const op = jsonGetStr("op", input);
  const caller = jsonGetStr("caller", input);
  const proot = env.get("PROTECTED_NOSTR_ROOT");
  const envRoot = proot && strLength(proot) > 63 ? proot : "";
  const root = strLength(envRoot) > 63 ? envRoot
    : "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
  if (op == "attest") { return "{\"attested\":\"" + jsonGetStr("challenge", input) + "\"}"; }
  if (op == "challenge") {
    return "{\"challenge\":\"" + issueChallenge(root, caller, purposeOf(input)) +
      "\",\"caller\":\"" + caller + "\"}";
  }
  if (op == "sign") {
    return opSign(root, caller, jsonGetStr("tx", input), jsonGetStr("ts", input),
      jsonGetStr("kind", input), jsonGetStr("content", input));
  }
  return opDerive(root, caller, jsonGetStr("tx", input));
}
