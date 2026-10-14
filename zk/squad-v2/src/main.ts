/// <reference path="../types/lisp-rlm.d.ts" />

// ── zk-squad v2: sealed-tally threshold voting (Construction #2 skeleton) ──
//
// Architecture (SPEC.md, Arch A — native hosts only):
//   CHAIN = bulletin board: roster root, deadline, nullifier dedup,
//           anonymous tallier-key registry, opaque PVSS transcript blobs,
//           quorum close (t distinct ed25519 sigs — native host verify),
//           challenge window, execute.
//   OFF-CHAIN = Λ_indv 3PVSS (lib/pvss.js): ballots verified by every
//           member/watcher; closers compute V* = G^Σv off-chain and sign it.
//
// zk-identity circuit v2: public = [root, nullifier, claim=1, nonce]
//   nonce=1 → enroll (registers tallier pk, anonymous)
//   nonce=2 → ballot (sealed vote blob, anonymous)
// Nullifier = Poseidon(secret, nonce) — action-separated, one use each.
//
// What the chain sees: HOW MANY distinct members enrolled/balloted (counts),
// blobs it cannot read, and t sigs proving distinct members endorse a tally.
// Never: WHO voted, or WHAT any individual sealed.

const G1_LEN: number = 128;
const G2_LEN: number = 256;
const SCALAR_LEN: number = 64;
const PK_LEN: number = 64;     // ed25519 pk hex (32B)
const SIG_LEN: number = 128;   // ed25519 sig hex (64B)
const ONE_HEX: string = "0100000000000000000000000000000000000000000000000000000000000000";
const NONCE_ENROLL: string = "0100000000000000000000000000000000000000000000000000000000000000";
const NONCE_BALLOT: string = "0200000000000000000000000000000000000000000000000000000000000000";

