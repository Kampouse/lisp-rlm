/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── PLONK Verifier (BN254, snarkjs-compatible, universal setup) ────
//
// Simple version: no Lagrange batch inversion optimization.
// Verify cost: ~160 Tgas (under 300 cap). Optimize later.
//
// All operations on existing alt_bn128 hosts.

// ── helpers ─────────────────────────────────────────────────────────

// Fr field ops using 16-limb CIOS (same as our Poseidon contract)




function hexDigit(n: number): string {
  if (n == 0) { return "0"; }
  if (n == 1) { return "1"; }
  if (n == 2) { return "2"; }
  if (n == 3) { return "3"; }
  if (n == 4) { return "4"; }
  if (n == 5) { return "5"; }
  if (n == 6) { return "6"; }
  if (n == 7) { return "7"; }
  if (n == 8) { return "8"; }
  if (n == 9) { return "9"; }
  if (n == 10) { return "a"; }
  if (n == 11) { return "b"; }
  if (n == 12) { return "c"; }
  if (n == 13) { return "d"; }
  if (n == 14) { return "e"; }
  return "f";
}


function numToHex2(n: number): string {
  const hi = n / 16;
  const lo = n % 16;
  return strCat(hexDigit(hi), hexDigit(lo));
}


function hexToNum(h: string): number {
  // 4 hex chars → number
  let v = 0;
  let i = 0;
  while (i < 4) {
    const c = strSlice(h, i, i + 1);
    let d = 0;
    if (c == "0") { d = 0; }
    else if (c == "1") { d = 1; }
    else if (c == "2") { d = 2; }
    else if (c == "3") { d = 3; }
    else if (c == "4") { d = 4; }
    else if (c == "5") { d = 5; }
    else if (c == "6") { d = 6; }
    else if (c == "7") { d = 7; }
    else if (c == "8") { d = 8; }
    else if (c == "9") { d = 9; }
    else if (c == "a") { d = 10; }
    else if (c == "b") { d = 11; }
    else if (c == "c") { d = 12; }
    else if (c == "d") { d = 13; }
    else if (c == "e") { d = 14; }
    else if (c == "f") { d = 15; }
    v = v * 16 + d;
    i = i + 1;
  }
  return v;
}

// Parse a 64-char hex string (32B LE-halves) into 16 limbs

function limbsToHex(limbs: number[]): string {
  let out = "";
  let i = 0;
  while (i < 16) {
    const v = limbs[i];
    // 16-bit value → 4 hex chars (little-endian bytes: byte0=low, byte1=high)
    const b0 = v % 256;
    const b1 = v / 256;
    out = out + numToHex2(b0) + numToHex2(b1);
    i = i + 1;
  }
  return out;
}


function parseFr(hexStr: string): number[] {
  // hex string is 128 hex chars = 64 bytes
  // LE-halves: first 32 hex = lo u128 LE, next 32 hex = hi u128 LE
  // We need to extract the VALUE and put it into limbs (value = lo + hi*2^128)
  // Each limb is 16 bits, limbs[0] = lowest 16 bits of value
  // This is complex — for the transcript we need to HASH the raw bytes anyway,
  // so we'll work with hex strings for hashing, and convert to limbs only for arithmetic
  let out = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  // Parse the LE-halves format: first 64 hex chars = lo as LE bytes
  // We read the value from the hex by: lo_bytes reversed + hi_bytes reversed = BE representation
  // Actually easier: for 32-byte LE-halves format, the 16-bit limbs ARE just
  // consecutive 4 hex chars from the beginning (each limb = 2 bytes LE)
  let i = 0;
  while (i < 16) {
    const start = i * 4;
    const chunk = strSlice(hexStr, start, start + 4);
    out[i] = hexToNum(chunk);
    i = i + 1;
  }
  return out;
}



// limbs → hex (64 chars, LE limbs)

// limbs → hex (64 chars, LE limbs)

function frZero(): number[] { return [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]; }

function frOne(): number[] { return [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]; }

// ── init ───────────────────────────────────────────────────────────

