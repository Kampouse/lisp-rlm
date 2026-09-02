/// <reference path="../../../ts/lisp-rlm.d.ts" />
// nostr-gov Phase-1 — TypeScript port (differential twin of main.lisp)
// Scope: legacy (owner-key) auth path. Event-auth (`ev` param) paths stub out.
// Helpers are internal; `export function` = contract method. `get_*` = view.

// ── constants ────────────────────────────────────────────────────────────
const VERSION = "1";
const NAME_CHARS = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";

// ── helpers ──────────────────────────────────────────────────────────────

function die(m: string) {
  near.log(m);
  near.abort(m);
}

function getStr(k: string) {
  return near.storageGet(k) ?? "";
}

function numStr(k: string) {
  const v = getStr(k);
  return strLength(v) === 0 ? "0" : v;
}

function getNum(k: string) {
  return strToNum(numStr(k));
}

// sliding nonce window (64 nonces), bitmap lo/hi u32 pair
function nonceWindowCheck(n: number) {
  const base = getNum("ononce");
  if (n < base) {
    die("ERR_NONCE_TOO_LOW");
  }
  if (n >= base + 64) {
    die("ERR_NONCE_WINDOW_EXCEEDED");
  }
}

function slideWindow(loIn: number, hiIn: number) {
  let lo = loIn;
  let hi = hiIn;
  while ((lo & 1) === 1) {
    const nlo = (lo >> 1) | ((hi & 1) << 31);
    const nhi = hi >> 1;
    near.storageSet("ononce", toStr(getNum("ononce") + 1));
    near.storageSet("obm_lo", toStr(nlo));
    near.storageSet("obm_hi", toStr(nhi));
    lo = nlo;
    hi = nhi;
  }
}

function nonceBitSet(k: number, lo: number, hi: number) {
  return k < 32 ? lo | (1 << k) : hi;
}

function nonceBitSetHi(k: number, hi: number) {
  return k < 32 ? hi : hi | (1 << (k - 32));
}

function nonceBitGet(k: number, lo: number, hi: number) {
  return k < 32 ? lo & (1 << k) : hi & (1 << (k - 32));
}

function consumeNonce(n: number) {
  nonceWindowCheck(n);
  const k = n - getNum("ononce");
  const lo = getNum("obm_lo");
  const hi = getNum("obm_hi");
  const cur = nonceBitGet(k, lo, hi);
  if (cur !== 0) {
    die("ERR_NONCE_ALREADY_USED");
  }
  near.storageSet("obm_lo", toStr(nonceBitSet(k, lo, hi)));
  near.storageSet("obm_hi", toStr(nonceBitSetHi(k, hi)));
  slideWindow(nonceBitSet(k, lo, hi), nonceBitSetHi(k, hi));
}

function verifyOwner(action: string, sig: string, expires: string, nonce: string) {
  const ts = near.blockTimestamp();
  if (u128.gt(ts, expires)) {
    die("ERR_SIG_EXPIRED");
  }
  const msg = `expires ${expires}.000000000: ${action} | nonce: ${nonce} | contract: ${near.currentAccountId()}`;
  const pk = getStr("owner_npub0");
  if (strLength(pk) === 0) {
    die("ERR_NOT_INITIALIZED");
  }
  const pkb = hexDecode(pk);
  const sigb = hexDecode(sig);
  const mh = hexDecode(sha256Hash(msg));
  const ok = schnorrVerify(pkb, sigb, mh);
  if (ok === 1) {
    consumeNonce(strToNum(nonce));
  } else {
    die("ERR_INVALID_OWNER_SIGNATURE");
  }
}

// name validation: chars must be in NAME_CHARS (scan — str-index-of needs literals)
function charMatches(a: string, b: string, j: number, m: number) {
  let r = 0;
  let jj = j;
  while (jj < m) {
    if (a === strSlice(b, jj, jj + 1)) {
      r = 1;
      break;
    }
    jj = jj + 1;
  }
  return r;
}

function nameCharOk(s: string, i: number) {
  const c = strSlice(s, i, i + 1);
  return charMatches(c, NAME_CHARS, 0, strLength(NAME_CHARS));
}

function nameValid(s: string) {
  const n = strLength(s);
  if (n === 0) {
    return 0;
  }
  if (n > 64) {
    return 0;
  }
  let i = 0;
  let ok = 1;
  while (i < n && ok === 1) {
    ok = nameCharOk(s, i);
    i = i + 1;
  }
  return ok;
}

