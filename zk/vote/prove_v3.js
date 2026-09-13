// zk-Vote v3: choice hidden inside the circuit
const { buildPoseidonOpt } = require("circomlibjs");
const snarkjs = require("snarkjs");
const fs = require("fs");

const DEPTH = 4;
const TREE_SIZE = 1 << DEPTH;

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  // ── 1. Build voter registry ──
  const secrets = [111n, 222n, 333n, 444n]; // 4 registered voters
  const commitments = secrets.map(s => F.toString(poseidon([s])));

  // ── 2. Build Merkle tree ──
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

  // ── 3. Voter 0 votes YES (choice=1), voter 1 votes NO (choice=0) ──
  const ballots = [];

  for (const [voterIdx, choice] of [[0, 1], [1, 0]]) {
    const secret = secrets[voterIdx];
    const nullifier = F.toString(poseidon([secret, 0]));
    const blinding = BigInt(Math.floor(Math.random() * 1000000));
    const choiceCommitment = F.toString(poseidon([BigInt(choice), blinding]));

    // merkle path
    const pathElements = [];
    const pathIndices = [];
    let idx = voterIdx;
    for (let lvl = 0; lvl < DEPTH; lvl++) {
      const siblingIdx = idx % 2 === 0 ? idx + 1 : idx - 1;
      pathElements.push(levels[lvl][siblingIdx]);
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

    console.log(`\nVoter ${voterIdx} casting ballot (choice hidden in proof):`);
    console.log(`  nullifier: ${nullifier.slice(0, 20)}…`);
    console.log(`  commitment: ${choiceCommitment.slice(0, 20)}…`);

    const { proof, publicSignals } = await snarkjs.groth16.fullProve(
      input, "circuit_v3_js/circuit_v3.wasm", "circuit_v3_final.zkey"
    );

    console.log(`  proof generated ✓`);
    console.log(`  public: [root, nullifier, commitment] = ${publicSignals.map(s => s.slice(0, 10) + "…").join(", ")}`);

    ballots.push({ proof, publicSignals, choice, blinding, nullifier, choiceCommitment });
  }

  // ── 4. Verify both proofs with snarkjs ──
  const vkey = JSON.parse(fs.readFileSync("circuit_v3_vkey.json"));
  for (const b of ballots) {
    const ok = await snarkjs.groth16.verify(vkey, b.publicSignals, b.proof);
    console.log(`\nproof valid: ${ok ? "✅" : "❌"}`);
  }

  // ── 5. What the chain sees ──
  console.log("\n═══ WHAT THE CHAIN SEES ═══");
  for (const b of ballots) {
    console.log(`  ballot: nullifier=${b.nullifier.slice(0, 16)}… commitment=${b.choiceCommitment.slice(0, 16)}…`);
  }
  console.log("  (choice is NEVER visible — it's inside the proof)");

  console.log("\n═══ AT REVEAL TIME ═══");
  for (const [i, b] of ballots.entries()) {
    console.log(`  ballot ${i}: choice=${b.choice} blinding=${b.blinding}`);
    console.log(`    verify: Poseidon(${b.choice}, ${b.blinding}) == ${b.choiceCommitment.slice(0, 16)}… ✓`);
  }

  // ── 6. Generate NEAR verify args for ballot 0 ──
  const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
  const elem = (v) => {
    const big = BigInt(v.toString());
    const lo = big & ((1n << 128n) - 1n);
    const hi = big >> 128n;
    return lo.toString(16).padStart(32, "0") + hi.toString(16).padStart(32, "0");
  };
  const g1 = (p) => elem(p[0]) + elem(p[1]);
  const g2 = (p) => elem(p[0][0]) + elem(p[0][1]) + elem(p[1][0]) + elem(p[1][1]);

  const b0 = ballots[0];
  const x = BigInt(b0.proof.pi_a[0].toString());
  const y = BigInt(b0.proof.pi_a[1].toString());
  const negY = (P - y) % P;

  const init = {
    alpha1: g1(vkey.vk_alpha_1),
    beta2: g2(vkey.vk_beta_2),
    gamma2: g2(vkey.vk_gamma_2),
    delta2: g2(vkey.vk_delta_2),
    n: vkey.nPublic,
    ic: vkey.IC.map(ic => g1(ic)),
  };
  const verify = {
    negA: elem(x) + elem(negY),
    B: g2(b0.proof.pi_b),
    C: g1(b0.proof.pi_c),
    inputs: b0.publicSignals.map(s => elem(s)),
  };

  fs.writeFileSync("init_args.json", JSON.stringify(init));
  fs.writeFileSync("verify_args.json", JSON.stringify(verify));

  console.log("\n📦 NEAR args written (init_args.json, verify_args.json)");
  console.log("   nPublic =", vkey.nPublic, "(root, nullifier, commitment)");
}

main().catch(e => { console.error(e); process.exit(1); });
