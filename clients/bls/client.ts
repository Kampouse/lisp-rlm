/**
 * lisp-rlm #16 — BLS12-381 threshold multisig, TypeScript client.
 *
 * Produces the exact wire values consumed by the NEAR contract
 * (fixtures/bls_msig.ts): sign-free hex points, LE fr scalars,
 * client-side Lagrange-weighted partial signatures.
 *
 * Wire ABI (testnet-verified 2026-09-02):
 *   G1 = X(48B) ‖ Y(48B), sign-free, big-endian field elements
 *   G2 = X_im ‖ X_re ‖ Y_im ‖ Y_re (im FIRST), 192B
 *   scalars = 32B little-endian
 *   submit sig = "00" + ser_g1(partial)   (0x00 = use-Y-as-is flag for p1_sum)
 *   coeffs entry = hex(idx+1, 2) + le_fr(c_i), 66 hex chars each
 *
 * Run: node client.ts            (node ≥23, type stripping)
 * Deps: @noble/curves
 */
import { bls12_381 } from "@noble/curves/bls12-381.js";

const G1 = bls12_381.G1;
const Fp = bls12_381.fields.Fp.ORDER; // field modulus p
const r = bls12_381.fields.Fr.ORDER; // group order r

const DST = "BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

// ---------- BigInt helpers ----------

function mod(a: bigint, m: bigint = r): bigint {
  const x = a % m;
  return x >= 0n ? x : x + m;
}

/** modular inverse via extended Euclid */
function inv(a: bigint, m: bigint = r): bigint {
  let [old_r, rr] = [mod(a, m), m];
  let [old_s, s] = [1n, 0n];
  while (rr !== 0n) {
    const q = old_r / rr;
    [old_r, rr] = [rr, old_r - q * rr];
    [old_s, s] = [s, old_s - q * s];
  }
  if (old_r !== 1n) throw new Error("not invertible");
  return mod(old_s, m);
}

/** big-endian 48-byte hex */
function be48(n: bigint): string {
  let h = n.toString(16);
  if (h.length > 96) throw new Error("field element overflow");
  return h.padStart(96, "0");
}

/** little-endian 32-byte hex (fr scalar) */
function le32(n: bigint): string {
  const v = mod(n);
  let h = v.toString(16).padStart(64, "0");
  const bytes: string[] = [];
  for (let i = 62; i >= 0; i -= 2) bytes.push(h.slice(i, i + 2));
  return bytes.join("");
}

// ---------- G1 wire ----------

type P = ReturnType<typeof G1.hashToCurve>;

/** affine (x, y) of a projective point */
function affine(Pt: P): [bigint, bigint] {
  const zinv = inv(Pt.Z, Fp);
  return [(Pt.X * zinv) % Fp, (Pt.Y * zinv) % Fp];
}

/** sign-free 96-byte serialization */
function serG1(Pt: P): string {
  const [x, y] = affine(Pt);
  return be48(x) + be48(y);
}

/** negation in affine coords (for msgPoint = −H(m)) */
function negG1Hex(Pt: P): string {
  const [x, y] = affine(Pt);
  return be48(x) + be48(Fp - y);
}

// ---------- threshold scheme ----------

/** Lagrange coefficients at x=0 for signer set S (1-based ids) */
function lagrange(S: number[]): Map<number, bigint> {
  const out = new Map<number, bigint>();
  for (const i of S) {
    let c = 1n;
    for (const j of S) {
      if (j === i) continue;
      c = (c * BigInt(j) * inv(BigInt(j - i))) % r;
    }
    out.set(i, c);
  }
  return out;
}

// ---------- the #16 scenario (deterministic, matches the py_ecc oracle) ----------

const MSG = "lisp-rlm #16 crypto-true gate, 2026-09-02";
const SKS = [1, 2, 3, 4].map((i) => BigInt(1000 + i * 777)); // demo keys
const S = [1, 2, 3]; // signer subset, t=3

function run(): void {
  const enc = new TextEncoder();
  const H = G1.hashToCurve(enc.encode(MSG), { DST: enc.encode(DST) });

  const cs = lagrange(S);

  const partials = S.map((i) => ({
    i,
    hex: "00" + serG1(H.multiply(mod(BigInt(SKS[i - 1]) * cs.get(i)!))),
  }));

  let sigma = G1.Point.ZERO;
  for (const p of partials) sigma = sigma.add(H.multiply(mod(BigInt(SKS[p.i - 1]) * cs.get(p.i)!)));

  const coeffs = S.map((i) => i.toString(16).padStart(2, "0") + le32(cs.get(i)!)).join("");

  console.log(JSON.stringify({
    msg: MSG,
    msgPoint: negG1Hex(H),
    partials,
    sigma: serG1(sigma),
    coeffs,
  }, null, 2));
}

run();
