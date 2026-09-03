/// <reference path="../../../ts/lisp-rlm.d.ts" />
// nostr-gov Phase-1 — TypeScript port (differential twin of main.lisp)
// Scope: legacy (owner-key) auth path. Event-auth (`ev` param) paths stub out.
// Helpers are internal; `export function` = contract method. `get_*` = view.

// ── constants ────────────────────────────────────────────────────────────
const VERSION = "2";
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

// sliding nonce window (64 nonces), bitmap lo/hi u32 pair.
// jump-on-high (2026-09-02): nonce >= base+64 is ACCEPTED and jumps the
// window (base=n, bitmap=bit0) — sparse signers can never brick the
// wallet; in-window nonces keep bit-precision replay protection.
function nonceWindowCheck(n: number) {
  const base = getNum("ononce");
  if (n < base) {
    die("ERR_NONCE_TOO_LOW");
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
  if (n >= getNum("ononce") + 64) {
    near.storageSet("ononce", toStr(n));
    near.storageSet("obm_lo", "1");
    near.storageSet("obm_hi", "0");
    return;
  }
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

const EMPTY = "";

function tagGet(tags: string, key: string): string {
  const nd = `["${key}","`;
  const i = strIndexOf(tags, nd);
  if (i === -1) {
    return EMPTY;
  }
  const rest = strSlice(tags, i + strLength(nd), strLength(tags));
  if (strLength(rest) === 0) {
    return EMPTY;
  }
  return strSlice(rest, 0, strIndexOf(rest, "\""));
}

function tagAction(tags: string) {
  return tagGet(tags, "action");
}
function tagContract(tags: string) {
  return tagGet(tags, "contract");
}
function tagNonce(tags: string) {
  return tagGet(tags, "nonce");
}
function tagExpires(tags: string) {
  return tagGet(tags, "expires");
}

function eventSerialize(pk: string, cat: string, kind: string, tags: string, content: string) {
  return `[0,"${pk}",${cat},${kind},${tags},"${content}"]`;
}

// ── v2 governance: admin set = approvers of wallet "gov" ──
// Bootstrap: while a:gov is absent, admin set = owner_npub0, thr 1.
function admPks(): string {
  const a = getStr("a:gov");
  return strLength(a) === 0 ? getStr("owner_npub0") : jsonGet("pks", a);
}
function admThr(): string {
  const a = getStr("a:gov");
  return strLength(a) === 0 ? "1" : jsonGet("thr", a);
}
function pkMember(pk: string, pks: string): i32 {
  const n = approverCount(pks);
  for (let i = 0; i < n; i++) {
    if (nthField(pks, i) === pk) { return 1; }
  }
  return 0;
}
function walletLiveCheck(name: string): void {
  if (name !== "gov" && strLength(getStr(`w:${name}`)) === 0) {
    die("ERR_WALLET_NOT_FOUND");
  }
}
function walPks(name: string): string {
  return name === "gov" ? admPks() : jsonGet("pks", getStr(`a:${name}`));
}
function walThr(name: string): string {
  return name === "gov" ? admThr() : jsonGet("thr", getStr(`a:${name}`));
}

function verifyOwnerEvent(actionStr: string) {
  const pk = near.jsonGetStr("pk") ?? "";
  const kind = near.jsonGetStr("kind") ?? "";
  const tags = near.jsonGetStr("tags") ?? "";
  const content = near.jsonGetStr("ct") ?? "";
  const sig = near.jsonGetStr("sig") ?? "";
  const cat = near.jsonGetStr("cat") ?? "";
  if (strLength(pk) !== 64) {
    die("ERR_EVENT_PK_LEN");
  }
  if (strLength(sig) !== 128) {
    die("ERR_EVENT_SIG_LEN");
  }
  if (kind !== "37500") {
    die("ERR_EVENT_KIND");
  }
  if (pkMember(pk, admPks()) !== 1) {
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
  const pk = near.jsonGetStr("pk") ?? "";
  const kind = near.jsonGetStr("kind") ?? "";
  const tags = near.jsonGetStr("tags") ?? "";
  const content = near.jsonGetStr("ct") ?? "";
  const sig = near.jsonGetStr("sig") ?? "";
  const cat = near.jsonGetStr("cat") ?? "";
  if (strLength(pk) !== 64) {
    die("ERR_EVENT_PK_LEN");
  }
  if (strLength(sig) !== 128) {
    die("ERR_EVENT_SIG_LEN");
  }
  if (kind !== "37500") {
    die("ERR_EVENT_KIND");
  }
  if (pkMember(pk, admPks()) !== 1) {
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

// ── lifecycle ────────────────────────────────────────────────────────────

export function init() {
  if (strLength(getStr("owner_npub0")) !== 0) {
    die("ERR_ALREADY_INITIALIZED");
  }
  const npub = near.jsonGetStr("npub") ?? "";
  if (strLength(npub) !== 64) {
    die("ERR_BAD_NPUB");
  }
  near.storageSet("owner_npub0", npub);
  return 0;
}

export function create_wallet() {
  // pause gate FIRST and dialect-independent (2026-09-02 live catch).
  if (strLength(getStr("paused")) !== 0) {
    die("ERR_PAUSED");
  }
  const name = near.jsonGetStr("name") ?? "";
  // "gov" is the implicit governance wallet — not creatable
  if (name === "gov") {
    die("ERR_NAME_RESERVED");
  }
  // v2: admin actions are EVENT-auth only — the legacy single-key
  // dialect on admin paths was the pope backdoor
  if (strLength(near.jsonGetStr("ev") ?? "") === 0) {
    die("ERR_EV_REQUIRED");
  }
  verifyOwnerEvent(`create_wallet:${name}`);
  if (!near.depositGte(1001882102603448320, 27105)) {
    die("ERR_STORAGE_DEPOSIT");
  }
  if (strLength(getStr(`w:${name}`)) !== 0) {
    die("ERR_WALLET_EXISTS");
  }
  if (nameValid(name) === 0) {
    die("ERR_NAME_INVALID_CHARS");
  }
  // v2: wallets are BORN with their approver set (admin chooses at
  // creation — nothing at stake yet). Later rotation is approver-
  // gated; the admin can never swap approvers of a live wallet.
  const pks = near.jsonGetStr("pks") ?? "";
  const thr = near.jsonGetStr("thr") ?? "";
  if (strLength(pks) === 0) {
    die("ERR_APPROVERS_EMPTY");
  }
  if (strToNum(thr) === 0 || strToNum(thr) > approverCount(pks)) {
    die("ERR_THRESHOLD_INVALID");
  }
  near.storageSet(`a:${name}`, `{"thr":"${thr}","pks":"${pks}"}`);
  const creator = near.predecessorAccountId();
  const createdAt = near.blockTimestamp();
  const deposit = near.attachedDepositU128();
  near.storageSet(`w:${name}`, `{"name":"${name}","creator":"${creator}","created_at":"${createdAt}","deposit":"${deposit}"}`);
  near.log(`wallet created: ${name}`);
  return 0;
}

export function pause() {
  // v2: any admin (1-of-N) trips the breaker — cheap on purpose.
  if (strLength(near.jsonGetStr("ev") ?? "") === 0) {
    die("ERR_EV_REQUIRED");
  }
  verifyGuardianEvent("pause");
  near.storageSet("paused", "1");
  return 0;
}

// ── views ────────────────────────────────────────────────────────────────

export function get_wallet() {
  const name = near.jsonGetStr("name") ?? "";
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

export function propose() {
  // v2: proposals are TYPED. act "" = payout, "appr" = rotate this
  // wallet's approvers (current approvers must approve), "unp" =
  // unpause (governance wallet only). Admin proposes; the wallet's own
  // approvers still gate execution.
  const name0 = near.jsonGetStr("name") ?? "";
  walletLiveCheck(name0);
  // proposal id = the event NONCE (2026-09-02): unique, signed inside
  // the action tag, replay-protected — no pi:<name> counter write
  const id0 = tagNonce(near.jsonGetStr("tags") ?? "");
  if (strLength(id0) === 0) {
    die("ERR_EVENT_NONCE");
  }
  if (strLength(near.jsonGetStr("ev") ?? "") === 0) {
    die("ERR_EV_REQUIRED");
  }
  verifyOwnerEvent(`propose:${name0}:${id0}`);

  const name = near.jsonGetStr("name") ?? "";
  const pexp = near.jsonGetStr("pexp") ?? "";
  const amt = near.jsonGetStr("am") ?? "";
  const to = near.jsonGetStr("rc") ?? "";
  const act = near.jsonGetStr("act") ?? "";
  const np = near.jsonGetStr("np") ?? "";
  const nt = near.jsonGetStr("nt") ?? "";
  const ts = near.blockTimestamp();
  const id = tagNonce(near.jsonGetStr("tags") ?? "");
  if (u128.lt(pexp, toStr(ts))) {
    die("ERR_EXPIRED");
  }
  if (act === "") {
    if (strLength(to) === 0) {
      die("ERR_MISSING_RECIPIENT");
    }
    if (strLength(amt) === 0) {
      die("ERR_MISSING_AMOUNT");
    }
  }
  if (act === "appr") {
    if (strLength(np) === 0) {
      die("ERR_APPROVERS_EMPTY");
    }
    if (strToNum(nt) === 0 || strToNum(nt) > approverCount(np)) {
      die("ERR_THRESHOLD_INVALID");
    }
  }
  if (act === "unp") {
    if (name !== "gov") {
      die("ERR_NOT_GOVERNANCE");
    }
  }
  if (act !== "" && act !== "appr" && act !== "unp") {
    die("ERR_ACTION_UNKNOWN");
  }
  // while paused, ONLY the unpause recovery path may run
  if (strLength(getStr("paused")) !== 0) {
    if (act !== "unp") {
      die("ERR_PAUSED");
    }
  }
  // nil-guard: jsonGetStr(missing) is nil, not ""
  const tk = near.jsonGetStr("tk") ?? "";
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"active","exp":"${pexp}","amt":"${amt}","to":"${to}","tk":"${tk}","act":"${act}","np":"${np}","nt":"${nt}","bl":"0","bh":"0","ac":"0"}`);
  near.log(`proposal ${id} (${act}) created for ${name}`);
  return 0;
}

export function approve() {
  const name = near.jsonGetStr("name") ?? "";
  const id = near.jsonGetStr("id") ?? "";
  const ix = near.jsonGetStr("ix") ?? "";
  const pk = near.jsonGetStr("pubkey_hex") ?? "";
  const sig = near.jsonGetStr("signature") ?? "";
  const exp = near.jsonGetStr("expires_at") ?? "";
  const ts = near.blockTimestamp();
  const p = getStr(`p:${name}:${id}`);
  const a = getStr(`a:${name}`);
  if (strLength(p) === 0) {
    die("ERR_PROPOSAL_NOT_FOUND");
  }
  // gov proposals are approved by the ADMIN set (bootstrap:
  // owner_npub0 alone until admins rotate themselves)
  if (strLength(a) === 0 && name !== "gov") {
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
  const pks = walPks(name);
  const thr = walThr(name);
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
  // native i64 bitmap — bl is a decimal u64, approver ix < 64
  const bln = strToNum(bl);
  if ((bln & (1 << ixn)) !== 0) {
    die("ERR_ALREADY_APPROVED");
  }
  const nac = strToNum(ac) + 1;
  const nbl = toStr(bln | (1 << ixn));
  const nsth = nac >= strToNum(thr) ? "approved" : "active";
  const tk = jsonGet("tk", p);
  const act = jsonGet("act", p);
  const np = jsonGet("np", p);
  const nt = jsonGet("nt", p);
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"${nsth}","exp":"${pexp}","amt":"${amt}","to":"${to}","tk":"${tk}","act":"${act}","np":"${np}","nt":"${nt}","bl":"${nbl}","bh":"0","ac":"${nac}"}`);
  near.log(`approval ${ix} on ${name}:${id}`);
  return 0;
}

export function execute() {
  const name = near.jsonGetStr("name") ?? "";
  const id = near.jsonGetStr("id") ?? "";
  const ts = near.blockTimestamp();
  const p = getStr(`p:${name}:${id}`);
  if (strLength(p) === 0) {
    die("ERR_PROPOSAL_NOT_FOUND");
  }
  if (strLength(near.jsonGetStr("ev") ?? "") === 0) {
    die("ERR_EV_REQUIRED");
  }
  verifyOwnerEvent(`execute:${name}:${id}`);
  if (jsonGet("st", p) !== "approved") {
    die("ERR_NOT_APPROVED");
  }
  if (u128.lt(jsonGet("exp", p), toStr(ts))) {
    die("ERR_PROPOSAL_EXPIRED");
  }
  const act = jsonGet("act", p);
  const expp = jsonGet("exp", p);
  const amtp = jsonGet("amt", p);
  const top = jsonGet("to", p);
  const tkp = jsonGet("tk", p);
  const npp = jsonGet("np", p);
  const ntp = jsonGet("nt", p);
  const blp = jsonGet("bl", p);
  const acp = jsonGet("ac", p);
  // while paused, only the unpause recovery path may execute
  if (strLength(getStr("paused")) !== 0) {
    if (act !== "unp") {
      die("ERR_PAUSED");
    }
  }
  if (act === "appr") {
    near.storageSet(`a:${name}`, `{"thr":"${jsonGet("nt", p)}","pks":"${jsonGet("np", p)}"}`);
    near.log(`approvers rotated: ${name}`);
  }
  if (act === "unp") {
    near.storageRemove("paused");
    near.log("contract unpaused");
  }
  const tk = jsonGet("tk", p);
  if (act === "" && strLength(tk) === 0) {
    // native NEAR payout
    near.transferU128(jsonGet("to", p), jsonGet("amt", p));
  }
  if (act === "" && strLength(tk) !== 0) {
    // FT payout: NEP-141 ft_transfer on the token contract named by tk
    const pi = near.promiseBatchCreate(tk);
    near.promiseBatchActionFunctionCall(pi, "ft_transfer", `{"receiver_id":"${jsonGet("to", p)}","amount":"${jsonGet("amt", p)}","memo":"nostr-gov"}`, "1", 5000000000000);
  }
  near.storageSet(`p:${name}:${id}`, `{"id":"${id}","st":"executed","exp":"${expp}","amt":"${amtp}","to":"${top}","tk":"${tkp}","act":"${act}","np":"${npp}","nt":"${ntp}","bl":"${blp}","bh":"0","ac":"${acp}"}`);
  near.log(`proposal ${id} (${act}) executed: ${name}`);
  return 0;
}

export function get_proposal() {
  return getStr(`p:${near.jsonGetStr("name") ?? ""}:${near.jsonGetStr("id") ?? ""}`);
}

export function get_approvers() {
  return getStr(`a:${near.jsonGetStr("name") ?? ""}`);
}

// ── public self-test (port of lisp test_verify_nostr) ───────────────────
export function test_verify_nostr() {
  const msg = near.jsonGetStr("message") ?? "";
  const pk = near.jsonGetStr("pubkey_hex") ?? "";
  const sig = near.jsonGetStr("sig_hex") ?? "";
  const ok = schnorrVerify(hexDecode(pk), hexDecode(sig), hexDecode(sha256Hash(msg)));
  if (ok !== 1) {
    die("Invalid schnorr signature: verification failed");
  }
  return "true";
}