// approver list utilities (comma-separated pubkey fields)
function countCommas(s: string) {
  let c = 0;
  let i = 0;
  const n = strLength(s);
  while (i < n) {
    if (strSlice(s, i, i + 1) === ",") {
      c = c + 1;
    }
    i = i + 1;
  }
  return c;
}

function approverCount(pks: string) {
  return countCommas(pks) + 1;
}

function nthField(s: string, k: number) {
  const n = strLength(s);
  let i = 0;
  let start = 0;
  let kk = k;
  let out = "";
  let done = 0;
  while (i < n && done === 0) {
    if (strSlice(s, i, i + 1) === ",") {
      if (kk === 0) {
        out = strSlice(s, start, i);
        done = 1;
      } else {
        kk = kk - 1;
        start = i + 1;
      }
    }
    i = i + 1;
  }
  return done === 1 ? out : kk === 0 ? strSlice(s, start, n) : "";
}

// u128-string bitmap powers of two
function pow2(k: number) {
  let acc = "1";
  let i = 0;
  while (i < k) {
    acc = u128.mul(acc, "2");
    i = i + 1;
  }
  return acc;
}

function bmBitSet(bm: string, k: number) {
  return u128.add(bm, pow2(k));
}

function bmBitIsSet(bm: string, k: number) {
  return u128.mod(u128.div(bm, pow2(k)), "2") === "1" ? 1 : 0;
}

// ── Phase 1.5: event auth (nostr kind 37500) ─────────────────────────────
// Tag parsing with LITERAL needles only (str-index-of constraint at emit
// time) — each extractor inlines its own needle, mirroring the lisp twin.

const EMPTY = "";

function tagAction(tags: string) {
  const i = strIndexOf(tags, "[\"action\",\"");
  if (i === -1) {
    return EMPTY;
  }
  const rest = strSlice(tags, i + 11, strLength(tags));
  if (strLength(rest) === 0) {
    return EMPTY;
  }
  return strSlice(rest, 0, strIndexOf(rest, "\""));
}

function tagContract(tags: string) {
  const i = strIndexOf(tags, "[\"contract\",\"");
  if (i === -1) {
    return EMPTY;
  }
  const rest = strSlice(tags, i + 13, strLength(tags));
  if (strLength(rest) === 0) {
    return EMPTY;
  }
  return strSlice(rest, 0, strIndexOf(rest, "\""));
}

function tagNonce(tags: string) {
  const i = strIndexOf(tags, "[\"nonce\",\"");
  if (i === -1) {
    return EMPTY;
  }
  const rest = strSlice(tags, i + 10, strLength(tags));
  if (strLength(rest) === 0) {
    return EMPTY;
  }
  return strSlice(rest, 0, strIndexOf(rest, "\""));
}

function tagExpires(tags: string) {
  const i = strIndexOf(tags, "[\"expires\",\"");
  if (i === -1) {
    return EMPTY;
  }
  const rest = strSlice(tags, i + 12, strLength(tags));
  if (strLength(rest) === 0) {
    return EMPTY;
  }
  return strSlice(rest, 0, strIndexOf(rest, "\""));
}

// canonical nostr event serialization for signing:
// [0,"<pk>",<created_at>,<kind>,<tags json>,"<content>"]
function eventSerialize(pk: string, cat: string, kind: string, tags: string, content: string) {
  return `[0,"${pk}",${cat},${kind},${tags},"${content}"]`;
}

function verifyOwnerEvent(actionStr: string) {
  const pk = near.jsonGetStr("pk");
  const kind = near.jsonGetStr("kind");
  const tags = near.jsonGetStr("tags");
  const content = near.jsonGetStr("ct");
  const sig = near.jsonGetStr("sig");
  const cat = near.jsonGetStr("cat");
  if (strLength(pk) !== 64) {
    die("ERR_EVENT_PK_LEN");
  }
  if (strLength(sig) !== 128) {
    die("ERR_EVENT_SIG_LEN");
  }
  if (kind !== "37500") {
    die("ERR_EVENT_KIND");
  }
  if (pk !== getStr("owner_npub0")) {
    die("ERR_EVENT_PK_MISMATCH");
  }
  const ta = tagAction(tags);
  const tc = tagContract(tags);
  const tn = tagNonce(tags);
  const te = tagExpires(tags);
  const ts = near.blockTimestamp();
  if (u128.gt(ts, te)) {
    die("ERR_SIG_EXPIRED");
  }
  if (ta !== actionStr) {
    die("ERR_EVENT_ACTION");
  }
  if (tc !== near.currentAccountId()) {
    die("ERR_EVENT_CONTRACT");
  }
  const serialized = eventSerialize(pk, cat, kind, tags, content);
  const pkb = hexDecode(pk);
  const sigb = hexDecode(sig);
  const mh = hexDecode(sha256Hash(serialized));
  const ok = schnorrVerify(pkb, sigb, mh);
  if (ok === 1) {
    consumeNonce(strToNum(tn));
  } else {
    die("ERR_EVENT_SIG_INVALID");
  }
}

