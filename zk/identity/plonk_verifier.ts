/// <reference path="../../ts/lisp-rlm.d.ts" />

// ── PLONK Verifier (BN254, snarkjs-compatible, universal setup) ────
//
// Simple version: no Lagrange batch inversion optimization.
// Verify cost: ~160 Tgas (under 300 cap). Optimize later.
//
// All operations on existing alt_bn128 hosts.

// ── helpers ─────────────────────────────────────────────────────────

// Fr field ops using 16-limb CIOS (same as our Poseidon contract)




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


function hexCharVal(c: string): number {
  if (c == "0") { return 0; }
  if (c == "1") { return 1; }
  if (c == "2") { return 2; }
  if (c == "3") { return 3; }
  if (c == "4") { return 4; }
  if (c == "5") { return 5; }
  if (c == "6") { return 6; }
  if (c == "7") { return 7; }
  if (c == "8") { return 8; }
  if (c == "9") { return 9; }
  if (c == "a") { return 10; }
  if (c == "b") { return 11; }
  if (c == "c") { return 12; }
  if (c == "d") { return 13; }
  if (c == "e") { return 14; }
  return 15;
}

// BE hex char pair → byte value
function hexPairToNum(s: string): number {
  const hi = hexCharVal(strSlice(s, 0, 1));
  const lo = hexCharVal(strSlice(s, 1, 2));
  return hi * 16 + lo;
}

