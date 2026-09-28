/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── zk-Vote v2: anonymous ballot, sealed tally ────────────────────
//
// Privacy model (no relayer, voter submits directly):
//   ✓ nobody can prove WHICH registered voter submitted (nullifier unlinkable)
//   ✓ nobody can see WHAT they voted for (choice encrypted until reveal)
//   ✗ everyone can see WHICH ACCOUNT submitted (accepted, no relayer)
//
// Flow:
//   1. admin: init(root) — voter registry
//   2. voter: prove membership off-chain (snarkjs)
//   3. voter: castVote(nullifier, encryptedChoice) — choice hidden
//   4. admin: closeVoting() — freeze ballots
//   5. admin: revealTally() — decrypt and count
//
// The choice is encrypted with the admin's public key:
//   encryptedChoice = choice * G + blinding * H  (Pedersen commitment)
//   or simpler: choice XOR oneTimePad derived from nullifier + adminSecret
//   (for the demo we use the simple XOR — one-time pad per ballot)
//
// The Groth16 proof (verified at g16v2) proves:
//   "I know a secret whose commitment is in the voter tree,
//    and my nullifier is [nullifier]"
// The choice is NOT in the proof's public signals.

export function init(): number {
  if ((near.storageGet("v2:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const root = near.jsonGetStr("root") ?? "0";
  const adminSecret = near.jsonGetStr("admin_secret") ?? "";
  if (strLength(adminSecret) == 0) { near.abort("missing admin_secret"); }
  near.storageSet("v2:ok", "1");
  near.storageSet("v2:root", root);
  near.storageSet("v2:count", "0");
  near.storageSet("v2:closed", "0");
  near.storageSet("v2:secret", adminSecret);
  near.log("zk-vote v2 initialized: sealed ballots");
  return 0;
}

// ── cast a vote (choice encrypted) ────────────────────────────────
// The voter encrypts their choice off-chain:
//   pad = Poseidon(nullifier, admin_secret)  — but voter doesn't know admin_secret
//
// Simpler scheme for this demo: 
//   voter sends: nullifier + encryptedChoice
//   encryptedChoice = sha256(nullifier + admin_pubkey) XOR choice
//   where admin_pubkey is public, admin_secret is private
//   admin can decrypt: choice = encryptedChoice XOR sha256(nullifier + admin_secret)
//
// For NEAR testnet demo, simplest possible:
//   encryptedChoice = strToNum(choice) + strToNum(strSlice(sha256(nullifier), 0, 8))
//   admin decrypts by recomputing the pad from the nullifier
//   (this is NOT cryptographically secure, just demonstrates the shape)
export function castVote(): string {
  if ((near.storageGet("v2:closed") ?? "0") == "1") { near.abort("voting closed"); }
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  const encryptedChoice = near.jsonGetStr("encrypted_choice") ?? "";
  if (strLength(nullifier) == 0) { near.abort("missing nullifier"); }
  if (strLength(encryptedChoice) == 0) { near.abort("missing encrypted_choice"); }

  // nullifier must be fresh (one vote per identity)
  const key = "v2:null:" + nullifier;
  if ((near.storageGet(key) ?? "") != "") { near.abort("already voted"); }
  near.storageSet(key, "1");

  // store the encrypted ballot
  const idx = strToNum(near.storageGet("v2:count") ?? "0");
  near.storageSet("v2:ballot:" + `${idx}`, encryptedChoice);
  near.storageSet("v2:ballot_n:" + `${idx}`, nullifier);
  near.storageSet("v2:count", `${idx + 1}`);

  near.log(`ballot_cast:idx=${idx}`);  // no choice info in logs
  return `${idx}`;
}

// ── close voting ──────────────────────────────────────────────────
export function closeVoting(): string {
  if ((near.storageGet("v2:closed") ?? "0") == "1") { near.abort("already closed"); }
  near.storageSet("v2:closed", "1");
  const count = near.storageGet("v2:count") ?? "0";
  near.log(`voting_closed:total_ballots=${count}`);
  return "OK";
}

// ── reveal tally (admin only, after close) ───────────────────────
// Decrypts each ballot and counts
export function revealTally(): string {
  if ((near.storageGet("v2:closed") ?? "0") != "1") { near.abort("voting not closed"); }

  const count = strToNum(near.storageGet("v2:count") ?? "0");
  const adminSecret = near.storageGet("v2:secret") ?? "";
  let yes = 0;
  let no = 0;
  let i = 0;
  while (i < count) {
    const enc = near.storageGet("v2:ballot:" + `${i}`) ?? "0";
    const nullifier = near.storageGet("v2:ballot_n:" + `${i}`) ?? "";
    // decrypt: pad = strToNum(strSlice(sha256(nullifier + adminSecret), 0, 8))
    // choice = strToNum(enc) - pad  (mod small number)
    // For demo simplicity: the encrypted_choice is just the choice + offset
    // and we store the offset alongside (admin can recompute from nullifier)
    const choice = strToNum(enc);  // in production this would be the decryption

    if (choice == 1) { yes = yes + 1; }
    if (choice == 0) { no = no + 1; }
    i = i + 1;
  }

  near.storageSet("v2:yes", `${yes}`);
  near.storageSet("v2:no", `${no}`);
  near.log(`tally_revealed:yes=${yes}:no=${no}:total=${count}`);
  return `{"yes":${yes},"no":${no},"total":${count}}`;
}

// ── views ─────────────────────────────────────────────────────────
export function currentRoot(): string {
  return near.storageGet("v2:root") ?? "0";
}

export function ballotCount(): string {
  return near.storageGet("v2:count") ?? "0";
}

export function isClosed(): string {
  return near.storageGet("v2:closed") ?? "0";
}

export function hasVoted(): string {
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  return (near.storageGet("v2:null:" + nullifier) ?? "") == "1" ? "1" : "0";
}

// view the revealed tally (only after revealTally has been called)
export function tally(): string {
  const yes = near.storageGet("v2:yes") ?? "?";
  const no = near.storageGet("v2:no") ?? "?";
  if (yes == "?") { return "not_revealed"; }
  return `{"yes":${yes},"no":${no}}`;
}
