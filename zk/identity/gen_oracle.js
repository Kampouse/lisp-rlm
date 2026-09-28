const fs = require('fs');
const { keccak256 } = require('js-sha3');
const vkey = JSON.parse(fs.readFileSync('vkey_plonk.json'));
const proof = JSON.parse(fs.readFileSync('proof_plonk.json'));
const pub = JSON.parse(fs.readFileSync('public_plonk.json'));
const q = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;
const w32 = (v) => BigInt(v).toString(16).padStart(64, '0');
const vk = [vkey.Qm, vkey.Ql, vkey.Qr, vkey.Qo, vkey.Qc, vkey.S1, vkey.S2, vkey.S3];
let m = '';
for (const p of vk) m += w32(p[0]) + w32(p[1]);
for (let i = 0; i < 3; i++) m += w32(pub[i] || 0);
m += w32(proof.A[0]) + w32(proof.A[1]) + w32(proof.B[0]) + w32(proof.B[1]) + w32(proof.C[0]) + w32(proof.C[1]);
const beta = BigInt('0x' + keccak256(Buffer.from(m, 'hex'))) % q;
const gamma = BigInt('0x' + keccak256(Buffer.from(w32(beta), 'hex'))) % q;
const alpha = BigInt('0x' + keccak256(Buffer.from(w32(beta) + w32(gamma) + w32(proof.Z[0]) + w32(proof.Z[1]), 'hex'))) % q;
const xi = BigInt('0x' + keccak256(Buffer.from(w32(alpha) + w32(proof.T1[0]) + w32(proof.T1[1]) + w32(proof.T2[0]) + w32(proof.T2[1]) + w32(proof.T3[0]) + w32(proof.T3[1]), 'hex'))) % q;
const v1 = BigInt('0x' + keccak256(Buffer.from(w32(xi) + w32(proof.eval_a) + w32(proof.eval_b) + w32(proof.eval_c) + w32(proof.eval_s1) + w32(proof.eval_s2) + w32(proof.eval_zw), 'hex'))) % q;
const u = BigInt('0x' + keccak256(Buffer.from(w32(proof.Wxi[0]) + w32(proof.Wxi[1]) + w32(proof.Wxiw[0]) + w32(proof.Wxiw[1]), 'hex'))) % q;
const mul = (a, b) => a * b % q;
const add = (a, b) => (a + b) % q;
const sub = (a, b) => (a - b + q) % q;
const n = 1n << BigInt(vkey.power);
const w = BigInt(vkey.w);
const alpha2 = mul(alpha, alpha);
const betaxi = mul(beta, xi);
let xin = xi;
for (let i = 0; i < vkey.power; i++) xin = mul(xin, xin);
const zh = sub(xin, 1n);
const v2 = mul(v1, v1), v3 = mul(v2, v1), v4 = mul(v3, v1), v5 = mul(v4, v1);
const ea = BigInt(proof.eval_a), eb = BigInt(proof.eval_b), ec = BigInt(proof.eval_c), es1 = BigInt(proof.eval_s1), es2 = BigInt(proof.eval_s2), ezw = BigInt(proof.eval_zw);
const w2 = mul(w, w);
const pre1 = mul(n, sub(xi, 1n));
const pre2 = mul(n, sub(xi, w));
const pre3 = mul(n, sub(xi, w2));
const D0 = mul(mul(zh, pre1), mul(pre2, pre3));
const zh2 = mul(zh, zh);
const L1p = mul(zh2, mul(pre2, pre3));
const L2p = mul(w, mul(zh2, mul(pre1, pre3)));
const L3p = mul(w2, mul(zh2, mul(pre1, pre2)));
const p0 = BigInt(pub[0]), p1 = BigInt(pub[1]), p2 = BigInt(pub[2]);
const PIp = sub(0n, add(add(mul(L1p, p0), mul(L2p, p1)), mul(L3p, p2)));
const e3a = add(add(ea, mul(beta, es1)), gamma);
const e3b = add(add(eb, mul(beta, es2)), gamma);
const e3c = add(ec, gamma);
const e3 = mul(mul(mul(e3a, e3b), e3c), mul(ezw, alpha));
const r0p = sub(sub(PIp, mul(L1p, alpha2)), mul(D0, e3));
const val1 = add(add(ea, betaxi), gamma);
const val2 = add(add(eb, mul(betaxi, 2n)), gamma);
const val3 = add(add(ec, mul(betaxi, 3n)), gamma);
const d2a = mul(mul(mul(val1, val2), val3), alpha);
const d2b = mul(L1p, alpha2);
const sZ = add(mul(D0, add(d2a, u)), d2b); // d2b=L1p*alpha2 is ALREADY D0-scaled — must NOT be wrapped in D0 again
const q1p = add(add(ea, mul(beta, es1)), gamma);
const q2p = add(add(eb, mul(beta, es2)), gamma);
const q3p = mul(mul(alpha, beta), ezw);
const sS3 = sub(q, mul(D0, mul(mul(q1p, q2p), q3p)));
const sT1 = sub(q, mul(D0, zh));
const sT2 = sub(q, mul(D0, mul(zh, xin)));
const sT3 = sub(q, mul(D0, mul(zh, mul(xin, xin))));
const ab = mul(ea, eb);
const inner = add(add(add(mul(ea, v1), mul(eb, v2)), add(mul(ec, v3), mul(es1, v4))), add(mul(es2, v5), mul(ezw, u)));
const sG1 = sub(r0p, mul(D0, inner));
const scal = {
  sQc: D0, sQm: mul(D0, ab), sQl: mul(D0, ea), sQr: mul(D0, eb), sQo: mul(D0, ec),
  sZ, sS3, sT1, sT2, sT3,
  sA: mul(D0, v1), sB: mul(D0, v2), sC: mul(D0, v3), sS1: mul(D0, v4), sS2: mul(D0, v5),
  sWxi: mul(D0, xi), sWxiw: mul(D0, mul(mul(u, xi), w)), sG1,
  a1Wxi: sub(q, D0), a1Wxiw: sub(q, mul(D0, u)),
};
fs.writeFileSync('/tmp/plonk_oracle.json', JSON.stringify({ beta, gamma, alpha, xi, v1, u, alpha2, betaxi, xin, zh, v2, v3, v4, v5, pre1, pre2, pre3, D0, L1p, L2p, L3p, PIp, r0p, ...scal }, (k, v) => typeof v === 'bigint' ? v.toString() : v, 1));
console.log('oracle regenerated; xin =', xin.toString().slice(0, 20) + '...');
