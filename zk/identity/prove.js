// Client: build identity tree, create witness, prove, verify
// The "eligible set" = list of identity commitments (Poseidon of secrets)
const { buildPoseidonOpt } = require("circomlibjs");
const { wasm: wasm_tester } = require("circom_tester");
const snarkjs = require("snarkjs");
const fs = require("fs");
const path = require("path");

const DEPTH = 4;
const TREE_SIZE = 1 << DEPTH; // 16

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  // ── 1. Build the identity set ──
  // Each member has a secret; commitment = Poseidon(secret)
  const secrets = [123n, 456n, 789n, 101112n]; // 4 members
  const commitments = secrets.map(s => F.toString(poseidon([s])));

  console.log("Identity commitments:");
  commitments.forEach((c, i) => console.log(`  [${i}] ${c.slice(0, 20)}…`));

  // ── 2. Build Merkle tree (Poseidon hash, depth 4) ──
  const zeroHash = F.toString(poseidon([0]));
  const levels = [[...commitments]];
  // pad with zero hashes
  while (levels[0].length < TREE_SIZE) levels[0].push(zeroHash);

  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const next = [];
    for (let i = 0; i < levels[lvl].length; i += 2) {
      next.push(F.toString(poseidon([levels[lvl][i], levels[lvl][i + 1]])));
    }
    levels.push(next);
  }
  const root = levels[DEPTH][0];
  console.log(`\nMerkle root: ${root.slice(0, 30)}…`);

  // ── 3. Generate witness for member 0 ──
  const memberIdx = 0;
  const secret = secrets[memberIdx];
  const commitment = commitments[memberIdx];

  // compute nullifier
  const nullifier = F.toString(poseidon([secret, 0]));

  // compute merkle path
  const pathElements = [];
  const pathIndices = [];
  let idx = memberIdx;
  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const siblingIdx = idx % 2 === 0 ? idx + 1 : idx - 1;
    pathElements.push(levels[lvl][siblingIdx]);
    pathIndices.push(idx % 2 === 0 ? 0 : 1); // 0 = left, 1 = right
    idx = Math.floor(idx / 2);
  }

  console.log(`\nMember 0 proving membership:`);
  console.log(`  secret: ${secret}`);
  console.log(`  nullifier: ${nullifier.slice(0, 20)}…`);
  console.log(`  path: [${pathIndices.join(",")}]`);

  // ── 4. Create witness and prove ──
  const input = {
    identity_secret: secret.toString(),
    pathElements: pathElements.map(e => e.toString()),
    pathIndices: pathIndices.map(i => i.toString()),
    merkle_root: root.toString(),
    nullifier: nullifier.toString(),
    claim_value: "1",
  };

  // write witness input
  fs.writeFileSync("input.json", JSON.stringify(input, null, 2));
  console.log("\ninput.json written — proving with snarkjs…");

  // ── 5. Prove ──
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(
    input,
    "circuit_js/circuit.wasm",
    "circuit_final.zkey"
  );

  fs.writeFileSync("proof.json", JSON.stringify(proof, null, 2));
  fs.writeFileSync("public.json", JSON.stringify(publicSignals, null, 2));

  console.log("\n✅ Proof generated!");
  console.log(`  public signals: [root, nullifier, claim]=${publicSignals.map(s => s.slice(0, 12) + "…").join(", ")}`);

  // ── 6. Verify locally (snarkjs) ──
  const vkey = JSON.parse(fs.readFileSync("vkey.json"));
  const valid = await snarkjs.groth16.verify(vkey, publicSignals, proof);
  console.log(`\n${valid ? "✅ snarkjs verify: VALID" : "❌ snarkjs verify: INVALID"}`);

  // ── 7. Generate NEAR call args ──
  const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
  const elem = (v) => BigInt(v).toString(16).padStart(64, "0");
  const elemLE = (v) => {
    const big = BigInt(v.toString());
    const lo = big & ((1n << 128n) - 1n);
    const hi = big >> 128n;
    return lo.toString(16).padStart(32, "0") + hi.toString(16).padStart(32, "0");
  };

  const negPiA = () => {
    const x = BigInt(proof.pi_a[0].toString());
    const y = BigInt(proof.pi_a[1].toString());
    const negY = (P - y) % P;
    return elemLE(x) + elemLE(negY);
  };
  const g2 = (p) => elemLE(p[0][0]) + elemLE(p[0][1]) + elemLE(p[1][0]) + elemLE(p[1][1]);
  const g1 = (p) => elemLE(p[0]) + elemLE(p[1]);

  const verifyArgs = {
    negA: negPiA(),
    B: g2(proof.pi_b),
    C: g1(proof.pi_c),
    inputs: publicSignals.map(s => elemLE(s)),
  };

  const initArgs = {
    alpha1: g1(vkey.vk_alpha_1),
    beta2: g2(vkey.vk_beta_2),
    gamma2: g2(vkey.vk_gamma_2),
    delta2: g2(vkey.vk_delta_2),
    n: vkey.nPublic,
    ic: vkey.IC.map(ic => g1(ic)),
  };

  fs.writeFileSync("init_args.json", JSON.stringify(initArgs));
  fs.writeFileSync("verify_args.json", JSON.stringify(verifyArgs));

  console.log("\n📦 NEAR args written:");
  console.log(`  init_args.json (${JSON.stringify(initArgs).length} bytes)`);
  console.log(`  verify_args.json (${JSON.stringify(verifyArgs).length} bytes)`);
  console.log(`\nReady to deploy + verify on NEAR!`);
}

main().catch(e => { console.error(e); process.exit(1); });