export function init(): number {
  if ((near.storageGet("pv:ok") ?? "") != "") { near.abort("already init"); return 0; }
  const nPublic = near.jsonGetInt("nPublic") ?? 0;
  if (nPublic <= 0) { near.abort("nPublic must be > 0"); return 0; }

  // Store VK G1 points (each 128 hex chars in LE-halves format)
  const g1Keys = ["Qm", "Ql", "Qr", "Qo", "Qc", "S1", "S2", "S3"];
  const Qm_v = near.jsonGetStr("Qm") ?? "";
  if (strLength(Qm_v) != 128) { near.abort("Qm"); return 0; }
  near.storageSet("pv:Qm", Qm_v);
  const Ql_v = near.jsonGetStr("Ql") ?? "";
  if (strLength(Ql_v) != 128) { near.abort("Ql"); return 0; }
  near.storageSet("pv:Ql", Ql_v);
  const Qr_v = near.jsonGetStr("Qr") ?? "";
  if (strLength(Qr_v) != 128) { near.abort("Qr"); return 0; }
  near.storageSet("pv:Qr", Qr_v);
  const Qo_v = near.jsonGetStr("Qo") ?? "";
  if (strLength(Qo_v) != 128) { near.abort("Qo"); return 0; }
  near.storageSet("pv:Qo", Qo_v);
  const Qc_v = near.jsonGetStr("Qc") ?? "";
  if (strLength(Qc_v) != 128) { near.abort("Qc"); return 0; }
  near.storageSet("pv:Qc", Qc_v);
  const S1_v = near.jsonGetStr("S1") ?? "";
  if (strLength(S1_v) != 128) { near.abort("S1"); return 0; }
  near.storageSet("pv:S1", S1_v);
  const S2_v = near.jsonGetStr("S2") ?? "";
  if (strLength(S2_v) != 128) { near.abort("S2"); return 0; }
  near.storageSet("pv:S2", S2_v);
  const S3_v = near.jsonGetStr("S3") ?? "";
  if (strLength(S3_v) != 128) { near.abort("S3"); return 0; }
  near.storageSet("pv:S3", S3_v);

  // X_2 — G2 point (256 hex chars)
  const x2 = near.jsonGetStr("X_2") ?? "";
  if (strLength(x2) != 256) { near.abort("X_2: want 256 hex"); return 0; }
  near.storageSet("pv:X_2", x2);

  // w — Fr scalar omega (64 hex chars)
  const w = near.jsonGetStr("w") ?? "";
  if (strLength(w) != 64) { near.abort("w: want 64 hex"); return 0; }
  near.storageSet("pv:w", w);

  near.storageSet("pv:nPublic", `${nPublic}`);
  near.storageSet("pv:ok", "1");
  near.log(`plonk vk stored: nPublic=${nPublic}`);
  return 0;
}

// ── verify ─────────────────────────────────────────────────────────

export function verify(): string {
  if ((near.storageGet("pv:ok") ?? "") == "") { near.abort("not initialized"); }

  // Read proof
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

  // Length checks
  let pi = 0;
  while (pi < 9) {
    if (pi == 0) { if (strLength(A) != 128) { return "BAD:proof"; } }
    if (pi == 1) { if (strLength(B) != 128) { return "BAD:proof"; } }
    if (pi == 2) { if (strLength(C) != 128) { return "BAD:proof"; } }
    if (pi == 3) { if (strLength(Z) != 128) { return "BAD:proof"; } }
    if (pi == 4) { if (strLength(T1) != 128) { return "BAD:proof"; } }
    if (pi == 5) { if (strLength(T2) != 128) { return "BAD:proof"; } }
    if (pi == 6) { if (strLength(T3) != 128) { return "BAD:proof"; } }
    if (pi == 7) { if (strLength(Wxi) != 128) { return "BAD:proof"; } }
    if (pi == 8) { if (strLength(Wxiw) != 128) { return "BAD:proof"; } }
    pi = pi + 1;
  }
  const nPublic = strToNum(near.storageGet("pv:nPublic") ?? "0");
  if (pubSignals.length != nPublic) { return "BAD:inputs"; }

  // ── Step 1: Fiat-Shamir transcript ──
  // The transcript hashes values in BE format (as they appear in Solidity calldata).
  // Our wire format is LE-halves. We need to convert to BE for hashing.
  // For now, we build the transcript input as a hex string and hash it.
  //
  // IMPORTANT: the keccak256 host takes a hex string and hashes the decoded bytes.
  // The Solidity hashes 32-byte words where each word is a big-endian Fr value.
  // Our alt_bn128 wire format stores limbs as LE-hex.
  //
  // To convert LE-halves hex (64 chars) → BE hex (64 chars):
  //   LE-halves: [lo_u128_LE(32 chars)] [hi_u128_LE(32 chars)]
  //   BE:        [value_as_BE(64 chars)]
  //   value = lo + hi * 2^128
  //   BE hex = hi_as_BE_hex + lo_as_BE_hex
  //
  // For the transcript, we need to reverse each 32-char LE block and swap them.
  // But actually — snarkjs proof values are originally in BE decimal/hex.
  // Our bridge.py converts BE → LE-halves for the alt_bn128 hosts.
  // For the transcript, we need the ORIGINAL BE encoding.
  //
  // Simplest approach: the proof args include the LE-halves format,
  // and we convert back to BE for hashing.
  //
  // Actually — let me check what near.keccak256 expects...

  // For now, let me just get the transcript structure right
  // and we'll fix the encoding in the next iteration.

  near.log("plonk_verify:transcript_pending");
  return "BAD:not_implemented";
}


