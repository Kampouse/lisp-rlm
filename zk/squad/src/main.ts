/// <reference path="../types/lisp-rlm.d.ts" />

// ── zk-squad: threshold anonymous authorization (Construction A) ────────
//
// n members enroll (Merkle root of Poseidon(secret) commitments).
// A member approves by submitting the deployed zk-identity proof:
//   public signals = [merkle_root, nullifier, claim=1]
// The chain verifies the Groth16 proof ON-CHAIN, pins the nullifier
// (dedup — one approval per member, unlinkable), counts, and FIRES
// when count ≥ threshold.
//
// Public learns: "T distinct squad members approved" — never which.
// Uses the same verifier math as zk/groth16_verify.ts (proven on
// testnet at 33.6 Tgas): e(-A,B) · e(α,β) · e(pub,γ) · e(C,δ) == 1.

const G1_LEN: number = 128;    // hex chars (64 bytes)
const G2_LEN: number = 256;    // hex chars (128 bytes)
const SCALAR_LEN: number = 64; // hex chars (32 bytes LE)
const ONE_HEX: string = "0100000000000000000000000000000000000000000000000000000000000000";

// ── init: pin the squad + verification key (one-time) ───────────────────
//   root_hex   : LE-hex of the merkle root (compares 1:1 with inputs[0])
//   threshold  : t — fire when t distinct members approved
//   members    : n (informational; squad size)
//   alpha1..ic : verification key (same shape as zk/identity init_args)
export function init(): number {
  if ((near.storageGet("sq:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const rootHex = near.jsonGetStr("root_hex") ?? "";
  const threshold = near.jsonGetInt("threshold") ?? 0;
  const members = near.jsonGetInt("members") ?? 0;
  if (strLength(rootHex) != SCALAR_LEN) { near.abort("root_hex: want 64 hex chars"); return 0; }
  if (threshold <= 0) { near.abort("threshold must be positive"); return 0; }
  if (members < threshold) { near.abort("members < threshold — can never fire... or always"); return 0; }

  const a1 = near.jsonGetStr("alpha1") ?? "";
  const b2 = near.jsonGetStr("beta2") ?? "";
  const g2 = near.jsonGetStr("gamma2") ?? "";
  const d2 = near.jsonGetStr("delta2") ?? "";
  const n = near.jsonGetInt("n") ?? 0;
  if (n != 3) { near.abort("vk: expect 3 public signals [root, nullifier, claim]"); return 0; }
  if (strLength(a1) != G1_LEN) { near.abort("alpha1_len"); return 0; }
  if (strLength(b2) != G2_LEN) { near.abort("beta2_len"); return 0; }
  if (strLength(g2) != G2_LEN) { near.abort("gamma2_len"); return 0; }
  if (strLength(d2) != G2_LEN) { near.abort("delta2_len"); return 0; }
  const IC = near.jsonArr("ic");
  if (IC.length != n + 1) { near.abort("ic_count"); return 0; }

  near.storageSet("sq:ok", "1");
  near.storageSet("sq:root", rootHex);
  near.storageSet("sq:threshold", `${threshold}`);
  near.storageSet("sq:members", `${members}`);
  near.storageSet("sq:count", "0");
  near.storageSet("sq:fired", "0");
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
  near.log(`zk-squad armed: members=${members} threshold=${threshold}`);
  return 0;
}

// ── approve: anonymous approval + on-chain proof verify + tally ─────────
//   {negA, B, C, inputs[]}: inputs = [merkle_root, nullifier, claim]
export function approve(): string {
  if ((near.storageGet("sq:ok") ?? "") == "") { near.abort("not initialized"); }
  if ((near.storageGet("sq:fired") ?? "0") == "1") { near.abort("already fired"); }

  const negA = near.jsonGetStr("negA") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  if (strLength(negA) != G1_LEN) { near.abort("proof:negA"); }
  if (strLength(B) != G2_LEN) { near.abort("proof:B"); }
  if (strLength(C) != G1_LEN) { near.abort("proof:C"); }
  const ins = near.jsonArr("inputs");
  if (ins.length != 3) { near.abort("inputs: want [root, nullifier, claim]"); }

  // bind public signals to THIS squad
  if (ins[0] != (near.storageGet("sq:root") ?? "")) { near.abort("wrong root"); }
  if (ins[2] != ONE_HEX) { near.abort("claim must be 1"); }
  const nullifier = ins[1];

  // nullifier must be fresh (dedup: same member ⇒ same nullifier)
  const key = "sq:null:" + nullifier;
  if ((near.storageGet(key) ?? "") != "") { near.abort("already approved"); }
  near.storageSet(key, "1");

  // ── Groth16 gate (same math as zk/groth16_verify.ts) ──
  // pub = IC[0]·1 + Σ IC[i]·xᵢ
  let mx = strCat(near.storageGet("vk:ic:0") ?? "", ONE_HEX);
  let i = 1;
  while (i <= 3) {
    if (strLength(ins[i - 1]) != SCALAR_LEN) { near.abort("inputs:len"); }
    mx = strCat(mx, near.storageGet("vk:ic:" + `${i}`) ?? "", ins[i - 1]);
    i = i + 1;
  }
  const pub = near.altBn128G1Multiexp(mx);
  const alpha = near.storageGet("vk:alpha1") ?? "";
  const beta = near.storageGet("vk:beta2") ?? "";
  const gamma = near.storageGet("vk:gamma2") ?? "";
  const delta = near.storageGet("vk:delta2") ?? "";
  const gate = strCat(negA, B, alpha, beta, pub, gamma, C, delta);
  const ok = near.altBn128PairingCheck(gate);
  if (ok != 1) {
    // roll back the nullifier burn so a bad proof doesn't grief the member
    near.storageSet(key, "");
    near.abort("BAD:pairing");
  }

  // tally
  const count = strToNum(near.storageGet("sq:count") ?? "0") + 1;
  near.storageSet("sq:count", `${count}`);
  const threshold = strToNum(near.storageGet("sq:threshold") ?? "0");
  near.log(`approval:${count}/${threshold}`);  // no identity info

  if (count >= threshold) {
    near.storageSet("sq:fired", "1");
    near.storageSet("sq:fired_count", `${count}`);
    near.log(`FIRED: ${count} distinct members approved`);
    return "FIRED";
  }
  return `${count}`;
}

// ── views ───────────────────────────────────────────────────────────────
export function status(): string {
  const ok = (near.storageGet("sq:ok") ?? "") != "" ? "1" : "0";
  const count = near.storageGet("sq:count") ?? "0";
  const threshold = near.storageGet("sq:threshold") ?? "0";
  const members = near.storageGet("sq:members") ?? "0";
  const fired = near.storageGet("sq:fired") ?? "0";
  return `{"armed":${ok},"members":${members},"threshold":${threshold},"approved":${count},"fired":${fired}}`;
}

export function fired(): string {
  return near.storageGet("sq:fired") ?? "0";
}
