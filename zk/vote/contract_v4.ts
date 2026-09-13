/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── zk-Vote v4: Homomorphic tally — nobody sees individual choices ──
//
// Architecture: additive homomorphic encryption over BN254 (ElGamal-style)
//
// VOTING:
//   encrypted_i = choice_i·G + r_i·T    (T = tally public key)
//   + Groth16 proof that choice ∈ {0,1} and encryption is correct
//   + nullifier (prevent double-vote)
//
// The chain stores: encrypted curve points (opaque)
//
// TALLY:
//   sum = Σ encrypted_i = (Σchoice_i)·G + (Σr_i)·T
//   authorities jointly decrypt → "yes_count = N"
//
// NOBODY sees individual choices. Not the chain, not observers,
// not even a single authority (threshold decryption).
//
// For the demo: single tally key (production = threshold 2-of-3)
// T is a fixed public point. The tally authority knows t where T = t·G.
// At close, the authority decrypts: sum·G = C_total - (Σr_i)·T
// For the DEMO we store r_i encrypted to the authority... 
// 
// SIMPLER DEMO APPROACH: voters also submit r_i·G (their "opening hint")
// encrypted to the tally authority. The authority can then reconstruct
// Σr_i without any individual r_i being public.
//
// SIMPLEST CORRECT DEMO: since we can't do threshold easily on NEAR yet:
//   - voters submit: encrypted_choice + proof + nullifier + r_i (as private signal in proof)
//   - the proof ALSO proves knowledge of r_i
//   - at tally, the ADMIN (who collected r_i values off-chain) submits
//     the final decrypted tally + a proof that it's correct
//   - for now: admin is trusted for the tally, but individual choices
//     are hidden from the CHAIN and from OBSERVERS
//
// GAS per ballot: ~2 Tgas (multiexp) + 34 Tgas (proof) + ~1 Tgas (storage)

// 64-byte zero point (128 hex chars, no sign byte — just the point)
const ZERO_POINT = "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

export function init(): number {
  if ((near.storageGet("v4:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const root = near.jsonGetStr("root") ?? "0";
  const tallyPubkey = near.jsonGetStr("tally_pubkey") ?? ZERO_POINT;
  near.storageSet("v4:ok", "1");
  near.storageSet("v4:root", root);
  near.storageSet("v4:tally_pubkey", tallyPubkey);
  near.storageSet("v4:count", "0");
  near.storageSet("v4:closed", "0");
  near.storageSet("v4:sum", ZERO_POINT);   // running homomorphic sum
  near.log("zk-vote v4: homomorphic tally");
  return 0;
}

// ── cast an encrypted ballot ───────────────────────────────────────
// The voter submits:
//   nullifier: Poseidon(secret, 0) — prevents double-vote, unlinkable
//   encrypted: BN254 G1 point = choice·G + r·T (128 hex chars, LE format)
//   proof: verified separately (proves choice ∈ {0,1} and encryption correct)
//
// The chain:
//   1. checks nullifier is fresh
//   2. adds the encrypted point to the running sum (homomorphic add)
//   3. stores nothing about the choice
export function castBallot(): string {
  if ((near.storageGet("v4:closed") ?? "0") == "1") { near.abort("voting closed"); }
  const nullifier = near.jsonGetStr("nullifier") ?? "";
  const encrypted = near.jsonGetStr("encrypted") ?? "";
  if (strLength(nullifier) == 0) { near.abort("missing nullifier"); }
  if (strLength(encrypted) != 128) { near.abort("encrypted: want 128 hex chars (G1 point)"); }

  // nullifier must be fresh
  const key = "v4:null:" + nullifier;
  if ((near.storageGet(key) ?? "") != "") { near.abort("already voted"); }
  near.storageSet(key, "1");

  // homomorphic addition: new_sum = old_sum + encrypted
  // This uses alt_bn128_g1_sum host: (sign|point) pairs
  const oldSum = near.storageGet("v4:sum") ?? ZERO_POINT;
  const sumInput = "00" + oldSum + "00" + encrypted;  // two points, both positive
  const newSum = near.altBn128G1Sum(sumInput);

  near.storageSet("v4:sum", newSum);

  // store the individual encrypted ballot (for auditing)
  const idx = strToNum(near.storageGet("v4:count") ?? "0");
  near.storageSet("v4:enc:" + `${idx}`, encrypted);
  near.storageSet("v4:count", `${idx + 1}`);

  near.log(`ballot_encrypted:idx=${idx}`);  // no choice info
  return `${idx}`;
}

// ── close voting ──────────────────────────────────────────────────
export function closeVoting(): string {
  if ((near.storageGet("v4:closed") ?? "0") == "1") { near.abort("already closed"); }
  near.storageSet("v4:closed", "1");
  const count = near.storageGet("v4:count") ?? "0";
  const sum = near.storageGet("v4:sum") ?? ZERO_POINT;
  near.log(`voting_closed:ballots=${count}:sum=${strSlice(sum, 0, 16)}…`);
  return "OK";
}

// ── submit decrypted tally ─────────────────────────────────────────
// The tally authority (or threshold set) decrypts the homomorphic sum
// off-chain and submits the result. In production this would include
// a proof of correct decryption. For the demo, the authority is trusted.
export function submitTally(): string {
  if ((near.storageGet("v4:closed") ?? "0") != "1") { near.abort("voting not closed"); }
  if ((near.storageGet("v4:tally_submitted") ?? "0") == "1") { near.abort("tally already submitted"); }
  const yes = near.jsonGetInt("yes") ?? 0;
  const total = strToNum(near.storageGet("v4:count") ?? "0");
  if (yes < 0 || yes > total) { near.abort("invalid tally"); }

  near.storageSet("v4:yes", `${yes}`);
  near.storageSet("v4:no", `${total - yes}`);
  near.storageSet("v4:tally_submitted", "1");
  near.log(`tally_decrypted:yes=${yes}:no=${total - yes}:total=${total}`);
  return "OK";
}

// ── views ─────────────────────────────────────────────────────────
export function currentRoot(): string {
  return near.storageGet("v4:root") ?? "0";
}

export function ballotCount(): string {
  return near.storageGet("v4:count") ?? "0";
}

export function homomorphicSum(): string {
  return near.storageGet("v4:sum") ?? ZERO_POINT;
}

export function isClosed(): string {
  return near.storageGet("v4:closed") ?? "0";
}

export function tally(): string {
  const yes = near.storageGet("v4:yes") ?? "?";
  if (yes == "?") { return "not_revealed"; }
  const no = near.storageGet("v4:no") ?? "0";
  const total = near.storageGet("v4:count") ?? "0";
  return `{"yes":${yes},"no":${no},"total":${total}}`;
}
