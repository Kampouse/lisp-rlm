// Helper: generate the PLONK bridge (snarkjs PLONK → NEAR wire format)
// Outputs: init_args.json (VK) and verify_args.json (proof, both as LE-hex)

const fs = require("fs");

const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;

function toLEHex(v) {
  const val = BigInt(v.toString());
  const lo = val & ((1n << 128n) - 1n);
  const hi = val >> 128n;
  const loBytes = [];
  let l = lo;
  for (let i = 0; i < 16; i++) { loBytes.push(Number(l & 0xffn)); l >>= 8n; }
  const hiBytes = [];
  let h = hi;
  for (let i = 0; i < 16; i++) { hiBytes.push(Number(h & 0xffn)); h >>= 8n; }
  return [...loBytes, ...hiBytes].map(b => b.toString(16).padStart(2, "0")).join("");
}

function g1(p) {
  return toLEHex(p[0]) + toLEHex(p[1]);
}

function g2(p) {
  return toLEHex(p[0][0]) + toLEHex(p[0][1]) + toLEHex(p[1][0]) + toLEHex(p[1][1]);
}

function fr(v) {
  return toLEHex(v);
}

const proof = JSON.parse(fs.readFileSync("proof_plonk.json"));
const vkey = JSON.parse(fs.readFileSync("vkey_plonk.json"));
const pub = JSON.parse(fs.readFileSync("public_plonk.json"));

const init = {
  nPublic: vkey.nPublic,
  Qm: g1(vkey.Qm),
  Ql: g1(vkey.Ql),
  Qr: g1(vkey.Qr),
  Qo: g1(vkey.Qo),
  Qc: g1(vkey.Qc),
  S1: g1(vkey.S1),
  S2: g1(vkey.S2),
  S3: g1(vkey.S3),
  X_2: g2(vkey.X_2),
  w: fr(vkey.w),
};

const verify = {
  A: g1(proof.A),
  B: g1(proof.B),
  C: g1(proof.C),
  Z: g1(proof.Z),
  T1: g1(proof.T1),
  T2: g1(proof.T2),
  T3: g1(proof.T3),
  Wxi: g1(proof.Wxi),
  Wxiw: g1(proof.Wxiw),
  eval_a: fr(proof.eval_a),
  eval_b: fr(proof.eval_b),
  eval_c: fr(proof.eval_c),
  eval_s1: fr(proof.eval_s1),
  eval_s2: fr(proof.eval_s2),
  eval_zw: fr(proof.eval_zw),
  pub: pub.map(s => fr(s)),
};

fs.writeFileSync("plonk_init_args.json", JSON.stringify(init));
fs.writeFileSync("plonk_verify_args.json", JSON.stringify(verify));

console.log("PLONK bridge ok");
console.log("init:", JSON.stringify(init).length, "bytes");
console.log("verify:", JSON.stringify(verify).length, "bytes");
console.log("nPublic:", vkey.nPublic);