// guardian variant: pause carries NO nonce (mirrors legacy pause)
function verifyGuardianEvent(actionStr: string) {
  const pk = near.jsonGetStr("pk");
  const kind = near.jsonGetStr("kind");
  const tags = near.jsonGetStr("tags");
  const content = near.jsonGetStr("ct");
  const sig = near.jsonGetStr("sig");
  const cat = near.jsonGetStr("cat");
  if (strLength(pk) !== 64) {
    die("ERR_EVENT_PK_LEN");
  }
  if (strLength(sig) !== 128) {
    die("ERR_EVENT_SIG_LEN");
  }
  if (kind !== "37500") {
    die("ERR_EVENT_KIND");
  }
  if (pk !== getStr("owner_npub0")) {
    die("ERR_EVENT_PK_MISMATCH");
  }
  const ta = tagAction(tags);
  const tc = tagContract(tags);
  const te = tagExpires(tags);
  const ts = near.blockTimestamp();
  if (u128.gt(ts, te)) {
    die("ERR_SIG_EXPIRED");
  }
  if (ta !== actionStr) {
    die("ERR_EVENT_ACTION");
  }
  if (tc !== near.currentAccountId()) {
    die("ERR_EVENT_CONTRACT");
  }
  const serialized = eventSerialize(pk, cat, kind, tags, content);
  const pkb = hexDecode(pk);
  const sigb = hexDecode(sig);
  const mh = hexDecode(sha256Hash(serialized));
  const ok = schnorrVerify(pkb, sigb, mh);
  if (ok !== 1) {
    die("ERR_EVENT_SIG_INVALID");
  }
  return 0;
}

function authOwner(action: string) {
  const sig = near.jsonGetStr("signature");
  const expires = near.jsonGetStr("expires_at");
  const nonce = near.jsonGetStr("nonce");
  // pause gates all owner-auth'd actions, both auth dialects
  if (strLength(getStr("paused")) !== 0) {
    die("ERR_PAUSED");
  }
  if (strLength(near.jsonGetStr("ev")) === 0) {
    // legacy sig path: expires/nonce are top-level args
    if (strLength(expires) === 0) {
      die("ERR_ARG_EXPIRES");
    }
    if (strLength(nonce) === 0) {
      die("ERR_ARG_NONCE");
    }
    verifyOwner(action, sig, expires, nonce);
  } else {
    // event-auth path: expires/nonce live in event tags
    verifyOwnerEvent(action);
  }
}

// ── lifecycle ────────────────────────────────────────────────────────────

export function init() {
  if (strLength(getStr("owner_npub0")) !== 0) {
    die("ERR_ALREADY_INITIALIZED");
  }
  const npub = near.jsonGetStr("npub");
  if (strLength(npub) !== 64) {
    die("ERR_BAD_NPUB");
  }
  near.storageSet("owner_npub0", npub);
  return 0;
}

export function create_wallet() {
  const name = near.jsonGetStr("name");
  const sig = near.jsonGetStr("signature");
  const expires = near.jsonGetStr("expires_at");
  const nonce = near.jsonGetStr("nonce");
  // ev routing first (mirrors the Rust reference): event-auth calls carry
  // tags, not legacy expires_at/nonce, so arg validation is legacy-only.
  if (strLength(near.jsonGetStr("ev")) === 0) {
    if (strLength(expires) === 0) {
      die("ERR_ARG_EXPIRES");
    }
    if (strLength(nonce) === 0) {
      die("ERR_ARG_NONCE");
    }
    if (strLength(getStr("paused")) !== 0) {
      die("ERR_PAUSED");
    }
    verifyOwner(`create_wallet:${name}`, sig, expires, nonce);
  } else {
    verifyOwnerEvent(`create_wallet:${name}`);
  }
  if (!near.depositGte(1001882102603448320, 27105)) {
    die("ERR_STORAGE_DEPOSIT");
  }
  if (strLength(getStr(`w:${name}`)) !== 0) {
    die("ERR_WALLET_EXISTS");
  }
  if (nameValid(name) === 0) {
    die("ERR_NAME_INVALID_CHARS");
  }
  const creator = near.predecessorAccountId();
  const createdAt = near.blockTimestamp();
  const deposit = near.attachedDepositU128();
  near.storageSet(`w:${name}`, `{"name":"${name}","creator":"${creator}","created_at":"${createdAt}","deposit":"${deposit}"}`);
  near.log(`wallet created: ${name}`);
  return 0;
}

