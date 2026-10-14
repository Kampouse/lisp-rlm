#!/usr/bin/env node
// prove one more squad member (index from argv) for the live probe
const { buildPoseidonOpt } = require("circomlibjs");
const snarkjs = require("snarkjs");
const fs = require("fs");
const path = require("path");
const IDENTITY = path.join(__dirname, "..", "identity");
const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
const DEPTH = 4;
function intToLEHex(big) {
  let hex = big.toString(16); if (hex.length % 2) hex = "0" + hex;
  let le = ""; for (let i = hex.length - 2; i >= 0; i -= 2) le += hex.slice(i, i + 2);
  while (le.length < 64) le += "0"; return le;
}
const g1 = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(BigInt(p[1]));
const g1_neg = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(P - BigInt(p[1]));
const g2 = (p) => intToLEHex(BigInt(p[0][0])) + intToLEHex(BigInt(p[0][1])) + intToLEHex(BigInt(p[1][0])) + intToLEHex(BigInt(p[1][1]));

async function main() {
  const m = parseInt(process.argv[2] || "5");
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;
  const secrets = [1001n, 1002n, 1003n, 1004n, 1005n, 1006n, 1007n, 1008n];
  const commitments = secrets.map((s) => F.toString(poseidon([s])));
  const zeroHash = F.toString(poseidon([0]));
  const levels = [[...commitments]];
  while (levels[0].length < 1 << DEPTH) levels[0].push(zeroHash);
  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const next = [];
    for (let i = 0; i < levels[lvl].length; i += 2)
      next.push(F.toString(poseidon([levels[lvl][i], levels[lvl][i + 1]])));
    levels.push(next);
  }
  const root = levels[DEPTH][0];
  const secret = secrets[m];
  const nullifier = F.toString(poseidon([secret, 0]));
  const pathElements = [], pathIndices = [];
  let idx = m;
  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const sib = idx % 2 === 0 ? idx + 1 : idx - 1;
    pathElements.push(levels[lvl][sib]); pathIndices.push(idx % 2 === 0 ? 0 : 1);
    idx = Math.floor(idx / 2);
  }
  const input = {
    identity_secret: secret.toString(),
    pathElements: pathElements.map((e) => e.toString()),
    pathIndices: pathIndices.map((i) => i.toString()),
    merkle_root: root.toString(),
    nullifier: nullifier.toString(),
    claim_value: "1",
  };
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(
    input, path.join(IDENTITY, "circuit_js", "circuit.wasm"), path.join(IDENTITY, "squad_circuit_final.zkey"));
  const vkey = JSON.parse(fs.readFileSync(path.join(IDENTITY, "squad_vkey.json")));
  const ok = await snarkjs.groth16.verify(vkey, publicSignals, proof);
  if (!ok) throw new Error("local verify failed");
  fs.writeFileSync(`approve_m${m}.json`, JSON.stringify({
    negA: g1_neg(proof.pi_a), B: g2(proof.pi_b), C: g1(proof.pi_c),
    inputs: publicSignals.map((s) => intToLEHex(BigInt(s))),
  }));
  console.log(`member ${m} proof ✅ (nullifier ${nullifier.slice(0, 12)}…)`);
  process.exit(0); // snarkjs workers can hang exit
}
main().catch((e) => { console.error(e); process.exit(1); });
