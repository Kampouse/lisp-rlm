/// <reference path="../ts/lisp-rlm.d.ts" />

// ── Groth16 on-chain verifier (BN254, snarkjs-compatible) ────────────────
//
// Verifies:  e(-A, B) · e(α, β) · e(pub, γ) · e(C, δ) == 1
//   where pub = IC[0] + Σ(xᵢ · IC[i]) — computed on-chain from the
//   submitted public inputs. The contract NEVER trusts a precomputed
//   combination point; that would break soundness.
//
// Wire format (nearcore alt_bn128 hosts, LE bytes per field element —
// see bn254.rs decode_u256):
//   G1 point = 64B  (x ‖ y)                           → 128 hex chars
//   G2 point = 128B (x.c0 ‖ x.c1 ‖ y.c0 ‖ y.c1)     → 256 hex chars
//   scalar   = 32B LE                                 →  64 hex chars
//   pairing gate = 4 pairs × (G1 64B ‖ G2 128B) = 768B
//   multiexp    = N   pairs × (G1 64B ‖ scalar 32B) = 96B stride
//
// The A point arrives NEGATED (x, p−y) — done by the client bridge
// (5 lines there vs field arithmetic here; soundness is unaffected:
// the equation binds A regardless of who computes its negation).
//
// ── Deployment ────────────────────────────────────────────────────────────
//
//   1. Call init() with the verification key (alpha1, beta2, gamma2,
//      delta2, n, ic[]). One-time; re-init aborts.
//   2. Call verify(negA, B, C, inputs[]) with a proof + public signals.
//      Returns "OK" or "BAD:reason".
//   3. Use checkState() to audit the stored VK.
//
// ── Bug 2 fix ─────────────────────────────────────────────────────────────
//
// Early `return "BAD:…"` now correctly skips subsequent impure const
// initializers (calls to altBn128G1Multiexp, storageGet, etc.). Before
// the fix, `const pub = near.altBn128G1Multiexp(mx)` after a validation
// return would still execute the multiexp host call — wasting gas and
// potentially corrupting state.

const G1_LEN: number = 128;    // hex chars for a G1 point (64 bytes)
const G2_LEN: number = 256;   // hex chars for a G2 point (128 bytes)
const SCALAR_LEN: number = 64; // hex chars for a BN254 scalar (32 bytes LE)
const ONE_HEX: string = "0100000000000000000000000000000000000000000000000000000000000000";

// ── init: store the verification key (one-time) ──────────────────────────
export function init(): string {
  if ((near.storageGet("vk:ok") ?? "") != "") {
    return "ERR:already_initialized";
  }
  const a1 = near.jsonGetStr("alpha1") ?? "";
  const b2 = near.jsonGetStr("beta2") ?? "";
  const g2 = near.jsonGetStr("gamma2") ?? "";
  const d2 = near.jsonGetStr("delta2") ?? "";
  const n = near.jsonGetInt("n") ?? 0;
  if (n <= 0) { return "ERR:n_must_be_positive"; }
  if (strLength(a1) != G1_LEN) { return "ERR:alpha1_len"; }
  if (strLength(b2) != G2_LEN) { return "ERR:beta2_len"; }
  if (strLength(g2) != G2_LEN) { return "ERR:gamma2_len"; }
  if (strLength(d2) != G2_LEN) { return "ERR:delta2_len"; }

  near.storageSet("vk:alpha1", a1);
  near.storageSet("vk:beta2", b2);
  near.storageSet("vk:gamma2", g2);
  near.storageSet("vk:delta2", d2);
  near.storageSet("vk:n", `${n}`);

  const IC = near.jsonArr("ic");
  if (IC.length != n + 1) { return "ERR:ic_count"; }
  let i = 0;
  while (i <= n) {
    if (strLength(IC[i]) != G1_LEN) { return `ERR:ic${i}_len`; }
    near.storageSet("vk:ic:" + `${i}`, IC[i]);
    i = i + 1;
  }
  near.storageSet("vk:ok", "1");
  near.log(`vk stored: n=${n}`);
  return "OK";
}

// ── verify: check a Groth16 proof against the stored VK ──────────────────
export function verify(): string {
  if ((near.storageGet("vk:ok") ?? "") == "") { return "ERR:not_initialized"; }
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const negA = near.jsonGetStr("negA") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  if (strLength(negA) != G1_LEN) { return "BAD:proof:negA"; }
  if (strLength(B) != G2_LEN) { return "BAD:proof:B"; }
  if (strLength(C) != G1_LEN) { return "BAD:proof:C"; }
  const ins = near.jsonArr("inputs");
  if (ins.length != n) { return "BAD:inputs:count"; }

  // pub = IC[0]·1 + Σ IC[i]·xᵢ  (scalar 1 folds the constant term
  // into the same multiexp call — no g1_sum needed)
  let mx = strCat(near.storageGet("vk:ic:0") ?? "", ONE_HEX);
  let i = 1;
  while (i <= n) {
    if (strLength(ins[i - 1]) != SCALAR_LEN) { return "BAD:inputs:len"; }
    mx = strCat(mx, near.storageGet("vk:ic:" + `${i}`) ?? "", ins[i - 1]);
    i = i + 1;
  }
  const pub = near.altBn128G1Multiexp(mx);

  // pairing gate: e(-A,B) · e(α,β) · e(pub,γ) · e(C,δ) == 1
  const alpha = near.storageGet("vk:alpha1") ?? "";
  const beta = near.storageGet("vk:beta2") ?? "";
  const gamma = near.storageGet("vk:gamma2") ?? "";
  const delta = near.storageGet("vk:delta2") ?? "";
  const gate = strCat(negA, B, alpha, beta, pub, gamma, C, delta);
  const ok = near.altBn128PairingCheck(gate);
  if (ok != 1) { return "BAD:pairing"; }
  near.log("verified");
  return "OK";
}

// ── audit helpers ─────────────────────────────────────────────────────────
export function isInitialized(): string {
  return (near.storageGet("vk:ok") ?? "") != "" ? "1" : "0";
}

export function inputCount(): number {
  return strToNum(near.storageGet("vk:n") ?? "0");
}

// Dump multiexp buffer length for debugging (without running multiexp)
export function debugMxLen(): string {
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const ins = near.jsonArr("inputs");
  let mx = strCat(near.storageGet("vk:ic:0") ?? "", ONE_HEX);
  let i = 1;
  while (i <= n) {
    mx = strCat(mx, near.storageGet("vk:ic:" + `${i}`) ?? "", ins[i - 1]);
    i = i + 1;
  }
  return `${strLength(mx)}`;
}