export function pause() {
  if (strLength(near.jsonGetStr("ev")) === 0) {
    // legacy: owner signature
    const sig = near.jsonGetStr("signature");
    const expires = near.jsonGetStr("expires_at");
    const ts = near.blockTimestamp();
    if (u128.gt(ts, expires)) {
      die("ERR_SIG_EXPIRED");
    }
    const msg = `expires ${expires}.000000000: pause | contract: ${near.currentAccountId()}`;
    const pk = getStr("owner_npub0");
    if (strLength(pk) === 0) {
      die("ERR_NOT_INITIALIZED");
    }
    const pkb = hexDecode(pk);
    const sigb = hexDecode(sig);
    const mh = hexDecode(sha256Hash(msg));
    const ok = schnorrVerify(pkb, sigb, mh);
    if (ok === 1) {
      near.storageSet("paused", "1");
    } else {
      die("ERR_NOT_AUTHORIZED_TO_PAUSE");
    }
  } else {
    verifyGuardianEvent("pause");
    near.storageSet("paused", "1");
  }
  return 0;
}

export function unpause() {
  const sig = near.jsonGetStr("signature");
  const expires = near.jsonGetStr("expires_at");
  const nonce = near.jsonGetStr("nonce");
  if (strLength(near.jsonGetStr("ev")) === 0) {
    if (strLength(expires) === 0) {
      die("ERR_ARG_EXPIRES");
    }
    if (strLength(nonce) === 0) {
      die("ERR_ARG_NONCE");
    }
    verifyOwner("unpause", sig, expires, nonce);
  } else {
    verifyOwnerEvent("unpause");
  }
  near.storageRemove("paused");
  return 0;
}

// ── views ────────────────────────────────────────────────────────────────

export function get_wallet() {
  const name = near.jsonGetStr("name");
  return getStr(`w:${name}`);
}

export function get_owner_nonce() {
  return numStr("ononce");
}

export function is_paused() {
  return near.jsonReturnStr(numStr("paused"));
}

export function get_version() {
  return VERSION;
}

// ── Phase 2: proposals ───────────────────────────────────────────────────

export function set_approvers() {
  const name0 = near.jsonGetStr("name");
  if (strLength(getStr(`w:${name0}`)) === 0) {
    die("ERR_WALLET_NOT_FOUND");
  }
  authOwner(`set_approvers:${name0}`);
  const name = near.jsonGetStr("name");
  const pks = near.jsonGetStr("pks");
  const thr = near.jsonGetStr("thr");
  if (strLength(pks) === 0) {
    die("ERR_APPROVERS_EMPTY");
  }
  if (strToNum(thr) === 0 || strToNum(thr) > approverCount(pks)) {
    die("ERR_THRESHOLD_INVALID");
  }
  near.storageSet(`a:${name}`, `{"thr":"${thr}","pks":"${pks}"}`);
  near.log(`approvers set: ${name}`);
  return 0;
}

