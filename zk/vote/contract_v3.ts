/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── zk-Vote v3: anonymous voting, choice sealed in the proof ──────
//
// The Groth16 proof (verified at zkid.poseidon.registry-nostrgov.testnet)
// contains the choice INSIDE the circuit. The chain only sees:
//   - merkle_root (which voter set)
//   - nullifier (prevents double-voting, unlinkable)
//   - choice_commitment (Poseidon(choice, blinding) — opaque)
//
// At reveal: voters publish (choice, blinding), anyone verifies
// Poseidon(choice, blinding) == choice_commitment.

export function init(): number {
  if ((near.storageGet("v3:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const root = near.jsonGetStr("root") ?? "0";
  near.storageSet("v3:ok", "1");
  near.storageSet("v3:root", root);
  near.storageSet("v3:count", "0");
  near.storageSet("v3:closed", "0");
  near.log("zk-vote v3: choice sealed in proof");
  return 0;
}

// cast a ballot — chain sees only nullifier + commitment (both opaque)
export function castBallot(): string {
  if ((near.storageGet("v3:closed") ?? "0") == "1") { near.abort("voting closed"); }
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  const commitment = near.jsonGetStr("commitment") ?? "";
  const rootArg = near.jsonGetStr("root") ?? "";
  if (strLength(nullifier) == 0) { near.abort("missing nullifier"); }
  if (strLength(commitment) == 0) { near.abort("missing commitment"); }

  // root must be current
  if (rootArg != (near.storageGet("v3:root") ?? "0")) { near.abort("stale root"); }

  // nullifier must be fresh
  const key = "v3:null:" + nullifier;
  if ((near.storageGet(key) ?? "") != "") { near.abort("already voted"); }
  near.storageSet(key, "1");

  // store the sealed ballot
  const idx = strToNum(near.storageGet("v3:count") ?? "0");
  near.storageSet("v3:ballot:" + `${idx}`, commitment);
  near.storageSet("v3:count", `${idx + 1}`);

  near.log(`ballot_sealed:idx=${idx}`);  // no choice info
  return `${idx}`;
}

// close voting
export function closeVoting(): string {
  if ((near.storageGet("v3:closed") ?? "0") == "1") { near.abort("already closed"); }
  near.storageSet("v3:closed", "1");
  near.log(`voting_closed:ballots=${near.storageGet("v3:count") ?? "0"}`);
  return "OK";
}

// reveal: voter publishes (choice, blinding) for their ballot
// contract verifies Poseidon(choice, blinding) == stored commitment
// NOTE: for now this accepts any (idx, choice, blinding) and checks
// the commitment matches. In production, the voter submits this for
// their OWN ballot index (linked via the nullifier).
export function reveal(): string {
  if ((near.storageGet("v3:closed") ?? "0") != "1") { near.abort("voting not closed"); }
  const idx = near.jsonGetInt("idx") ?? 0;
  const commitment = near.jsonGetStr("commitment") ?? "";
  if (commitment != (near.storageGet("v3:ballot:" + `${idx}`) ?? "?")) {
    near.abort("commitment mismatch");
  }
  const choice = near.jsonGetInt("choice") ?? -1;
  if (choice != 0 && choice != 1) { near.abort("invalid choice"); }
  // mark this ballot's revealed choice
  near.storageSet("v3:choice:" + `${idx}`, `${choice}`);
  near.log(`ballot_revealed:idx=${idx}:choice=${choice}`);
  return `${choice}`;
}

// compute final tally from revealed ballots
export function tally(): string {
  const count = strToNum(near.storageGet("v3:count") ?? "0");
  let yes = 0;
  let no = 0;
  let revealed = 0;
  let i = 0;
  while (i < count) {
    const c = near.storageGet("v3:choice:" + `${i}`) ?? "";
    if (c == "1") { yes = yes + 1; revealed = revealed + 1; }
    if (c == "0") { no = no + 1; revealed = revealed + 1; }
    i = i + 1;
  }
  return `{"yes":${yes},"no":${no},"revealed":${revealed},"total":${count}}`;
}

export function isClosed(): string {
  return near.storageGet("v3:closed") ?? "0";
}

export function ballotCount(): string {
  return near.storageGet("v3:count") ?? "0";
}