// ── init: squad parameters + Groth16 vk (4 public signals) ────────────
export function init(): number {
  if ((near.storageGet("sq2:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const rootHex = near.jsonGetStr("root_hex") ?? "";
  const threshold = near.jsonGetInt("threshold") ?? 0;
  const members = near.jsonGetInt("members") ?? 0;
  const t0 = near.jsonGetInt("t0") ?? 0;        // ballot window open (unix s)
  const t1 = near.jsonGetInt("t1") ?? 0;        // ballot window close (unix s)
  const challenge = near.jsonGetInt("challenge_secs") ?? 0;
  if (strLength(rootHex) != SCALAR_LEN) { near.abort("root_hex len"); return 0; }
  if (threshold <= 0 || members < threshold) { near.abort("bad threshold"); return 0; }
  if (t1 <= t0 || challenge <= 0) { near.abort("bad window"); return 0; }

  const a1 = near.jsonGetStr("alpha1") ?? "";
  const b2 = near.jsonGetStr("beta2") ?? "";
  const g2 = near.jsonGetStr("gamma2") ?? "";
  const d2 = near.jsonGetStr("delta2") ?? "";
  const n = near.jsonGetInt("n") ?? 0;
  if (n != 4) { near.abort("vk: expect 4 public signals [root,nullifier,claim,nonce]"); return 0; }
  if (strLength(a1) != G1_LEN || strLength(b2) != G2_LEN) { near.abort("vk len a/b"); return 0; }
  if (strLength(g2) != G2_LEN || strLength(d2) != G2_LEN) { near.abort("vk len g/d"); return 0; }
  const IC = near.jsonArr("ic");
  if (IC.length != n + 1) { near.abort("ic_count"); return 0; }

  near.storageSet("sq2:ok", "1");
  near.storageSet("sq2:root", rootHex);
  near.storageSet("sq2:threshold", `${threshold}`);
  near.storageSet("sq2:members", `${members}`);
  near.storageSet("sq2:t0", `${t0}`);
  near.storageSet("sq2:t1", `${t1}`);
  near.storageSet("sq2:challenge", `${challenge}`);
  near.storageSet("sq2:enrolled", "0");
  near.storageSet("sq2:ballots", "0");
  near.storageSet("sq2:phase", "window");   // window → closed → executed | disputed
  near.storageSet("vk:alpha1", a1);
  near.storageSet("vk:beta2", b2);
  near.storageSet("vk:gamma2", g2);
  near.storageSet("vk:delta2", d2);
  near.storageSet("vk:n", `${n}`);
  let i = 0;
  while (i <= n) {
    if (strLength(IC[i]) != G1_LEN) { near.abort(`ic${i}_len`); return 0; }
    near.storageSet("vk:ic:" + `${i}`, IC[i]);
    i = i + 1;
  }
  near.log(`zk-squad-v2 armed: members=${members} threshold=${threshold} window=[${t0},${t1}]`);
  return 0;
}


// ns timestamp string -> seconds (tagged num range is +/-2^60 — ns overflows;
// slice off the last 9 digits instead of dividing)
function nowSecs(): number {
  const ns = near.blockTimestamp();
  return strToNum(strSlice(ns, 0, strLength(ns) - 9));
}

// ── shared: Groth16 gate over [root, nullifier, claim=1, nonce] ────────
function gate(negA: string, B: string, C: string, ins: { length: number; [k: number]: string }): number {
  if (ins.length != 4) { near.abort("inputs: [root,nullifier,claim,nonce]"); return 0; }
  if (ins[0] != (near.storageGet("sq2:root") ?? "")) { near.abort("wrong root"); return 0; }
  if (ins[2] != ONE_HEX) { near.abort("claim must be 1"); return 0; }
  let mx = strCat(near.storageGet("vk:ic:0") ?? "", ONE_HEX);
  let i = 1;
  while (i <= 4) {
    if (strLength(ins[i - 1]) != SCALAR_LEN) { near.abort("inputs:len"); return 0; }
    mx = strCat(mx, near.storageGet("vk:ic:" + `${i}`) ?? "", ins[i - 1]);
    i = i + 1;
  }
  const pub = near.altBn128G1Multiexp(mx);
  const check = strCat(
    negA, B,
    near.storageGet("vk:alpha1") ?? "", near.storageGet("vk:beta2") ?? "",
    pub, near.storageGet("vk:gamma2") ?? "",
    C, near.storageGet("vk:delta2") ?? ""
  );
  return near.altBn128PairingCheck(check);
}

// ── enroll: anonymous tallier-key registration (nonce=1) ──────────────
//   {negA,B,C,inputs:[root,nullifier,1,1], pk_hex}
export function enroll(): number {
  if ((near.storageGet("sq2:ok") ?? "") == "") { near.abort("not init"); return 0; }
  if (nowSecs() >= strToNum(near.storageGet("sq2:t0") ?? "0")) {
    near.abort("enroll closed"); return 0;   // enroll before the window opens
  }
  const pk = near.jsonGetStr("pk_hex") ?? "";
  if (strLength(pk) != PK_LEN) { near.abort("pk len"); return 0; }
  if ((near.storageGet("sq2:pk:" + pk) ?? "") != "") { near.abort("pk already registered"); return 0; }

  const ins = near.jsonArr("inputs");
  if (ins[3] != NONCE_ENROLL) { near.abort("nonce must be 1 (enroll)"); return 0; }
  const key = "sq2:n1:" + ins[1];
  if ((near.storageGet(key) ?? "") != "") { near.abort("already enrolled"); return 0; }

  const negA = near.jsonGetStr("negA") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  if (strLength(negA) != G1_LEN || strLength(B) != G2_LEN || strLength(C) != G1_LEN) {
    near.abort("proof len"); return 0;
  }
  if (gate(negA, B, C, ins) != 1) { near.abort("BAD:pairing"); return 0; }

  near.storageSet(key, "1");
  near.storageSet("sq2:pk:" + pk, "1");
  const count = strToNum(near.storageGet("sq2:enrolled") ?? "0") + 1;
  near.storageSet("sq2:enrolled", `${count}`);
  near.log(`enrolled:${count}/${near.storageGet("sq2:members")} (anonymously)`);
  return 0;
}

// ── ballot: anonymous sealed vote (nonce=2) ───────────────────────────
//   {negA,B,C,inputs:[root,nullifier,1,2], blob_hex}  blob = opaque PVSS
export function ballot(): number {
  if ((near.storageGet("sq2:ok") ?? "") == "") { near.abort("not init"); return 0; }
  if ((near.storageGet("sq2:phase") ?? "") != "window") { near.abort("not in window"); return 0; }
  const now = nowSecs();
  if (now < strToNum(near.storageGet("sq2:t0") ?? "0")) { near.abort("window not open"); return 0; }
  if (now >= strToNum(near.storageGet("sq2:t1") ?? "0")) { near.abort("window closed"); return 0; }

  const blob = near.jsonGetStr("blob_hex") ?? "";
  const members = strToNum(near.storageGet("sq2:members") ?? "0");
  const seq = strToNum(near.storageGet("sq2:ballots") ?? "0");
  if (seq >= members) { near.abort("ballot cap reached"); return 0; }
  if (strLength(blob) < 64) { near.abort("blob too short"); return 0; }
  if (strLength(blob) > 4096) { near.abort("blob too large"); return 0; }

  const ins = near.jsonArr("inputs");
  if (ins[3] != NONCE_BALLOT) { near.abort("nonce must be 2 (ballot)"); return 0; }
  const key = "sq2:n2:" + ins[1];
  if ((near.storageGet(key) ?? "") != "") { near.abort("already voted"); return 0; }

  const negA = near.jsonGetStr("negA") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  if (strLength(negA) != G1_LEN || strLength(B) != G2_LEN || strLength(C) != G1_LEN) {
    near.abort("proof len"); return 0;
  }
  if (gate(negA, B, C, ins) != 1) { near.abort("BAD:pairing"); return 0; }

  near.storageSet(key, "1");
  near.storageSet("sq2:ballot:" + `${seq}`, blob);
  near.storageSet("sq2:ballots", `${seq + 1}`);
  near.log(`ballot:${seq + 1}/${members} (sealed)`);  // count only, never content
  return 0;
}

// ── close: quorum of t distinct talliers endorses the tally ───────────
//   {vstar_hex, sum, result:"YES"|"NO", pks:[...], sigs:[...]}
//   statement = sha256("zk-squad-v2:close|" + root + "|" + vstar + "|" + sum + "|" + result)
export function close(): number {
  if ((near.storageGet("sq2:phase") ?? "") != "window") { near.abort("already closed"); return 0; }
  const now = nowSecs();
  if (now < strToNum(near.storageGet("sq2:t1") ?? "0")) { near.abort("window still open"); return 0; }

  const vstar = near.jsonGetStr("vstar_hex") ?? "";
  const sum = near.jsonGetInt("sum") ?? -1;
  const result = near.jsonGetStr("result") ?? "";
  const members = strToNum(near.storageGet("sq2:members") ?? "0");
  if (strLength(vstar) != PK_LEN) { near.abort("vstar len"); return 0; }
  if (sum < 0 || sum > members) { near.abort("sum range"); return 0; }
  if (result != "YES" && result != "NO") { near.abort("result YES|NO"); return 0; }
  // claimed result must be internally consistent with threshold
  const threshold = strToNum(near.storageGet("sq2:threshold") ?? "0");
  if (result == "YES" && sum < threshold) { near.abort("YES but sum<threshold"); return 0; }
  if (result == "NO" && sum >= threshold) { near.abort("NO but sum>=threshold"); return 0; }

  const stmt = near.sha256(strCat("zk-squad-v2:close|",
    near.storageGet("sq2:root") ?? "", "|", vstar, "|", `${sum}`, "|", result));

  const pks = near.jsonArr("pks");
  const sigs = near.jsonArr("sigs");
  const threshold2 = strToNum(near.storageGet("sq2:threshold") ?? "0");
  if (pks.length != sigs.length || pks.length < threshold2) { near.abort("need t sigs"); return 0; }
  if (pks.length > members) { near.abort("too many sigs"); return 0; }

  let i = 0;
  while (i < pks.length) {
    if (strLength(pks[i]) != PK_LEN || strLength(sigs[i]) != SIG_LEN) { near.abort("pk/sig len"); return 0; }
    if ((near.storageGet("sq2:pk:" + pks[i]) ?? "") == "") { near.abort("pk not enrolled"); return 0; }
    if ((near.storageGet("sq2:closedby:" + pks[i]) ?? "") != "") { near.abort("dup closer pk"); return 0; }
    // runtime form: near.ed25519_verify takes RAW BYTES (hexDecode) — the
    // camelCase spelling only accepts compile-time literals
    // near.sha256 returns RAW digest bytes — pass straight to the raw-byte
    // ed25519 form (no hexDecode on the message); sig/pk come in as hex
    if (near.ed25519_verify(hexDecode(sigs[i]), stmt, hexDecode(pks[i])) != 1) { near.abort(`BAD:sig:${i}`); return 0; }
    near.storageSet("sq2:closedby:" + pks[i], "1");
    i = i + 1;
  }

  near.storageSet("sq2:phase", "closed");
  near.storageSet("sq2:vstar", vstar);
  near.storageSet("sq2:sum", `${sum}`);
  near.storageSet("sq2:result", result);
  near.storageSet("sq2:closers", `${pks.length}`);
  const challengeEnd = now + strToNum(near.storageGet("sq2:challenge") ?? "0");
  near.storageSet("sq2:challenge_end", `${challengeEnd}`);
  near.log(`closed: result=${result} sum=${sum} closers=${pks.length} — challenge until ${challengeEnd}`);
  return 0;
}

// ── dispute: any observer flags a bad close during the window ─────────
export function dispute(): number {
  if ((near.storageGet("sq2:phase") ?? "") != "closed") { near.abort("not closed"); return 0; }
  if (nowSecs() >= strToNum(near.storageGet("sq2:challenge_end") ?? "0")) {
    near.abort("challenge window over"); return 0;
  }
  near.storageSet("sq2:phase", "disputed");
  near.log("DISPUTED: tally flagged — result frozen, manual adjudication");
  return 0;
}

// ── execute: after the challenge window, fire if YES ──────────────────
export function execute(): number {
  const phase = near.storageGet("sq2:phase") ?? "";
  if (phase == "disputed") { near.abort("disputed — frozen"); return 0; }
  if (phase != "closed") { near.abort("not closed"); return 0; }
  if (nowSecs() < strToNum(near.storageGet("sq2:challenge_end") ?? "0")) {
    near.abort("challenge window still open"); return 0;
  }
  near.storageSet("sq2:phase", "executed");
  if ((near.storageGet("sq2:result") ?? "") == "YES") {
    near.storageSet("sq2:fired", "1");
    near.log(`FIRED: sealed tally YES (sum=${near.storageGet("sq2:sum")} ≥ threshold)`);
  } else {
    near.log(`HELD: sealed tally NO (sum=${near.storageGet("sq2:sum")} < threshold)`);
  }
  return 0;
}

// ── views ─────────────────────────────────────────────────────────────
export function status(): string {
  const sumv = near.storageGet("sq2:sum") ?? "";
  const sumOut = sumv.length > 0 ? sumv : "-1";
  const ok = (near.storageGet("sq2:ok") ?? "") != "" ? "1" : "0";
  const members = near.storageGet("sq2:members") ?? "0";
  const threshold = near.storageGet("sq2:threshold") ?? "0";
  const enrolled = near.storageGet("sq2:enrolled") ?? "0";
  const ballots = near.storageGet("sq2:ballots") ?? "0";
  const phase = near.storageGet("sq2:phase") ?? "none";
  const result = near.storageGet("sq2:result") ?? "";
  const closers = near.storageGet("sq2:closers") ?? "0";
  const fired = near.storageGet("sq2:fired") ?? "0";
  return `{"armed":${ok},"members":${members},"threshold":${threshold},` +
    `"enrolled":${enrolled},"ballots":${ballots},"phase":"${phase}",` +
    `"result":"${result}","sum":${sumOut},"closers":${closers},"fired":${fired}}`;
}

export function ballot_blob(idx: number): string {
  return near.storageGet("sq2:ballot:" + `${idx}`) ?? "";
}
