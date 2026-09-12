/// <reference path="../types/lisp-rlm.d.ts" />

// Groth16 verifier (BN254, snarkjs-compatible)
//
// Verifies:  e(-A, B) * e(alpha, beta) * e(pub, gamma) * e(C, delta) == 1
//   where pub = IC[0] + sum(x_i * IC[i])  (computed on-chain from the
//   submitted public inputs — the contract never trusts a precomputed
//   combination point, that would break soundness).
//
// Wire format (nearcore alt_bn128 hosts, LE-halves quirk = plain
// little-endian bytes per field element — see bn254.rs decode_u256):
//   G1 point = 64B (x 32B || y 32B)          -> 128 hex chars
//   G2 point = 128B (x.c0 || x.c1 || y.c0 || y.c1) -> 256 hex chars
//   scalar   = 32B LE                        -> 64 hex chars
// pairing gate = 4 pairs x (G1 64B || G2 128B) = 768B
// multiexp buf = N x (G1 64B || scalar 32B)  = 96B stride
//
// The A point must arrive NEGATED (x, p-y) — done by the client bridge
// (5 lines there vs field arithmetic here; soundness is unaffected: the
// equation binds A regardless of who computes its negation).

const ONE_HEX: string = "0100000000000000000000000000000000000000000000000000000000000000";

export function init(): number {
  if ((near.storageGet("vk:ok") ?? "") != "") { near.abort("already initialized"); return 0; }
  const a1 = near.jsonGetStr("alpha1") ?? "";
  const b2 = near.jsonGetStr("beta2") ?? "";
  const g2 = near.jsonGetStr("gamma2") ?? "";
  const d2 = near.jsonGetStr("delta2") ?? "";
  const n = near.jsonGetInt("n") ?? 0;
  if (n <= 0) { near.abort("n must be > 0"); return 0; }
  if (strLength(a1) != 128) { near.abort("alpha1: want 128 hex chars"); return 0; }
  if (strLength(b2) != 256) { near.abort("beta2: want 256 hex chars"); return 0; }
  if (strLength(g2) != 256) { near.abort("gamma2: want 256 hex chars"); return 0; }
  if (strLength(d2) != 256) { near.abort("delta2: want 256 hex chars"); return 0; }
  near.storageSet("vk:alpha1", a1);
  near.storageSet("vk:beta2", b2);
  near.storageSet("vk:gamma2", g2);
  near.storageSet("vk:delta2", d2);
  near.storageSet("vk:n", `${n}`);
  const IC = near.jsonArr("ic");
  if (IC.length != n + 1) { near.abort("ic: want n+1 points"); return 0; }
  let i = 0;
  while (i <= n) {
    if (strLength(IC[i]) != 128) { near.abort(`ic${i}: want 128 hex chars`); return 0; }
    near.storageSet("vk:ic:" + `${i}`, IC[i]);
    i = i + 1;
  }
  near.storageSet("vk:ok", "1");
  near.log(`vk stored: n=${n}`);
  return 0;
}

export function verify(): string {
  if ((near.storageGet("vk:ok") ?? "") == "") { near.abort("not initialized"); }
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const negA = near.jsonGetStr("negA") ?? "";
  const B = near.jsonGetStr("B") ?? "";
  const C = near.jsonGetStr("C") ?? "";
  if (strLength(negA) != 128) { near.abort("BAD:proof:negA"); }
  if (strLength(B) != 256) { near.abort("BAD:proof:B"); }
  if (strLength(C) != 128) { near.abort("BAD:proof:C"); }
  const ins = near.jsonArr("inputs");
  if (ins.length != n) { near.abort("BAD:inputs:count"); }

  // pub = multiexp(IC[0]|1, IC[1]|x1, ..., IC[n]|xn) — scalar 1 folds
  // the constant term into the same host call (no g1_sum needed).
  let mx = strCat(near.storageGet("vk:ic:0") ?? "", ONE_HEX);
  let i = 1;
  while (i <= n) {
    if (strLength(ins[i - 1]) != 64) { near.abort("BAD:inputs"); }
    mx = strCat(mx, near.storageGet("vk:ic:" + `${i}`) ?? "", ins[i - 1]);
    i = i + 1;
  }
  const pub = near.altBn128G1Multiexp(mx);

  // gate = (negA|B) (alpha|beta) (pub|gamma) (C|delta) — 768B binary
  const alpha = near.storageGet("vk:alpha1") ?? "";
  const beta = near.storageGet("vk:beta2") ?? "";
  const gamma = near.storageGet("vk:gamma2") ?? "";
  const delta = near.storageGet("vk:delta2") ?? "";
  const gate = strCat(negA, B, alpha, beta, pub, gamma, C, delta);
  const ok = near.altBn128PairingCheck(gate);
  if (ok != 1) { return "BAD"; }
  near.log("verified");
  return "OK";
}

// debug: report assembled multiexp buffer length
export function debugMx(): string {
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const ins = near.jsonArr("inputs");
  let mx = (near.storageGet("vk:ic:0") ?? "") + ONE_HEX;
  let i = 1;
  while (i <= n) {
    mx = mx + (near.storageGet("vk:ic:" + `${i}`) ?? "") + ins[i - 1];
    i = i + 1;
  }
  return `n=${n} ins=${ins.length} ic0=${strLength(near.storageGet("vk:ic:0") ?? "")} ic1=${strLength(near.storageGet("vk:ic:1") ?? "")} ic2=${strLength(near.storageGet("vk:ic:2") ?? "")} mx=${strLength(mx)}`;
}

export function debugIns(): string {
  const ins = near.jsonArr("inputs");
  let r = `count=${ins.length}`;
  let i = 0;
  while (i < ins.length) {
    r = r + ` [${i}]=${strLength(ins[i])}`;
    i = i + 1;
  }
  return r;
}

export function debugMx2(): string {
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const ins = near.jsonArr("inputs");
  let mx = (near.storageGet("vk:ic:0") ?? "") + ONE_HEX;
  let i = 1;
  while (i <= n) {
    mx = mx + (near.storageGet("vk:ic:" + `${i}`) ?? "") + ins[i - 1];
    i = i + 1;
  }
  return `${strLength(mx)}|${strSlice(mx, 180, 220)}|end=${strSlice(mx, strLength(mx) - 40, strLength(mx))}`;
}

export function debugMx3(): string {
  const n = strToNum(near.storageGet("vk:n") ?? "0");
  const ins = near.jsonArr("inputs");
  let mx = (near.storageGet("vk:ic:0") ?? "") + ONE_HEX;
  let i = 1;
  while (i <= n) {
    mx = mx + (near.storageGet("vk:ic:" + `${i}`) ?? "") + ins[i - 1];
    i = i + 1;
  }
  return mx;
}

export function debugOne(): string { return ONE_HEX; }
export function debugIc0(): string { return near.storageGet("vk:ic:0") ?? "?"; }
