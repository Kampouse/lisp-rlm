// TypeScript identity agent — elegant surface (2026-09-30).
// Semantics-identical twin of nostr_identity_agent.ts, written the way the
// dialect now intends: template literals, .length/.includes, boolean-typed
// predicates, + concat. No nested strCat, no hand-rolled contains, no
// manual string-length C-isms.
function sep(): string { return hexDecode("1f"); }
function deriveSk(root: string, salt: string, caller: string): string {
  return hexDecode(sha256Hash(`${hexDecode(root)}${sep()}${hexDecode(salt)}${sep()}${caller}`));
}
function pkOf(sk: string): string { return hexEncode(schnorrPubkey(sk)); }
function evSer(pk: string, ts: string, kind: string, content: string): string {
  return `[0,"${pk}",${ts},${kind},[],"${content}"]`;
}
function bumpStore(key: string): string {
  const cur = outlayer.storageGet(key);
  const n = cur ? strToNum(cur) : 0;
  outlayer.storageSet(key, `${n + 1}`);
  return `${n}`;
}
function purposeOf(input: string): string {
  const ts = jsonGetStr("ts", input);
  if (ts && ts.length > 0) {
    return `sign|${ts}|${jsonGetStr("kind", input)}|${jsonGetStr("content", input)}`;
  }
  return "derive";
}
function issueChallenge(root: string, caller: string, purpose: string): string {
  const ctr = bumpStore("ctr:identity");
  const ch = sha256Hash(`${hexDecode(root)}${sep()}${caller}${sep()}${ctr}${sep()}${purpose}`);
  outlayer.storageSet(`ch:${caller}`, ch);
  outlayer.storageSet(`chp:${caller}`, purpose);
  return ch;
}
function verifyAttestation(caller: string, tx: string, purpose: string): boolean {
  const ch = outlayer.storageGet(`ch:${caller}`);
  const stp = outlayer.storageGet(`chp:${caller}`);
  if (ch && stp == purpose) {
    const res = outlayer.raw("tx", `["${tx}","${caller}"]`);
    if (res.includes(`"signer_id":"${caller}"`) && res.includes(ch)) {
      outlayer.storageDelete(`ch:${caller}`);
      outlayer.storageDelete(`chp:${caller}`);
      return true;
    }
  }
  return false;
}
function ensureSalt(caller: string): string {
  const s = outlayer.storageGet(`salt:${caller}`);
  if (s) { return s; }
  // NB: near:vrf host rejects ':' in seeds — keep colon-free
  const fresh = vrfGenerate(`nostr-salt-${caller}`);
  outlayer.storageSet(`salt:${caller}`, fresh);
  return fresh;
}
function opDerive(root: string, caller: string, tx: string): string {
  if (!verifyAttestation(caller, tx, "derive")) { return `{"error":"attestation-failed"}`; }
  const salt = ensureSalt(caller);
  const pk = pkOf(deriveSk(root, salt, caller));
  const isDemo = root == "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
  return `{"pk":"${pk}","caller":"${caller}","root":"${isDemo ? "demo" : "real"}","salt":"${salt ? salt : "?"}"}`;
}
function opSign(root: string, caller: string, tx: string, ts: string, kind: string, content: string): string {
  const purpose = `sign|${ts}|${kind}|${content}`;
  if (!verifyAttestation(caller, tx, purpose)) { return `{"error":"attestation-failed"}`; }
  const salt = ensureSalt(caller);
  const sk = deriveSk(root, salt, caller);
  const pk = pkOf(sk);
  const idh = sha256Hash(evSer(pk, ts, kind, content));
  const sig = schnorrSign(sk, hexDecode(idh), hexDecode("0000000000000000000000000000000000000000000000000000000000000000"));
  return `{"id":"${idh}","sig":"${hexEncode(sig)}","pk":"${pk}","caller":"${caller}"}`;
}
export function run(input: string): string {
  const op = jsonGetStr("op", input);
  const caller = jsonGetStr("caller", input);
  const proot = env.get("PROTECTED_NOSTR_ROOT");
  const envRoot = proot && proot.length > 63 ? proot : "";
  const root = envRoot.length > 63 ? envRoot
    : "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
  if (op == "attest") { return `{"attested":"${jsonGetStr("challenge", input)}"}`; }
  if (op == "challenge") {
    return `{"challenge":"${issueChallenge(root, caller, purposeOf(input))}","caller":"${caller}"}`;
  }
  if (op == "sign") {
    return opSign(root, caller, jsonGetStr("tx", input), jsonGetStr("ts", input),
      jsonGetStr("kind", input), jsonGetStr("content", input));
  }
  return opDerive(root, caller, jsonGetStr("tx", input));
}
