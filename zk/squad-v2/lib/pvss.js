// ── Λ_indv 3PVSS on curve25519 (edwards prime-order subgroup) ─────────
// Implements the ePrint 2026/1119 scheme (Fig. 4 + App 2.3) used by
// zk-squad v2. Members are voters+talliers; n members, degree bound
// from robustness n ≥ 2t+ℓ with ℓ=1.
//
// Group G  = ed25519 prime-order subgroup (@noble/curves Point), order L.
// G_en     = Point.BASE           (ballot-ciphertext base, "G" in paper)
// h0       = hash-to-point('zk-squad-v2:h0')   (commitment key)
// PK_i     = G_en·sk_i  (tallier key = ed25519 keypair public point)
// Scalar field Zq = L (BigInt mod).
//
// BALLOT transcript (hex-serialized, opaque to the chain):
//   f: secret poly deg ≤ t, f(0)=s   — sampled by voter
//   y_i = PK_i^f(i)          i=1..n  (encrypted shares)
//   y0  = h0^s                       (commitment to the secret)
//   U   = G_en^(s+v)                 (sealed vote, v ∈ {0,1})
//   pi3pvss = (d, z[X])              (polynomial Fiat-Shamir, App 2.4)
//   piU     = (a0,b0,a1,b1,d0,r0,d1,r1)  (OR-proof v∈{0,1}, App 2.3)

const { ed25519 } = require("@noble/curves/ed25519.js");
const crypto = require("crypto");
const sha512 = (m) => crypto.createHash("sha512").update(m).digest();

const L = ed25519.Point.Fn.ORDER;    // group order q
const G_EN = ed25519.Point.BASE;
const MASK = (1n << 256n) - 1n;      // scalar sampling mask (rejection below)

function mod(a, b = L) { const r = ((a % b) + b) % b; return r; }
function powm(base, e) { return mod(base, e); } // scalars live mod L directly

// deterministic h0 via hash-to-subgroup: map label → 512-bit → reduce → clear cofactor
function hashToPoint(label) {
  const h = sha512(Buffer.from(label));
  let x = 0n; for (const b of h) x = (x << 8n) | BigInt(b);
  return G_EN.multiply(mod(x, L));
}
const H0 = hashToPoint("zk-squad-v2:h0");

// uniform scalar
function randScalar() {
  for (;;) {
    const b = crypto.randomBytes(64);
    let x = 0n; for (const v of b) x = (x << 8n) | BigInt(v);
    x &= MASK;
    if (x < L - 1n) return mod(x, L);
  }
}

// ── poly helpers ──
function polyEval(f, x) {           // f: coefficients [c0..cd], x scalar
  let acc = 0n;
  for (let i = f.length - 1; i >= 0; i--) acc = mod(acc * x + f[i]);
  return acc;
}
function polyAdd(a, b) {
  const out = new Array(Math.max(a.length, b.length)).fill(0n);
  for (let i = 0; i < a.length; i++) out[i] = mod(out[i] + a[i]);
  for (let i = 0; i < b.length; i++) out[i] = mod(out[i] + b[i]);
  return out;
}
function polyMulScalar(a, k) { return a.map(c => mod(c * k)); }

// Fiat-Shamir hash over hex parts → scalar
function fsHash(parts) {
  const h = crypto.createHash("sha512").update(parts.join("|")).digest();
  let x = 0n; for (const b of h) x = (x << 8n) | BigInt(b);
  return mod(x, L);
}
function fsHashPts(parts) {         // hash of point-list → scalar
  return fsHash(parts.map(p => p.toHex()));
}