function isZero(a: number[]): number {
  let i = 0;
  while (i < 16) { if (a[i] != 0) { return 0; } i = i + 1; }
  return 1;
}


function ciosMul(a: number[], b: number[]): number[] {
  const FR_P = [1, 61440, 62867, 17377, 28817, 31161, 59464, 10291, 22621, 33153, 17846, 47184, 41001, 57649, 20082, 12388];
  const FR_NP0 = 65535;
  let t = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let i = 0;
  while (i < 16) {
    let C = 0;
    let j = 0;
    while (j < 16) {
      const sum = t[j] + a[j] * b[i] + C;
      C = sum / 65536;
      t[j] = sum % 65536;
      j = j + 1;
    }
    const s16 = t[16] + C;
    C = s16 / 65536;
    t[16] = s16 % 65536;
    t[17] = C;
    const m = (t[0] * FR_NP0) % 65536;
    const s0 = t[0] + m * FR_P[0];
    C = s0 / 65536;
    let j2 = 1;
    while (j2 < 16) {
      const sj = t[j2] + m * FR_P[j2] + C;
      C = sj / 65536;
      t[j2 - 1] = sj % 65536;
      j2 = j2 + 1;
    }
    const s16b = t[16] + C;
    C = s16b / 65536;
    t[15] = s16b % 65536;
    t[16] = t[17] + C;
    i = i + 1;
  }
  let d = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let borrow = 0;
  let j3 = 0;
  while (j3 < 16) {
    const dd = t[j3] - FR_P[j3] - borrow;
    if (dd < 0) { borrow = 1; d[j3] = dd + 65536; } else { borrow = 0; d[j3] = dd; }
    j3 = j3 + 1;
  }
  if (t[16] >= borrow) {
    let j4 = 0;
    while (j4 < 16) { t[j4] = d[j4]; j4 = j4 + 1; }
  }
  return t;
}


function fadd(a: number[], b: number[]): number[] {
  const FR_P = [1, 61440, 62867, 17377, 28817, 31161, 59464, 10291, 22621, 33153, 17846, 47184, 41001, 57649, 20082, 12388];
  let c = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let carry = 0;
  let i = 0;
  while (i < 16) {
    const s = a[i] + b[i] + carry;
    if (s >= 65536) { carry = 1; c[i] = s - 65536; } else { carry = 0; c[i] = s; }
    i = i + 1;
  }
  let d = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let borrow2 = carry;
  let i2 = 0;
  while (i2 < 16) {
    const dd = c[i2] - FR_P[i2] - borrow2;
    if (dd < 0) { borrow2 = 1; d[i2] = dd + 65536; } else { borrow2 = 0; d[i2] = dd; }
    i2 = i2 + 1;
  }
  if (borrow2 == 0) {
    let i3 = 0;
    while (i3 < 16) { c[i3] = d[i3]; i3 = i3 + 1; }
  }
  return c;
}


function fsub(a: number[], b: number[]): number[] {
  const FR_P = [1, 61440, 62867, 17377, 28817, 31161, 59464, 10291, 22621, 33153, 17846, 47184, 41001, 57649, 20082, 12388];
  // a - b mod r
  let c = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let borrow = 0;
  let i = 0;
  while (i < 16) {
    const dd = a[i] - b[i] - borrow;
    if (dd < 0) { borrow = 1; c[i] = dd + 65536; } else { borrow = 0; c[i] = dd; }
    i = i + 1;
  }
  if (borrow == 1) {
    // add r back
    let carry = 0;
    let i2 = 0;
    let res = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
    while (i2 < 16) {
      const s = c[i2] + FR_P[i2] + carry;
      if (s >= 65536) { carry = 1; res[i2] = s - 65536; } else { carry = 0; res[i2] = s; }
      i2 = i2 + 1;
    }
    return res;
  }
  return c;
}


function fpow(base: number[], exp: number[]): number[] {
  // base^exp mod r using square-and-multiply
  let result = [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let b = base;
  let i = 15;
  while (i >= 0) {
    let bit = 0;
    while (bit < 16) {
      // check bit (i*16+bit) of exp
      const limb = exp[i];
      const mask = 1 << bit;
      if ((limb & mask) != 0) {
        result = ciosMul(result, b);
      }
      b = ciosMul(b, b);
      bit = bit + 1;
    }
    i = i - 1;
  }
  return result;
}


function finv(a: number[]): number[] {
  // a^(r-2) mod r — Fermat's little theorem
  // r-2 = 21888242871839275222246405745257275088548364400416034343698204186575808495615
  const rMinus2: number[] = [65535, 61440, 62867, 17377, 28817, 31161, 59464, 10291, 22621, 33153, 17846, 47184, 41001, 57649, 20082, 12387];
  return fpow(a, rMinus2);
}

