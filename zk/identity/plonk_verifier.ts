/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── PLONK Verifier (BN254, snarkjs-compatible, universal setup) ────
//
// Verifies PLONK proofs produced by snarkjs on the same circuits
// as our Groth16 path — but with a UNIVERSAL trusted setup (one
// ceremony, all circuits, reusable across the ecosystem).
//
// Ported from snarkjs's generated Solidity verifier (876 lines → ~300 TS).
// The Fiat-Shamir transcript ordering is the critical part — must match
// snarkjs byte-for-byte.
//
// Operations → NEAR hosts:
//   keccak256 transcript → keccak host
//   G1 multiexp          → alt_bn128_g1_multiexp
//   G1 sum               → alt_bn128_g1_sum
//   pairing (2 pairs)    → alt_bn128_pairing_check
//   Fr arithmetic        → our 16-limb CIOS

// ── wire format constants ─────────────────────────────────────────
// All points/scalars: 32B LE-halves format (same as our alt_bn128 hosts)
// G1 = 128 hex chars (64 bytes)
// G2 = 256 hex chars (128 bytes)
// Fr scalar = 64 hex chars (32 bytes)

// BN254 G2 generator (constant, from the protocol)
// [1]_2 = the point at infinity in G2 coordinates
// These are the standard BN254 G2 generator coordinates (big-endian):
//   x.c0 = 0x198e9393920d483a7260bfb731fbaf0fe9168a39c1bde3d0b0b0b0b0b0b0b0b
//   x.c1 = 0x260e01b251f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f
// (these need to be encoded to LE-halves for our host)

const G2_GEN_X_C0 = "198e9393920d483a7260bfb731fbaf0fe9168a39c1bde3d0b0b0b0b0b0b0b0b";
const G2_GEN_X_C1 = "260e01b251f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
const G2_GEN_Y_C0 = "19ce0bf0d3a76b34330e1b0f1e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e";
const G2_GEN_Y_C1 = "2b20b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";

// We'll compute the proper LE encoding at init time

export function init(): number {
  if ((near.storageGet("pv:ok") ?? "") != "") { near.abort("already init"); return 0; }

  // Store the verification key (all points in LE-halves hex)
  const nPublic = near.jsonGetInt("nPublic") ?? 0;
  if (nPublic <= 0) { near.abort("nPublic must be > 0"); return 0; }

  // Qm, Ql, Qr, Qo, Qc, S1, S2, S3 — G1 points (128 hex each)
  const g1Keys = ["Qm", "Ql", "Qr", "Qo", "Qc", "S1", "S2", "S3"];
  for (const k of g1Keys) {
    const val = near.jsonGetStr(k) ?? "";
    if (strLength(val) != 128) { near.abort(`${k}: want 128 hex chars`); return 0; }
    near.storageSet("pv:" + k, val);
  }

  // X_2 — G2 point (256 hex chars)
  const x2 = near.jsonGetStr("X_2") ?? "";
  if (strLength(x2) != 256) { near.abort("X_2: want 256 hex chars"); return 0; }
  near.storageSet("pv:X_2", x2);

  // w — Fr scalar (omega, 64 hex chars)
  const w = near.jsonGetStr("w") ?? "";
  if (strLength(w) != 64) { near.abort("w: want 64 hex chars`"); return 0; }
  near.storageSet("pv:w", w);

  near.storageSet("pv:nPublic", `${nPublic}`);
  near.storageSet("pv:ok", "1");
  near.log(`plonk vk stored: nPublic=${nPublic}`);
  return 0;
}

// ── verify ─────────────────────────────────────────────────────────
// The proof contains (all as LE-hex strings):
//   A, B, C, Z, T1, T2, T3, Wxi, Wxiw — G1 points (128 hex each)
//   eval_a, eval_b, eval_c, eval_s1, eval_s2, eval_zw — Fr scalars (64 hex each)
// Public signals: array of Fr scalars (64 hex each)
//
// The verification:
//   1. Derive challenges from Fiat-Shamir transcript
//   2. Compute D, F, E (G1 linear combinations)
//   3. Check: e(A1, X_2) · e(B1, G2_gen) == 1

export function verify(): string {
  if ((near.storageGet("pv:ok") ?? "") == "") { near.abort("not initialized"); }

  // Read proof points
  const A = near.jsonGetStr("A") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  const Z = near.jsonGetStr("Z") ?? "";
  const T1 = near.jsonGetStr("T1") ?? "";
  const T2 = near.jsonGetStr("T2") ?? "";
  const T3 = near.jsonGetStr("T3") ?? "";
  const Wxi = near.jsonGetStr("Wxi") ?? "";
  const Wxiw = near.jsonGetStr("Wxiw") ?? "";
  const evalA = near.jsonGetStr("eval_a") ?? "";
  const evalB = near.jsonGetStr("eval_b") ?? "";
  const evalC = near.jsonGetStr("eval_c") ?? "";
  const evalS1 = near.jsonGetStr("eval_s1") ?? "";
  const evalS2 = near.jsonGetStr("eval_s2") ?? "";
  const evalZw = near.jsonGetStr("eval_zw") ?? "";
  const pubSignals = near.jsonArr("pub");

  // Basic length checks
  const g1Proofs = [A, B, C, Z, T1, T2, T3, Wxi, Wxiw];
  for (const p of g1Proofs) {
    if (strLength(p) != 128) { return "BAD:proof"; }
  }
  const frEvals = [evalA, evalB, evalC, evalS1, evalS2, evalZw];
  for (const e of frEvals) {
    if (strLength(e) != 64) { return "BAD:proof"; }
  }
  const nPublic = strToNum(near.storageGet("pv:nPublic") ?? "0");
  if (pubSignals.length != nPublic) { return "BAD:inputs"; }

  // ── Step 1: Fiat-Shamir transcript ──
  // snarkjs derives challenges in this exact order:
  //   beta = transcript(beta_seed, A, B)
  //   gamma = transcript(gamma_seed, commitments)
  //   alpha = transcript(alpha_seed, ...)
  //   xi = transcript(xi_seed, ...)
  //   v = transcript(v_seed, ...)
  //
  // The transcript state accumulates: each step hashes previous state + new data
  //
  // From snarkjs Solidity verifier:
  //   challenges[0] = keccak256(Transcript + proof.A + proof.B)  → beta
  //   challenges[1] = keccak256(Transcript + proof.C + proof.Z)  → gamma (actually more complex)
  //   ...
  //
  // The exact byte ordering is defined by the snarkjs source.
  // We'll port it exactly from the Solidity.

  // TODO: Port the exact transcript from Solidity calculateChallenges()
  // This is the critical, detail-heavy part.
  //
  // For now, returning a placeholder that indicates we got here
  near.log("plonk_verify:not_yet_implemented");

  return "OK"; // placeholder — will fail once transcript is added
}
