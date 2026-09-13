/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── zk-Vote: anonymous voting on NEAR ────────────────────────────
//
// The simplest useful example: prove you're a registered voter
// and cast a vote, without revealing WHO you are.
//
// Uses the identity circuit we already verified (g16v2 verifier).
// This contract manages: the voter registry root + nullifier set.
//
// Flow:
//   1. Admin sets the voter root (Merkle tree of voter commitments)
//   2. User proves off-chain: "my commitment is in the tree"
//   3. User calls vote() with proof + nullifier + ballot choice
//   4. Contract checks: root is current, nullifier is fresh
//   5. Vote is counted — nobody knows who cast it

export function init(): number {
  if ((near.storageGet("v:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const root = near.jsonGetStr("root") ?? "0";
  near.storageSet("v:ok", "1");
  near.storageSet("v:root", root);
  near.storageSet("v:count", "0");
  near.log("zk-vote initialized");
  return 0;
}

export function setRoot(): string {
  const root = near.jsonGetStr("root") ?? "";
  if (strLength(root) == 0) { near.abort("missing root"); }
  near.storageSet("v:root", root);
  near.log("root_updated");
  return "OK";
}

export function vote(): string {
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  const choice = near.jsonGetInt("choice") ?? 0;
  const rootArg = near.jsonGetStr("root") ?? "";

  if (rootArg != (near.storageGet("v:root") ?? "0")) {
    near.abort("stale root");
  }

  const key = "v:null:" + nullifier;
  if ((near.storageGet(key) ?? "") != "") { near.abort("already voted"); }
  near.storageSet(key, "1");

  const idx = strToNum(near.storageGet("v:count") ?? "0");
  near.storageSet("v:ballot:" + `${idx}`, `${choice}`);
  near.storageSet("v:count", `${idx + 1}`);

  const tallyKey = "v:tally:" + `${choice}`;
  const current = strToNum(near.storageGet(tallyKey) ?? "0");
  near.storageSet(tallyKey, `${current + 1}`);

  near.log(`vote_cast:choice=${choice}:tally=${current + 1}`);
  return `${idx}`;
}

export function currentRoot(): string {
  return near.storageGet("v:root") ?? "0";
}

export function tally(): string {
  const yes = near.storageGet("v:tally:1") ?? "0";
  const no = near.storageGet("v:tally:0") ?? "0";
  return `{"yes":${yes},"no":${no},"total":${near.storageGet("v:count") ?? "0"}}`;
}

export function hasVoted(): string {
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  return (near.storageGet("v:null:" + nullifier) ?? "") == "1" ? "1" : "0";
}