function parseFr(hexStr: string): number[] {
  // wire format = LE bytes: limb i = chars 4i..4i+4 as TWO byte pairs,
  // LOW byte pair first ("3ad4" = bytes 0x3a,0xd4 → limb 0xd43a).
  // The old hexToNum read the chunk as a big-endian u16 — every limb
  // came out byte-swapped (found via the dbgW oracle: parseFr(w) ≠ w).
  let out = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let i = 0;
  while (i < 16) {
    const start = i * 4;
    const loPair = hexPairToNum(strSlice(hexStr, start, start + 2));
    const hiPair = hexPairToNum(strSlice(hexStr, start + 2, start + 4));
    out[i] = loPair + hiPair * 256;
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
//
// Port of the snarkjs PLONK Solidity verifier (zk/identity/plonk_verifier.sol).
// NO field inversions: the standard verifier batch-inverts Zh and the
// Lagrange pre-numerators (≈380 CIOS muls ≈ 2× the gas cap); instead we
// scale BOTH pairing sides by D0 = Zh·pre1·pre2·pre3 (bilinearity:
// e(D0·A1,X2)·e(D0·B1,1₂) = (e(A1,X2)·e(B1,1₂))^D0 — 1 stays 1), turning
// every 1/x into a polynomial. ~30 extra muls instead of ~380.
//
// G1 work: TWO multiexp calls (18-term B1', 2-term A1') — negations and
// the D0 scaling folded into scalars. G2 points are STATIC (VK X_2 + the
// G2 generator) — the pairing host never needs runtime G2 math.
//
// Wire formats: points/scalars are LE-halves hex (bridge format); the
// transcript hashes BE words (Solidity memory layout) — leWord() converts.
// keccak256 host hashes RAW BYTES, so words are materialized as 1-char
// strings from a 256-byte table literal (\xNN escapes).

// FULL byte table: hexDecode of the 00..ff hex sequence gives a 256-char
// string holding EVERY byte value — literals can't carry bytes ≥ 0xC0
// (\xNN compiles to UTF-8, whose continuation bytes cap at 0xBF — found
// via the TBL-length probe), but hexDecode produces real raw bytes.
// byteChar(b) = strSlice(TBL_ALL, b, b+1); byte value of a char = its
// strIndexOf position in TBL_ALL (all chars distinct).
const TBL_ALL: string = hexDecode("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");

// 1-char string holding raw byte value b (0..255)
function byteChar(b: number): string {
  return strSlice(TBL_ALL, b, b + 1);
}

const G1GEN: string = "01000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000";
const G2GEN: string = "edf692d95cbdde46ddda5ef7d422436779445c5e66006a42761e1f12efde0018c212f3aeb785e49712e7a9353349aaf1255dfb31b7bf60723a480d9293938e19aa7dfa6601cce64c7bd3430c69e7d1e38f40cb8d8071ab4aeb6d8cdba55ec8125b9722d1dcdaac55f38eb37033314bbc95330c69ad999eec75f05f58d0890609";
const ZERO_BE_HEX: string = "0000000000000000000000000000000000000000000000000000000000000000";

// reverse a 32-char hex half byte-wise (2-char groups)
function revHalf(h: string): string {
  let out = "";
  let i = 15;
  while (i >= 0) {
    out = out + strSlice(h, i * 2, i * 2 + 2);
    i = i - 1;
  }
  return out;
}

// LE-halves hex (64 chars: lo-LE 32 + hi-LE 32) → BE hex (64 chars)
function leToBe(h: string): string {
  return strCat(revHalf(strSlice(h, 32, 64)), revHalf(strSlice(h, 0, 32)));
}



// BE hex (64) → 32 raw bytes
function beHexToBytes(h: string): string {
  return hexDecode(h);
}

// LE-halves scalar/coord (64 chars) → 32-byte BE word (raw bytes)
function leWord(h: string): string {
  return beHexToBytes(leToBe(h));
}

// G1 point (128 chars) → its two BE words concatenated (64 raw bytes)
function g1Words(p: string): string {
  return strCat(leWord(strSlice(p, 0, 64)), leWord(strSlice(p, 64, 128)));
}

// keccak digest (32 raw bytes) → Fr limbs, reduced mod q (MSB-first:
// r = r·2^16 + limb over the 16 BE limbs)
const FR_N_4096: number[] = [0,0,0,0,0,0,16,0,0,0,0,0,0,0,0,0]; // n = 2^12
const F_TWO: number[] = [2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
const F_THREE: number[] = [3,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];

// byte value of char position k in a raw string — its position in
// TBL_ALL (all 256 byte-chars are distinct)
function byteAt(s: string, k: number): number {
  return strIndexOf(TBL_ALL, strSlice(s, k, k + 1));
}

// digest mod q, MSB-first over 16-bit limbs. Each step is r = r·2^16 +
// limb (mod q). A limb array can't express 2^16 as a multiplier value
// (limbs are 16-bit — [65536,0,…] silently corrupts CIOS, found via the
// dbgReduce oracle), so the doubling is done as a limb-index SHIFT:
// r·2^16 = [0, r₀, …, r₁₄] + r₁₅·(2^256 mod q) — one ciosMul by the
// precomputed K, then adds. Same mul count as the broken version.
const K_2P256: number[] = [65531,20479,13340,44182,52521,40800,30357,14076,17966,30841,41839,26222,57135,39431,30657,3594];

// ── Montgomery field arithmetic ──
// ciosMul(a, b) = a·b·R⁻¹ mod q with R = 2^256 (CIOS — verified against
// node oracles: ciosMul(K,3) = K·3·R⁻¹ exactly). ALL products run in the
// Montgomery domain: constants are pre-converted (suffix _M), entry
// conversion is toMont(x) = ciosMul(x, R2), exit (multiexp scalars) is
// fromMont(x) = ciosMul(x, ONE_plain). fadd/fsub are domain-agnostic.
const R2_MOD: number[] = [28071,44577,58949,7096,23011,58204,15025,21502,32901,21435,33597,35913,17573,32590,53425,534];
const FR_N_4096_M: number[] = [43868,16383,8826,24234,14506,11332,27041,13959,54049,38948,29846,38323,46283,29607,3203,5632];
const F_TWO_M: number[] = [65526,40959,26680,22828,39507,16065,60715,28152,35932,61682,18142,52445,48734,13327,61315,7188];
const F_THREE_M: number[] = [65521,61439,40020,1474,26493,56886,25536,42229,53898,26987,59982,13131,40334,52759,26436,10783];
const FR_ONE_M: number[] = [65531,20479,13340,44182,52521,40800,30357,14076,17966,30841,41839,26222,57135,39431,30657,3594]; // R mod q
const F_ONE_PLAIN: number[] = [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];

function toMont(x: number[]): number[] {
  return ciosMul(x, R2_MOD);
}

function fromMont(x: number[]): number[] {
  return ciosMul(x, F_ONE_PLAIN);
}

// plain-domain limb compare: a ≥ b (both 16 limbs, values < 2^256)
function geqLimbs(a: number[], b: number[]): number {
  let i = 15;
  while (i >= 0) {
    if (a[i] > b[i]) { return 1; }
    if (a[i] < b[i]) { return 0; }
    i = i - 1;
  }
  return 1; // equal
}

// q limbs for the compare-subtract reduction
const FR_P_LIMBS: number[] = [1,61440,62867,17377,28817,31161,59464,10291,22621,33153,17846,47184,41001,57649,20082,12388];

// 32-byte BE digest → PLAIN Fr limbs, no multiplication: pack bytes, then
// subtract q up to 4 times (digest < 2^256 < 5q; compare-subtract beats a
// Horner loop and keeps the domain story simple — the challenge converts
// to Montgomery once, right here)
function digestToFr(d: string): number[] {
  let r = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let i = 0;
  while (i < 16) {
    const loB = byteAt(d, 31 - 2 * i);
    const hiB = byteAt(d, 30 - 2 * i);
    r[i] = loB + hiB * 256;
    i = i + 1;
  }
  // r < 2^256 < 5q → at most 4 subtractions
  i = 0;
  while (i < 4) {
    if (geqLimbs(r, FR_P_LIMBS) == 1) { r = fsub(r, FR_P_LIMBS); }
    i = i + 1;
  }
  return r;
}

// byte value of char position k in a raw string (TBL scan)

// keccak over concatenated raw words → Fr
function keccakFr(data: string): number[] {
  // hash → plain-domain reduction → Montgomery (all downstream products
  // run in the Mont domain)
  return toMont(digestToFr(near.keccak256(data)));
}

function frNeg(a: number[]): number[] {
  return fsub(frZero(), a);
}

// limbs (Mont) → BE hex of the PLAIN value (for hashing a challenge
// back into the transcript — the transcript carries plain-domain words)
function frToBeHex(a: number[]): string {
  return leToBe(limbsToHex(fromMont(a)));
}

export function verify(): string {
  if ((near.storageGet("pv:ok") ?? "") == "") { near.abort("not initialized"); }

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
  const nPublic = strToNum(near.storageGet("pv:nPublic") ?? "0");
  if (pubSignals.length != nPublic) { return "BAD:inputs"; }

  // VK points from storage
  const Qm = near.storageGet("pv:Qm") ?? "";
  const Ql = near.storageGet("pv:Ql") ?? "";
  const Qr = near.storageGet("pv:Qr") ?? "";
  const Qo = near.storageGet("pv:Qo") ?? "";
  const Qc = near.storageGet("pv:Qc") ?? "";
  const S1 = near.storageGet("pv:S1") ?? "";
  const S2 = near.storageGet("pv:S2") ?? "";
  const S3 = near.storageGet("pv:S3") ?? "";
  const X_2 = near.storageGet("pv:X_2") ?? "";
  const wLimbsPlain = parseFr(near.storageGet("pv:w") ?? "");
  const wLimbs = toMont(wLimbsPlain);

  const evalAL = toMont(parseFr(evalA));
  const evalBL = toMont(parseFr(evalB));
  const evalCL = toMont(parseFr(evalC));
  const evalS1L = toMont(parseFr(evalS1));
  const evalS2L = toMont(parseFr(evalS2));
  const evalZwL = toMont(parseFr(evalZw));

  // ── transcript ──
  let m = "";
  m = strCat(m, g1Words(Qm), g1Words(Ql), g1Words(Qr), g1Words(Qo),
              g1Words(Qc), g1Words(S1), g1Words(S2), g1Words(S3));
  let pi2 = 0;
  while (pi2 < 3) {
    if (pi2 < nPublic) { m = strCat(m, leWord(pubSignals[pi2])); }
    else { m = strCat(m, beHexToBytes(ZERO_BE_HEX)); }
    pi2 = pi2 + 1;
  }
  m = strCat(m, g1Words(A), g1Words(B), g1Words(C));
  const beta = keccakFr(m);

  // gamma = keccak(be(beta))
  const betaBE = frToBeHex(beta);
  const gamma = keccakFr(beHexToBytes(betaBE));

  // alpha = keccak(beta||gamma||Z)
  const alpha = keccakFr(strCat(beHexToBytes(betaBE), beHexToBytes(frToBeHex(gamma)), g1Words(Z)));

  // xi = keccak(alpha||T1||T2||T3)
  const xi = keccakFr(strCat(beHexToBytes(frToBeHex(alpha)), g1Words(T1), g1Words(T2), g1Words(T3)));

  // v1 = keccak(xi||evals)
  const v1 = keccakFr(strCat(beHexToBytes(frToBeHex(xi)),
    leWord(evalA), leWord(evalB), leWord(evalC), leWord(evalS1), leWord(evalS2), leWord(evalZw)));

  // u = keccak(Wxi||Wxiw)
  const u = keccakFr(strCat(g1Words(Wxi), g1Words(Wxiw)));

  // ── derived challenges (Montgomery domain) ──
  const alpha2 = ciosMul(alpha, alpha);
  const betaxi = ciosMul(beta, xi);
  let xin = xi; // xi^1; 12 squarings → xi^4096 = xi^n
  let sq = 0;
  while (sq < 12) { xin = ciosMul(xin, xin); sq = sq + 1; }
  const zh = fsub(xin, FR_ONE_M);
  const v2 = ciosMul(v1, v1);
  const v3 = ciosMul(v2, v1);
  const v4 = ciosMul(v3, v1);
  const v5 = ciosMul(v4, v1);

  // ── D0 machinery (no inversions) ──
  const w2 = ciosMul(wLimbs, wLimbs);
  const pre1 = ciosMul(FR_N_4096_M, fsub(xi, FR_ONE_M));
  const pre2 = ciosMul(FR_N_4096_M, fsub(xi, wLimbs));
  const pre3 = ciosMul(FR_N_4096_M, fsub(xi, w2));
  const D0 = ciosMul(ciosMul(zh, pre1), ciosMul(pre2, pre3));
  const zh2 = ciosMul(zh, zh);
  const L1p = ciosMul(zh2, ciosMul(pre2, pre3));
  // L_i = (ω^i/n)·zh/(ξ−ω^i): the ω and ω² prefactors are REQUIRED (the
  // Solidity multiplies eval_l2/l3 by w/w² after inversion — missed on
  // the first port; corrupted PI → r0 → E while every other scalar matched)
  const L2p = ciosMul(wLimbs, ciosMul(zh2, ciosMul(pre1, pre3)));
  const L3p = ciosMul(w2, ciosMul(zh2, ciosMul(pre1, pre2)));

  const p0 = toMont(parseFr(pubSignals[0]));
  const p1 = toMont(parseFr(pubSignals[1]));
  const p2 = toMont(parseFr(pubSignals[2]));
  const PIp = frNeg(fadd(fadd(ciosMul(L1p, p0), ciosMul(L2p, p1)), ciosMul(L3p, p2)));

  const e3a = fadd(fadd(evalAL, ciosMul(beta, evalS1L)), gamma);
  const e3b = fadd(fadd(evalBL, ciosMul(beta, evalS2L)), gamma);
  const e3c = fadd(evalCL, gamma);
  const e3 = ciosMul(ciosMul(ciosMul(e3a, e3b), e3c), ciosMul(evalZwL, alpha));
  const r0p = fsub(fsub(PIp, ciosMul(L1p, alpha2)), ciosMul(D0, e3));

  // d2 scalar parts
  const val1 = fadd(fadd(evalAL, betaxi), gamma);
  const val2 = fadd(fadd(evalBL, ciosMul(betaxi, F_TWO_M)), gamma);
  // betaxi·3 via (·2 + ·1): ciosMul(betaxi, F_THREE_M) hits a compiler
  // array-literal/slot bug (same code, same inputs — F_TWO_M path verified,
  // F_THREE_M path returns garbage; the TS CIOS itself is exact — proven
  // by a 200-pair BigInt fuzz of the verbatim algorithm). Decomposed
  // instead of multiplied.
  const betaxi3 = fadd(ciosMul(betaxi, F_TWO_M), betaxi);
  const val3 = fadd(fadd(evalCL, betaxi3), gamma);
  const d2a = ciosMul(ciosMul(ciosMul(val1, val2), val3), alpha);
  // d2b = L1p·alpha² is ALREADY D0-scaled (L1p = D0·L1) — adding it INSIDE
  // the D0 multiply double-scales it. sZ = D0·(d2a + u) + d2b.
  const d2b = ciosMul(L1p, alpha2);
  const sZ = fadd(ciosMul(D0, fadd(d2a, u)), d2b);

  // d3 scalar (negated via scalar): S3 · −D0·(v1'·v2'·v3')
  // d3 uses PLAIN beta (not betaxi) — snarkjs: d3a = a + beta·s1 + gamma
  const q1p = fadd(fadd(evalAL, ciosMul(beta, evalS1L)), gamma);
  const q2p = fadd(fadd(evalBL, ciosMul(beta, evalS2L)), gamma);
  const q3p = ciosMul(ciosMul(alpha, beta), evalZwL);
  const sS3 = frNeg(ciosMul(D0, ciosMul(ciosMul(q1p, q2p), q3p)));

  // d4 scalars (negated): T1·−D0·zh, T2·−D0·zh·xin, T3·−D0·zh·xin²
  const sT1 = frNeg(ciosMul(D0, zh));
  const sT2 = frNeg(ciosMul(D0, ciosMul(zh, xin)));
  const sT3 = frNeg(ciosMul(D0, ciosMul(zh, ciosMul(xin, xin))));

  // gate + vanishing scalars
  const ab = ciosMul(evalAL, evalBL);
  const sQc = D0;
  const sQm = ciosMul(D0, ab);
  const sQl = ciosMul(D0, evalAL);
  const sQr = ciosMul(D0, evalBL);
  const sQo = ciosMul(D0, evalCL);

  // linear-combination scalars
  const sA = ciosMul(D0, v1);
  const sB = ciosMul(D0, v2);
  const sC = ciosMul(D0, v3);
  const sS1 = ciosMul(D0, v4);
  const sS2 = ciosMul(D0, v5);
  const sWxi = ciosMul(D0, xi);
  const sWxiw = ciosMul(D0, ciosMul(ciosMul(u, xi), wLimbs));

  // E's G1-generator scalar: r0' − D0·(a·v1 + b·v2 + c·v3 + s1·v4 + s2·v5 + zw·u)
  const inner = fadd(fadd(fadd(ciosMul(evalAL, v1), ciosMul(evalBL, v2)),
                           fadd(ciosMul(evalCL, v3), ciosMul(evalS1L, v4))),
                      fadd(ciosMul(evalS2L, v5), ciosMul(evalZwL, u)));
  const sG1 = fsub(r0p, ciosMul(D0, inner));

  // ── B1' = 18-term multiexp (scalars exit the Montgomery domain) ──
  const mx = strCat(
    Qc, limbsToHex(fromMont(sQc)),
    Qm, limbsToHex(fromMont(sQm)),
    Ql, limbsToHex(fromMont(sQl)),
    Qr, limbsToHex(fromMont(sQr)),
    Qo, limbsToHex(fromMont(sQo)),
    Z, limbsToHex(fromMont(sZ)),
    S3, limbsToHex(fromMont(sS3)),
    T1, limbsToHex(fromMont(sT1)),
    T2, limbsToHex(fromMont(sT2)),
    T3, limbsToHex(fromMont(sT3)),
    A, limbsToHex(fromMont(sA)),
    B, limbsToHex(fromMont(sB)),
    C, limbsToHex(fromMont(sC)),
    S1, limbsToHex(fromMont(sS1)),
    S2, limbsToHex(fromMont(sS2)),
    Wxi, limbsToHex(fromMont(sWxi)),
    Wxiw, limbsToHex(fromMont(sWxiw)),
    G1GEN, limbsToHex(fromMont(sG1)));
  const B1 = near.altBn128G1Multiexp(mx);

  // ── A1' (negated, D0-scaled) = 2-term multiexp ──
  const a1Wxi = frNeg(D0);
  const a1Wxiw = frNeg(ciosMul(D0, u));
  const mxA1 = strCat(Wxi, limbsToHex(fromMont(a1Wxi)), Wxiw, limbsToHex(fromMont(a1Wxiw)));
  const A1 = near.altBn128G1Multiexp(mxA1);

  // ── pairing: e(A1', X_2) · e(B1', G2gen) ──
  const gate = strCat(A1, X_2, B1, G2GEN);
  const ok = near.altBn128PairingCheck(gate);
  if (ok != 1) { return "BAD:pairing"; }
  return "OK";
}

// limbs → BE hex (for hashing a reduced Fr back to a word)




function nPub(): number { return strToNum(near.storageGet("pv:nPublic") ?? "0"); }





export function dbgScalars(): string {
  // duplicate of verify()'s math (kept in sync manually — debug only):
  // transcript → derived → D0 → all 20 multiexp scalars, PLAIN domain
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
  const nPublic = strToNum(near.storageGet("pv:nPublic") ?? "0");
  const Qm = near.storageGet("pv:Qm") ?? "";
  const Ql = near.storageGet("pv:Ql") ?? "";
  const Qr = near.storageGet("pv:Qr") ?? "";
  const Qo = near.storageGet("pv:Qo") ?? "";
  const Qc = near.storageGet("pv:Qc") ?? "";
  const S1 = near.storageGet("pv:S1") ?? "";
  const S2 = near.storageGet("pv:S2") ?? "";
  const S3 = near.storageGet("pv:S3") ?? "";
  const wLimbs = toMont(parseFr(near.storageGet("pv:w") ?? ""));
  const evalAL = toMont(parseFr(evalA));
  const evalBL = toMont(parseFr(evalB));
  const evalCL = toMont(parseFr(evalC));
  const evalS1L = toMont(parseFr(evalS1));
  const evalS2L = toMont(parseFr(evalS2));
  const evalZwL = toMont(parseFr(evalZw));
  let m = "";
  m = strCat(m, g1Words(Qm), g1Words(Ql), g1Words(Qr), g1Words(Qo),
              g1Words(Qc), g1Words(S1), g1Words(S2), g1Words(S3));
  let pi2 = 0;
  while (pi2 < 3) {
    if (pi2 < nPublic) { m = strCat(m, leWord(pubSignals[pi2])); }
    else { m = strCat(m, beHexToBytes(ZERO_BE_HEX)); }
    pi2 = pi2 + 1;
  }
  m = strCat(m, g1Words(A), g1Words(B), g1Words(C));
  const beta = keccakFr(m);
  const betaBE = frToBeHex(beta);
  const gamma = keccakFr(beHexToBytes(betaBE));
  const alpha = keccakFr(strCat(beHexToBytes(betaBE), beHexToBytes(frToBeHex(gamma)), g1Words(Z)));
  const xi = keccakFr(strCat(beHexToBytes(frToBeHex(alpha)), g1Words(T1), g1Words(T2), g1Words(T3)));
  const v1 = keccakFr(strCat(beHexToBytes(frToBeHex(xi)),
    leWord(evalA), leWord(evalB), leWord(evalC), leWord(evalS1), leWord(evalS2), leWord(evalZw)));
  const u = keccakFr(strCat(g1Words(Wxi), g1Words(Wxiw)));
  const alpha2 = ciosMul(alpha, alpha);
  const betaxi = ciosMul(beta, xi);
  let xin = xi;
  let sq = 0;
  while (sq < 12) { xin = ciosMul(xin, xin); sq = sq + 1; }
  const zh = fsub(xin, FR_ONE_M);
  const v2 = ciosMul(v1, v1);
  const v3 = ciosMul(v2, v1);
  const v4 = ciosMul(v3, v1);
  const v5 = ciosMul(v4, v1);
  const w2 = ciosMul(wLimbs, wLimbs);
  const pre1 = ciosMul(FR_N_4096_M, fsub(xi, FR_ONE_M));
  const pre2 = ciosMul(FR_N_4096_M, fsub(xi, wLimbs));
  const pre3 = ciosMul(FR_N_4096_M, fsub(xi, w2));
  const D0 = ciosMul(ciosMul(zh, pre1), ciosMul(pre2, pre3));
  const zh2 = ciosMul(zh, zh);
  const L1p = ciosMul(zh2, ciosMul(pre2, pre3));
  // L_i = (ω^i/n)·zh/(ξ−ω^i): the ω and ω² prefactors are REQUIRED (the
  // Solidity multiplies eval_l2/l3 by w/w² after inversion — missed on
  // the first port; corrupted PI → r0 → E while every other scalar matched)
  const L2p = ciosMul(wLimbs, ciosMul(zh2, ciosMul(pre1, pre3)));
  const L3p = ciosMul(w2, ciosMul(zh2, ciosMul(pre1, pre2)));
  const p0 = toMont(parseFr(pubSignals[0]));
  const p1 = toMont(parseFr(pubSignals[1]));
  const p2 = toMont(parseFr(pubSignals[2]));
  const PIp = frNeg(fadd(fadd(ciosMul(L1p, p0), ciosMul(L2p, p1)), ciosMul(L3p, p2)));
  const e3a = fadd(fadd(evalAL, ciosMul(beta, evalS1L)), gamma);
  const e3b = fadd(fadd(evalBL, ciosMul(beta, evalS2L)), gamma);
  const e3c = fadd(evalCL, gamma);
  const e3 = ciosMul(ciosMul(ciosMul(e3a, e3b), e3c), ciosMul(evalZwL, alpha));
  const r0p = fsub(fsub(PIp, ciosMul(L1p, alpha2)), ciosMul(D0, e3));
  const val1 = fadd(fadd(evalAL, betaxi), gamma);
  const val2 = fadd(fadd(evalBL, ciosMul(betaxi, F_TWO_M)), gamma);
  // betaxi·3 via (·2 + ·1): ciosMul(betaxi, F_THREE_M) hits a compiler
  // array-literal/slot bug (same code, same inputs — F_TWO_M path verified,
  // F_THREE_M path returns garbage; the TS CIOS itself is exact — proven
  // by a 200-pair BigInt fuzz of the verbatim algorithm). Decomposed
  // instead of multiplied.
  const betaxi3 = fadd(ciosMul(betaxi, F_TWO_M), betaxi);
  const val3 = fadd(fadd(evalCL, betaxi3), gamma);
  const d2a = ciosMul(ciosMul(ciosMul(val1, val2), val3), alpha);
  // d2b = L1p·alpha² is ALREADY D0-scaled (L1p = D0·L1) — adding it INSIDE
  // the D0 multiply double-scales it. sZ = D0·(d2a + u) + d2b.
  const d2b = ciosMul(L1p, alpha2);
  const sZ = fadd(ciosMul(D0, fadd(d2a, u)), d2b);
  // d3 uses PLAIN beta (not betaxi) — snarkjs: d3a = a + beta·s1 + gamma
  const q1p = fadd(fadd(evalAL, ciosMul(beta, evalS1L)), gamma);
  const q2p = fadd(fadd(evalBL, ciosMul(beta, evalS2L)), gamma);
  const q3p = ciosMul(ciosMul(alpha, beta), evalZwL);
  const sS3 = frNeg(ciosMul(D0, ciosMul(ciosMul(q1p, q2p), q3p)));
  const sT1 = frNeg(ciosMul(D0, zh));
  const sT2 = frNeg(ciosMul(D0, ciosMul(zh, xin)));
  const sT3 = frNeg(ciosMul(D0, ciosMul(zh, ciosMul(xin, xin))));
  const ab = ciosMul(evalAL, evalBL);
  const sQc = D0;
  const sQm = ciosMul(D0, ab);
  const sQl = ciosMul(D0, evalAL);
  const sQr = ciosMul(D0, evalBL);
  const sQo = ciosMul(D0, evalCL);
  const sA = ciosMul(D0, v1);
  const sB = ciosMul(D0, v2);
  const sC = ciosMul(D0, v3);
  const sS1 = ciosMul(D0, v4);
  const sS2 = ciosMul(D0, v5);
  const sWxi = ciosMul(D0, xi);
  const sWxiw = ciosMul(D0, ciosMul(ciosMul(u, xi), wLimbs));
  const inner = fadd(fadd(fadd(ciosMul(evalAL, v1), ciosMul(evalBL, v2)),
                           fadd(ciosMul(evalCL, v3), ciosMul(evalS1L, v4))),
                      fadd(ciosMul(evalS2L, v5), ciosMul(evalZwL, u)));
  const sG1 = fsub(r0p, ciosMul(D0, inner));
  const a1Wxi = frNeg(D0);
  const a1Wxiw = frNeg(ciosMul(D0, u));
  return strCat(
    limbsToHex(fromMont(sQc)), ",", limbsToHex(fromMont(sQm)), ",", limbsToHex(fromMont(sQl)), ",",
    limbsToHex(fromMont(sQr)), ",", limbsToHex(fromMont(sQo)), ",", limbsToHex(fromMont(sZ)), ",",
    limbsToHex(fromMont(sS3)), ",", limbsToHex(fromMont(sT1)), ",", limbsToHex(fromMont(sT2)), ",",
    limbsToHex(fromMont(sT3)), ",", limbsToHex(fromMont(sA)), ",", limbsToHex(fromMont(sB)), ",",
    limbsToHex(fromMont(sC)), ",", limbsToHex(fromMont(sS1)), ",", limbsToHex(fromMont(sS2)), ",",
    limbsToHex(fromMont(sWxi)), ",", limbsToHex(fromMont(sWxiw)), ",", limbsToHex(fromMont(sG1)), ",",
    limbsToHex(fromMont(a1Wxi)), ",", limbsToHex(fromMont(a1Wxiw)));
}







// ── debug: isolate conversions for oracle testing ──
export function debugChallenges(): string {
  // recomputes beta..u and returns them as LE-halves hex, comma-separated
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
  const nPublic = strToNum(near.storageGet("pv:nPublic") ?? "0");
  const Qm = near.storageGet("pv:Qm") ?? "";
  const Ql = near.storageGet("pv:Ql") ?? "";
  const Qr = near.storageGet("pv:Qr") ?? "";
  const Qo = near.storageGet("pv:Qo") ?? "";
  const Qc = near.storageGet("pv:Qc") ?? "";
  const S1 = near.storageGet("pv:S1") ?? "";
  const S2 = near.storageGet("pv:S2") ?? "";
  const S3 = near.storageGet("pv:S3") ?? "";

  let m = "";
  m = strCat(m, g1Words(Qm), g1Words(Ql), g1Words(Qr), g1Words(Qo),
              g1Words(Qc), g1Words(S1), g1Words(S2), g1Words(S3));
  let pi2 = 0;
  while (pi2 < 3) {
    if (pi2 < nPublic) { m = strCat(m, leWord(pubSignals[pi2])); }
    else { m = strCat(m, beHexToBytes(ZERO_BE_HEX)); }
    pi2 = pi2 + 1;
  }
  m = strCat(m, g1Words(A), g1Words(B), g1Words(C));
  const beta = keccakFr(m);
  const betaBE = frToBeHex(beta);
  const gamma = keccakFr(beHexToBytes(betaBE));
  const alpha = keccakFr(strCat(beHexToBytes(betaBE), beHexToBytes(frToBeHex(gamma)), g1Words(Z)));
  const xi = keccakFr(strCat(beHexToBytes(frToBeHex(alpha)), g1Words(T1), g1Words(T2), g1Words(T3)));
  const v1 = keccakFr(strCat(beHexToBytes(frToBeHex(xi)),
    leWord(evalA), leWord(evalB), leWord(evalC), leWord(evalS1), leWord(evalS2), leWord(evalZw)));
  const u = keccakFr(strCat(g1Words(Wxi), g1Words(Wxiw)));
  return strCat(limbsToHex(beta), ",", limbsToHex(gamma), ",", limbsToHex(alpha), ",",
                limbsToHex(xi), ",", limbsToHex(v1), ",", limbsToHex(u));
}


