// Step-by-step: generate witness, then prove, then bridge — with timing
const { buildPoseidonOpt } = require("circomlibjs");
const fs = require("fs");

const DEPTH = 4;
const TREE_SIZE = 1 << DEPTH;

async function main() {
  const t0 = Date.now();
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  // Build voter registry (same as before)
  const secrets = [111n, 222n, 333n, 444n];
  const commitments = secrets.map(s => F.toString(poseidon([s])));
  const zeroHash = F.toString(poseidon([0]));
  const levels = [[...commitments]];
  while (levels[0].length < TREE_SIZE) levels[0].push(zeroHash);
  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const next = [];
    for (let i = 0; i < levels[lvl].length; i += 2) {
      next.push(F.toString(poseidon([levels[lvl][i], levels[lvl][i + 1]])));
    }
    levels.push(next);
  }
  const root = levels[DEPTH][0];
  console.log(`setup: ${Date.now() - t0}ms`);

  // Voter 0: choice=1 (yes), blinding=12345
  const voter = 0, choice = 1, blinding = 12345;
  const secret = secrets[voter];
  const nullifier = F.toString(poseidon([secret, 0]));
  const choiceCommitment = F.toString(poseidon([BigInt(choice), BigInt(blinding)]));

  const pathElements = [];
  const pathIndices = [];
  let idx = voter;
  for (let lvl = 0; lvl < DEPTH; lvl++) {
    const sib = idx % 2 === 0 ? idx + 1 : idx - 1;
    pathElements.push(levels[lvl][sib]);
    pathIndices.push(idx % 2 === 0 ? 0 : 1);
    idx = Math.floor(idx / 2);
  }

  const input = {
    identity_secret: secret.toString(),
    pathElements: pathElements.map(e => e.toString()),
    pathIndices: pathIndices.map(i => i.toString()),
    choice: choice.toString(),
    blinding: blinding.toString(),
    merkle_root: root.toString(),
    nullifier: nullifier.toString(),
    choice_commitment: choiceCommitment.toString(),
  };

  fs.writeFileSync("input.json", JSON.stringify(input, null, 2));
  console.log(`input written: ${Date.now() - t0}ms`);

  // Witness generation (the slow JS→wasm call)
  console.log("generating witness…");
  const { execSync } = require("child_process");
  execSync(
    "node circuit_v3_js/generate_witness.js circuit_v3_js/circuit_v3.wasm input.json witness.wtns",
    { stdio: "pipe", timeout: 120000 }
  );
  console.log(`witness done: ${Date.now() - t0}ms`);

  // Prove (snarkjs CLI, faster than the JS API)
  console.log("proving…");
  execSync(
    "npx snarkjs groth16 prove circuit_v3_final.zkey witness.wtns proof.json public.json",
    { stdio: "pipe", timeout: 300000 }
  );
  console.log(`proof done: ${Date.now() - t0}ms`);

  // Write reveal data (what the voter publishes at tally time)
  fs.writeFileSync("reveal.json", JSON.stringify({
    nullifier: nullifier,
    choice_commitment: choiceCommitment,
    choice: choice,
    blinding: blinding,
  }, null, 2));

  const pub = JSON.parse(fs.readFileSync("public.json"));
  console.log("\npublic signals (what the chain sees):");
  console.log(`  root:        ${pub[0].slice(0, 20)}…`);
  console.log(`  nullifier:   ${pub[1].slice(0, 20)}…`);
  console.log(`  commitment:  ${pub[2].slice(0, 20)}…`);
  console.log(`  (choice=${choice} is NOT here — hidden in the proof)`);

  console.log(`\ntotal: ${Date.now() - t0}ms`);
}

main().catch(e => { console.error(e); process.exit(1); });