export function propose() {
  const name0 = near.jsonGetStr("name");
  if (strLength(getStr(`w:${name0}`)) === 0) {
    die("ERR_WALLET_NOT_FOUND");
  }
  const id0 = strLength(getStr(`pi:${name0}`)) === 0 ? "0" : getStr(`pi:${name0}`);
  authOwner(`propose:${name0}:${id0}`);

  const name = near.jsonGetStr("name");
  const pexp = near.jsonGetStr("pexp");
  const amt = near.jsonGetStr("am");
  const to = near.jsonGetStr("rc");
  const ts = near.blockTimestamp();
  const id = strLength(getStr(`pi:${name}`)) === 0 ? "0" : getStr(`pi:${name}`);
  if (strLength(to) === 0) {
    die("ERR_MISSING_RECIPIENT");
  }
  if (strLength(amt) === 0) {
    die("ERR_MISSING_AMOUNT");
  }
  if (u128.lt(pexp, toStr(ts))) {
    die("ERR_EXPIRED");
  }
  const tk = near.jsonGetStr("tk");
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"active","exp":"${pexp}","amt":"${amt}","to":"${to}","tk":"${tk}","bl":"0","bh":"0","ac":"0"}`);
  near.storageSet(`pi:${name}`, toStr(strToNum(id) + 1));
  near.log(`proposal ${id} created for ${name}`);
  return 0;
}

export function approve() {
  const name = near.jsonGetStr("name");
  const id = near.jsonGetStr("id");
  const ix = near.jsonGetStr("ix");
  const pk = near.jsonGetStr("pubkey_hex");
  const sig = near.jsonGetStr("signature");
  const exp = near.jsonGetStr("expires_at");
  const ts = near.blockTimestamp();
  const p = getStr(`p:${name}:${id}`);
  const a = getStr(`a:${name}`);
  if (strLength(p) === 0) {
    die("ERR_PROPOSAL_NOT_FOUND");
  }
  if (strLength(a) === 0) {
    die("ERR_APPROVERS_NOT_SET");
  }
  if (strLength(pk) !== 64) {
    die("ERR_APPROVER_PK_LEN");
  }
  if (strLength(sig) !== 128) {
    die("ERR_APPROVER_SIG_LEN");
  }
  const st = jsonGet("st", p);
  const pexp = jsonGet("exp", p);
  const bl = jsonGet("bl", p);
  const ac = jsonGet("ac", p);
  const amt = jsonGet("amt", p);
  const to = jsonGet("to", p);
  const pks = jsonGet("pks", a);
  const thr = jsonGet("thr", a);
  if (st !== "active") {
    die("ERR_NOT_ACTIVE");
  }
  if (u128.lt(pexp, toStr(ts))) {
    die("ERR_PROPOSAL_EXPIRED");
  }
  if (u128.lt(exp, toStr(ts))) {
    die("ERR_SIG_EXPIRED");
  }
  const ixn = strToNum(ix);
  if (ixn < approverCount(pks)) {
    // ok
  } else {
    die("ERR_INVALID_APPROVER_INDEX");
  }
  if (nthField(pks, ixn) !== pk) {
    die("ERR_APPROVER_PK_MISMATCH");
  }
  const msg = `expires ${exp}.000000000: approve:${name}:${id}:${ix} | contract: ${near.currentAccountId()}`;
  const pkb = hexDecode(pk);
  const sigb = hexDecode(sig);
  const mh = hexDecode(sha256Hash(msg));
  const ok = schnorrVerify(pkb, sigb, mh);
  if (ok !== 1) {
    die("ERR_APPROVER_SIG_INVALID");
  }
  if (bmBitIsSet(bl, ixn) === 1) {
    die("ERR_ALREADY_APPROVED");
  }
  const nac = strToNum(ac) + 1;
  const nbl = bmBitSet(bl, ixn);
  const nsth = nac >= strToNum(thr) ? "approved" : "active";
  const tk = jsonGet("tk", p);
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"${nsth}","exp":"${pexp}","amt":"${amt}","to":"${to}","tk":"${tk}","bl":"${nbl}","bh":"0","ac":"${nac}"}`);
  near.log(`approval ${ix} on ${name}:${id}`);
  return 0;
}

export function execute() {
  const name = near.jsonGetStr("name");
  const id = near.jsonGetStr("id");
  const ts = near.blockTimestamp();
  const p = getStr(`p:${name}:${id}`);
  if (strLength(p) === 0) {
    die("ERR_PROPOSAL_NOT_FOUND");
  }
  authOwner(`execute:${name}:${id}`);
  if (jsonGet("st", p) !== "approved") {
    die("ERR_NOT_APPROVED");
  }
  if (u128.lt(jsonGet("exp", p), toStr(ts))) {
    die("ERR_PROPOSAL_EXPIRED");
  }
  const tk = jsonGet("tk", p);
  if (strLength(tk) === 0) {
    // native NEAR payout
    near.transferU128(jsonGet("to", p), jsonGet("amt", p));
  } else {
    // FT payout: NEP-141 ft_transfer on the token contract named by tk
    const pi = near.promiseBatchCreate(tk);
    near.promiseBatchActionFunctionCall(pi, "ft_transfer", `{"receiver_id":"${jsonGet("to", p)}","amount":"${jsonGet("amt", p)}","memo":"nostr-gov"}`, "1", 5000000000000);
  }
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"executed","exp":"${jsonGet("exp", p)}","amt":"${jsonGet("amt", p)}","to":"${jsonGet("to", p)}","tk":"${jsonGet("tk", p)}","bl":"${jsonGet("bl", p)}","bh":"0","ac":"${jsonGet("ac", p)}"}`);
  near.log(`proposal ${id} executed: ${name}`);
  return 0;
}

export function get_proposal() {
  return getStr(`p:${near.jsonGetStr("name")}:${near.jsonGetStr("id")}`);
}

export function get_approvers() {
  return getStr(`a:${near.jsonGetStr("name")}`);
}
