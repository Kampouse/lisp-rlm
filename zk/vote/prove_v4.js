// Generate homomorphic encrypted ballots using CORRECT BN254 arithmetic
const { G, P, pointAdd, pointMul, onCurve, pointToHex } = require("./bn254");
const { buildPoseidonOpt } = require("circomlibjs");
const fs = require("fs");

const T_PRIVATE = 42n;
const T = pointMul(G, T_PRIVATE);

// Verify our point arithmetic is correct
console.log("G on curve:", onCurve(G));
console.log("T on curve:", onCurve(T));
console.log("2G on curve:", onCurve(pointAdd(G, G)));
console.log("3G on curve:", onCurve(pointAdd(pointAdd(G, G), G)));
console.log("G+(-G) = null:", pointAdd(G, [G[0], P - G[1]]) === null);

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  // Build voter registry
  const secrets = [111n, 222n, 333n, 444n];
  const commitments = secrets.map(s => F.toString(poseidon([s])));
  const zeroHash = F.toString(poseidon([0]));
  const levels = [[...commitments]];
  while (levels[0].length < 16) levels[0].push(zeroHash);
  for (let lvl = 0; lvl < 4; lvl++) {
    const next = [];
    for (let i = 0; i < levels[lvl].length; i += 2) {
      next.push(F.toString(poseidon([levels[lvl][i], levels[lvl][i + 1]])));
    }
    levels.push(next);
  }
  const root = levels[4][0];

  // 3 voters: yes, no, yes
  const voters = [
    { idx: 0, secret: 111n, choice: 1 },
    { idx: 1, secret: 222n, choice: 0 },
    { idx: 2, secret: 333n, choice: 1 },
  ];

  const ballots = [];
  let sumPoint = null;

  for (const voter of voters) {
    const nullifier = F.toString(poseidon([voter.secret, 0]));
    const r = BigInt(Math.floor(Math.random() * 1000000) + 1);

    // encrypted = choice·G + r·T
    const cG = pointMul(G, BigInt(voter.choice));
    const rT = pointMul(T, r);
    const encrypted = pointAdd(cG, rT);

    // Verify this point is on the curve
    if (!onCurve(encrypted)) {
      console.error("ERROR: encrypted point not on curve!");
      process.exit(1);
    }

    // Add to homomorphic sum
    sumPoint = pointAdd(sumPoint, encrypted);

    console.log(`voter${voter.idx}: choice=${voter.choice} | enc on curve: ${onCurve(encrypted)} | nullifier=${nullifier.slice(0, 16)}…`);

    // merkle path
    const pathElements = [];
    const pathIndices = [];
    let idx = voter.idx;
    for (let lvl = 0; lvl < 4; lvl++) {
      const sib = idx % 2 === 0 ? idx + 1 : idx - 1;
      pathElements.push(levels[lvl][sib]);
      pathIndices.push(idx % 2 === 0 ? 0 : 1);
      idx = Math.floor(idx / 2);
    }

    ballots.push({
      nullifier: nullifier,
      encryptedHex: pointToHex(encrypted),
      choice: voter.choice,
      r: r.toString(),
    });
  }

  // Verify homomorphic sum
  console.log("\nsum on curve:", onCurve(sumPoint));

  // Verify the sum equals (yes_count)·G + (Σr)·T
  const totalR = ballots.reduce((acc, b) => acc + BigInt(b.r), 0n);
  const yesCount = voters.filter(v => v.choice === 1).length;
  const check = pointAdd(pointMul(G, BigInt(yesCount)), pointMul(T, totalR));
  console.log("sum verification:", pointToHex(sumPoint) === pointToHex(check) ? "✅ MATCHES" : "❌ MISMATCH");
  console.log(`expected: yes=${yesCount}, Σr=${totalR}`);

  // Write ballot submission args
  const ballotArgs = ballots.map(b => ({
    nullifier: b.nullifier,
    encrypted: b.encryptedHex,
  }));
  fs.writeFileSync("ballots_v4.json", JSON.stringify(ballotArgs, null, 2));
  fs.writeFileSync("reveal_v4.json", JSON.stringify({
    tally_private: T_PRIVATE.toString(),
    total_r: totalR.toString(),
    yes_count: yesCount,
    tally_pubkey: pointToHex(T),
  }, null, 2));

  console.log("\n📦 ballots_v4.json + reveal_v4.json written");
  console.log(`   tally_pubkey: ${pointToHex(T).slice(0, 20)}…`);
  console.log(`   homomorphic_sum: ${pointToHex(sumPoint).slice(0, 20)}…`);
  console.log(`\nWhat the chain sees: nullifier + encrypted point (opaque)`);
  console.log(`What the chain NEVER sees: choices = ${voters.map(v => v.choice).join(", ")}`);
}

main().catch(e => { console.error(e); process.exit(1); });