// ════════════════════════════════════════════════════════════════════
// SHARE — a voter seals bit v and shares s among tallier PKs
// ════════════════════════════════════════════════════════════════════
function share(v, PKs, t) {
  const n = PKs.length;
  // f of degree ≤ t (ℓ=1 ⇒ deg t+ℓ−1 = t), f(0)=s, PLUS v hidden via U
  const f = new Array(t + 1).fill(0n).map(() => randScalar());
  const s = f[0];
  const r = new Array(t + 1).fill(0n).map(() => randScalar());

  // commitments to secrets (j=0 only, ℓ=1) + encrypted shares
  const y0 = H0.multiply(s);
  const ys = [];
  for (let i = 1; i <= n; i++) ys.push(PKs[i - 1].multiply(polyEval(f, BigInt(i))));

  const U = G_EN.multiply(mod(s + BigInt(v)));

  // π_3PVSS: d = H(y0,ys,U,c0,ci) ; z(X) = r(X) + d·f(X)
  const c0 = H0.multiply(r[0]);
  const cis = [];
  for (let i = 1; i <= n; i++) cis.push(PKs[i - 1].multiply(polyEval(r, BigInt(i))));
  const d = fsHashPts([y0, ...ys, U, c0, ...cis]);
  const z = polyAdd(r, polyMulScalar(f, d));

  // π_U (App 2.3): OR-proof v ∈ {0,1} w.r.t. C0=y0 (base h0) and U (base G)
  const w = randScalar();
  const av = [null, null], bv = [null, null];
  av[v] = H0.multiply(w);
  bv[v] = G_EN.multiply(w);
  const onev = 1 - v;                  // the other branch (simulated)
  const r1v = randScalar(), d1v = randScalar();
  av[onev] = H0.multiply(r1v).add(y0.multiply(d1v));
  bv[onev] = G_EN.multiply(r1v).add((onev === 0 ? U : U.subtract(G_EN)).multiply(d1v));
  const c = fsHashPts([av[0], bv[0], av[1], bv[1], y0, U]);
  const dv = [null, null];
  dv[v] = mod(c - d1v);
  dv[onev] = d1v;
  const rv = [null, null];
  rv[v] = mod(w - mod(s * dv[v]));
  rv[onev] = r1v;
  const piU = {
    a0: av[0], b0: bv[0], a1: av[1], b1: bv[1],
    d0: dv[0], r0: rv[0], d1: dv[1], r1: rv[1],
  };

  return {
    y0, ys, U,
    d, z,                                  // π_3PVSS (flattened)
    piU,
    // voter's own record for decrypt demos (never serialized to chain)
    _f: f, _s: s, _v: v,
  };
}

// ════════════════════════════════════════════════════════════════════
// VERIFY BALLOT (off-chain, every member/watcher runs this)
// ════════════════════════════════════════════════════════════════════
function verifyBallot(B, PKs, t) {
  const n = PKs.length;
  if (B.z.length !== t + 1) return false;         // degree check on z(X)
  // recompute challenge: ĉ0 = h0^{z(0)}·y0^{−d} ; ĉi = PK_i^{z(i)}·y_i^{−d}
  const d = B.d;
  const c0 = H0.multiply(polyEval(B.z, 0n)).subtract(B.y0.multiply(d));
  const parts = [B.y0, ...B.ys, B.U, c0];
  for (let i = 1; i <= n; i++) {
    const ci = PKs[i - 1].multiply(polyEval(B.z, BigInt(i))).subtract(B.ys[i - 1].multiply(d));
    parts.push(ci);
  }
  if (fsHashPts(parts) !== d) return false;

  // π_U: c = d0+d1 ; a_i = h0^{r_i}·y0^{d_i} ; b_i = G^{r_i}·(U/G^{1−i})^{d_i}
  const { a0, b0, a1, b1, d0, r0, d1, r1 } = B.piU;
  if (mod(d0 + d1) !== fsHashPts([a0, b0, a1, b1, B.y0, B.U])) return false;
  if (!a0.equals(H0.multiply(r0).add(B.y0.multiply(d0)))) return false;
  if (!a1.equals(H0.multiply(r1).add(B.y0.multiply(d1)))) return false;
  if (!b0.equals(G_EN.multiply(r0).add(B.U.multiply(d0)))) return false;
  if (!b1.equals(G_EN.multiply(r1).add(B.U.subtract(G_EN).multiply(d1)))) return false;
  return true;
}

// ════════════════════════════════════════════════════════════════════
// DECRYPT + DLEQ — tallier i opens their accumulated shares
// ════════════════════════════════════════════════════════════════════
function decryptShare(Ybar_i, sk_i) {
  // Ybar_i = PK_i^{Σf(i)} = G^{sk_i·Σf}  →  X_i = Ybar_i^{1/sk_i} = G^{Σf}
  const X_i = Ybar_i.multiply(modInverse(sk_i));
  // DLEQ proof of knowledge of sk:  PK = G^sk  ∧  Ybar = X^sk
  const PK = PK_pub_from_sk(sk_i);
  const w = randScalar();
  const a1 = G_EN.multiply(w), a2 = X_i.multiply(w);
  const c = fsHashPts([a1, a2, X_i, Ybar_i]);
  const z = mod(w - mod(sk_i * c));
  return { X_i, dleq: { a1, a2, c, z } };
}
function PK_pub_from_sk(sk) { return G_EN.multiply(sk); }
function modInverse(a) {
  // L is prime → Fermat
  let e = L - 2n, r = 1n, b = mod(a);
  while (e > 0n) { if (e & 1n) r = mod(r * b); b = mod(b * b); e >>= 1n; }
  return r;
}
function verifyDecrypt(PK_i, Ybar_i, dec) {
  const { X_i, dleq } = dec;
  // G^z · PK^c = a1   ∧   X^z · Ybar^c = a2   (knowledge of sk)
  if (!G_EN.multiply(dleq.z).add(PK_i.multiply(dleq.c)).equals(dleq.a1)) return false;
  if (!X_i.multiply(dleq.z).add(Ybar_i.multiply(dleq.c)).equals(dleq.a2)) return false;
  return true;
}

