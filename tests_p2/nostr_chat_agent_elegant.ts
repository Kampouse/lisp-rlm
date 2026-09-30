// Nostr chat agent v4 — elegant TypeScript surface (2026-09-30).
// Semantics-identical twin of tests_p2/nostr_chat_agent.lisp, rewritten in
// the modern dialect: template literals, .length/.slice/.includes, boolean
// logic. No nested strCat, no strLength/strIndexOf/toStr ceremony.
// NEAR_SENDER_ID (host-injected from the request_execution tx, unforgeable)
// is the caller; pk33 cache + schnorr-sign-pk skip the internal P=d·G mult;
// the signed NIP-01 event is POSTed to the relay by the agent itself.
//   salt = VRF at registration, sk = SHA256(root ‖ 0x1f ‖ salt ‖ 0x1f ‖ sender)
//   pk33 = SEC1(0x02/0x03 ‖ x) cached at pk33:<sender>
// Ops: pk {} | chat {content, ts, nonce, room, relay}
function sep(): string { return hexDecode("1f"); }
function deriveSk(root: string, salt: string, sender: string): string {
  return hexDecode(sha256Hash(`${hexDecode(root)}${sep()}${hexDecode(salt)}${sep()}${sender}`));
}
function ensureSalt(sender: string): string {
  const s = outlayer.storageGet(`salt:${sender}`);
  if (s) { return s; }
  // near:vrf host rejects ':' in seeds — keep colon-free
  const fresh = vrfGenerate(`nostr-salt-${sender}`);
  outlayer.storageSet(`salt:${sender}`, fresh);
  return fresh;
}
function ensurePk33(sender: string, sk: string): string {
  const c = outlayer.storageGet(`pk33:${sender}`);
  if (c && c.length == 66) { return hexDecode(c); }
  const p33 = schnorrPubkey33(sk);
  outlayer.storageSet(`pk33:${sender}`, hexEncode(p33));
  return p33;
}
function xOnly(pk33: string): string { return pk33.slice(1, 33); }
function tagsSer(room: string, nonce: string): string {
  // INNER list only — evSer/evJson add the enclosing [ ] (NIP-01 tags field)
  return `["t","${room}"],["nonce","${nonce}"]`;
}
function evSer(pk: string, ts: string, kind: string, tags: string, content: string): string {
  // content arrives PRE-QUOTED (jsonQuote output includes the quotes)
  return `[0,"${pk}",${ts},${kind},[${tags}],${content}]`;
}
function evJson(id: string, pk: string, ts: string, kind: string, tags: string, content: string, sig: string): string {
  return `{"id":"${id}","pubkey":"${pk}","created_at":${ts},"kind":${kind},"tags":[${tags}],"content":${content},"sig":"${sig}"}`;
}
function err(m: string): string { return `{"error":"${m}"}`; }
// Literal URLs ONLY: the outlayer host's dynamic http-post import (21) fails
// component instantiation on the worker, so relays are compile-time baked
// (0=nos.lol default, 1=relay.damus.io) via the native wasi:http bridge.
function postRelay(which: string, body: string): string {
  if (which == "1") { return httpPost("https://relay.damus.io", body); }
  return httpPost("https://nos.lol", body);
}
function resolveCaller(input: string): string {
  const envs = env.get("NEAR_SENDER_ID");
  if (envs) { return envs; }
  const proot = env.get("PROTECTED_NOSTR_ROOT");
  if (proot && proot.length > 63) { return ""; } // fail closed under a real root
  const inp = jsonGetStr("caller", input);
  return inp ? inp : ""; // value-default: || is boolean-only in this dialect
}
function rootOf(): string {
  const proot = env.get("PROTECTED_NOSTR_ROOT");
  if (proot && proot.length > 63) { return proot; }
  return "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
}
function opPk(sender: string): string {
  const salt = ensureSalt(sender);
  const sk = deriveSk(rootOf(), salt, sender);
  const pk = hexEncode(xOnly(ensurePk33(sender, sk)));
  return `{"pk":"${pk}","sender":"${sender}"}`;
}
function opChat(sender: string, content: string, ts: string, nonce: string, room: string, r: string): string {
  if (nonce.length < 1 || nonce.length > 32) { return err("invalid-chat-args"); }
  if (room.length < 1 || room.length > 64) { return err("invalid-chat-args"); }
  if (content.length > 800) { return err("invalid-chat-args"); }
  const salt = ensureSalt(sender);
  const sk = deriveSk(rootOf(), salt, sender);
  const pk33 = ensurePk33(sender, sk);
  const pk = hexEncode(xOnly(pk33));
  const tags = tagsSer(room, nonce);
  const ser = evSer(pk, ts, "1", tags, jsonQuote(content));
  const idh = sha256Hash(ser);
  // aux = SHA256(room ‖ 0x1f ‖ nonce): same nonce on retry → same sig →
  // relays dedupe; a new nonce gives fresh deterministic randomness.
  const aux = hexDecode(sha256Hash(`${room}${sep()}${nonce}`));
  const sig = hexEncode(schnorrSignPk(sk, pk33, hexDecode(idh), aux));
  const res = postRelay(r, `["EVENT",${evJson(idh, pk, ts, "1", tags, jsonQuote(content), sig)}]`);
  const ok = res && res.includes("\"OK\"") ? 1 : 0;
  return `{"id":"${idh}","sig":"${sig}","pk":"${pk}","posted":${ok}}`;
}
export function run(input: string): string {
  const op = jsonGetStr("op", input);
  const sender = resolveCaller(input);
  if (sender.length < 1) { return err("no-identity-path"); }
  if (op == "chat") {
    return opChat(sender, jsonGetStr("content", input), jsonGetStr("ts", input),
      jsonGetStr("nonce", input), jsonGetStr("room", input), jsonGetStr("r", input));
  }
  return opPk(sender);
}
