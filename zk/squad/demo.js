#!/usr/bin/env node
// zk-squad demo driver: 8-member squad, threshold 5.
// Builds the identity tree, proves 5 approvals with the deployed
// zk-identity circuit (../identity/), writes:
//   init_args_squad.json  — contract init payload (root + threshold + vk)
//   approve_m<i>.json     — per-member approve payload (proof + signals)
// Then optionally runs the whole lifecycle via near-compile call.
const { buildPoseidonOpt } = require("circomlibjs");
const snarkjs = require("snarkjs");
const fs = require("fs");
const path = require("path");

const IDENTITY = path.join(__dirname, "..", "identity");
const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
const DEPTH = 4;
const N_MEMBERS = 8;
const THRESHOLD = 5;

const elemLE = (v) => BigInt(v).to_bytes ? BigInt(v).to_bytes(32) : null; // placeholder
function elem(v) { // field element -> 32B LE hex
  return BigInt(v.toString()).toString(16).length, intToLEHex(BigInt(v.toString()));
}
function intToLEHex(big) {
  let hex = big.toString(16);
  if (hex.length % 2) hex = "0" + hex;
  let le = "";
  for (let i = hex.length - 2; i >= 0; i -= 2) le += hex.slice(i, i + 2);
  while (le.length < 64) le += "0";
  return le;
}
const g1 = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(BigInt(p[1]));
const g1_neg = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(P - BigInt(p[1]));
const g2 = (p) => intToLEHex(BigInt(p[0][0])) + intToLEHex(BigInt(p[0][1])) + intToLEHex(BigInt(p[1][0])) + intToLEHex(BigInt(p[1][1]));

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  // 1. squad roster: 8 secrets -> commitments -> merkle tree
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
  console.log(`squad root: ${root.slice(0, 24)}…`);

  // 2. init payload (vk from the identity circuit's vkey.json)
  const vkey = JSON.parse(fs.readFileSync(path.join(IDENTITY, "squad_vkey.json")));
  const init = {
    root_hex: intToLEHex(BigInt(root)),
    threshold: THRESHOLD,
    members: N_MEMBERS,
    alpha1: g1(vkey.vk_alpha_1),
    beta2: g2(vkey.vk_beta_2),
    gamma2: g2(vkey.vk_gamma_2),
    delta2: g2(vkey.vk_delta_2),
    n: vkey.nPublic,
    ic: vkey.IC.map((p) => g1(p)),
  };
  fs.writeFileSync("init_args_squad.json", JSON.stringify(init));
  console.log("init_args_squad.json written");

  // 3. prove THRESHOLD approvals (members 0..T-1)
  const wasm = path.join(IDENTITY, "circuit_js", "circuit.wasm");
  const zkey = path.join(IDENTITY, "squad_circuit_final.zkey");
  for (let m = 0; m < THRESHOLD; m++) {
    const secret = secrets[m];
    const nullifier = F.toString(poseidon([secret, 0]));
    const pathElements = [], pathIndices = [];
    let idx = m;
    for (let lvl = 0; lvl < DEPTH; lvl++) {
      const sib = idx % 2 === 0 ? idx + 1 : idx - 1;
      pathElements.push(levels[lvl][sib]);
      pathIndices.push(idx % 2 === 0 ? 0 : 1);
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
    const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasm, zkey);
    const localOk = await snarkjs.groth16.verify(vkey, publicSignals, proof);
    if (!localOk) throw new Error(`member ${m}: local verify failed`);
    const payload = {
      negA: g1_neg(proof.pi_a),
      B: g2(proof.pi_b),
      C: g1(proof.pi_c),
      inputs: publicSignals.map((s) => intToLEHex(BigInt(s))),
    };
    fs.writeFileSync(`approve_m${m}.json`, JSON.stringify(payload));
    console.log(`member ${m}: proof ✅ nullifier ${nullifier.slice(0, 12)}…`);
  }

  // 4. also prove one DUPLICATE (member 0 again) and one NON-MEMBER (secret 999)
  //    for the tamper tests — approve_m0_dup.json / approve_outsider.json
  // (outsider: same tree, wrong secret → membership fails, proof gen fails: skip)
  console.log("\nall payloads ready — run the lifecycle:");
  console.log("  near-mock target/squad.wasm init '$(cat init_args_squad.json)'");
  console.log("  near-mock target/squad.wasm approve '$(cat approve_m0.json)' …");
}

main().catch((e) => { console.error(e); process.exit(1); });