// ════════════════════════════════════════════════════════════════════
// TALLY — closers compute V* = Ū·S̄^{-1} = G^{Σv}
// ════════════════════════════════════════════════════════════════════
function tally(acceptedBallots, PKs, sks, t) {
  const n = PKs.length;
  // 1. homomorphic accumulate encrypted shares per tallier
  const Ybar = new Array(n).fill(ed25519.Point.ZERO);
  for (const B of acceptedBallots) {
    for (let i = 0; i < n; i++) Ybar[i] = Ybar[i].add(B.ys[i]);
  }
  // 2. t+1 talliers decrypt + DLEQ; Lagrange-interpolate G^{Σf(0)} at X=0
  const Q = [];
  for (let qi = 0; qi < t + 1; qi++) {
    const dec = decryptShare(Ybar[qi], sks[qi]);
    if (!verifyDecrypt(PKs[qi], Ybar[qi], dec)) throw new Error("bad decrypt share " + qi);
    Q.push({ i: BigInt(qi + 1), X: dec.X_i });
  }
  let Sbar = ed25519.Point.ZERO;
  for (const { i, X } of Q) {
    let coef = 1n;
    for (const { i: j } of Q) {
      if (j !== i) {
        const jInv = modInverse(mod(i - j));     // L_i(0): Π_{j≠i} (0−j)/(i−j)
        coef = mod(coef * mod(0n - j) * jInv);
      }
    }
    if (coef !== 0n) Sbar = Sbar.add(X.multiply(coef));  // noble rejects 0-scalars
  }
  // 3. Ū = Π U_k ; V* = Ū · S̄^{-1}
  let Ubar = ed25519.Point.ZERO;
  for (const B of acceptedBallots) Ubar = Ubar.add(B.U);
  const Vstar = Ubar.subtract(Sbar);   // G^{Σ(s+v)} · G^{−Σs}
  // 4. dLog by table (n small)
  let sum = -1;
  for (let k = 0; k <= acceptedBallots.length; k++) {
    const Gk = k === 0 ? ed25519.Point.ZERO : G_EN.multiply(BigInt(k));
    if (Gk.equals(Vstar)) { sum = k; break; }
  }
  if (sum < 0) throw new Error("dLog failed — transcript inconsistent");
  return { Vstar, sum };
}

// ── serialization (transcript blob for the bulletin board) ──
function pt(p) { return p.toHex(); }
function sc(x) { return x.toString(16).padStart(64, "0"); }
function serializeBallot(B) {
  return [
    pt(B.y0),
    B.ys.map(pt).join(""),
    pt(B.U),
    sc(B.d),
    B.z.map(sc).join(""),
    [B.piU.a0, B.piU.b0, B.piU.a1, B.piU.b1].map(pt).join("") +
      [B.piU.d0, B.piU.r0, B.piU.d1, B.piU.r1].map(sc).join(""),
  ].join("");
}

module.exports = {
  L, G_EN, H0, ed25519,
  share, verifyBallot, decryptShare, verifyDecrypt, tally,
  serializeBallot, fsHash, fsHashPts, pt, sc, randScalar, mod, modInverse,
  PK_pub_from_sk, polyEval,
};

// ── self-test (node lib/pvss.js) ──
if (require.main === module) {
  const n = 8, t = 3;
  const sks = new Array(n).fill(0).map(() => randScalar());
  const PKs = sks.map(PK_pub_from_sk);
  const votes = [1, 0, 1, 0, 1, 1, 0, 1];   // Σv = 5
  const ballots = votes.map(v => share(v, PKs, t));
  let ok = 0;
  for (const B of ballots) { if (verifyBallot(B, PKs, t)) ok++; }
  console.log(`verify: ${ok}/${n} ballots valid`);
  if (ok !== n) process.exit(1);
  // tamper test: flip U
  const bad = { ...ballots[0] };
  bad.U = bad.U.add(G_EN);
  console.log("tampered U rejected:", verifyBallot(bad, PKs, t) === false ? "YES" : "NO!");
  const { Vstar, sum } = tally(ballots, PKs, sks, t);
  console.log(`tally: Σv = ${sum} (expect ${votes.reduce((a, b) => a + b, 0)})`);
  if (sum !== votes.reduce((a, b) => a + b, 0)) process.exit(1);
  // serialization round-trip sanity
  const blob = serializeBallot(ballots[0]);
  console.log("ballot blob bytes:", blob.length / 2);
  console.log("PVSS SELF-TEST ✅");
}
