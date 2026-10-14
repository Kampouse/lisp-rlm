#!/usr/bin/env node
// zk-squad v2 e2e driver: sealed-tally threshold voting, full lifecycle.
//
//   1. roster tree (8 secrets) + tallier ed25519 keypairs
//   2. anonymous enroll payloads (zk proof nonce=1, tallier pk)
//   3. sealed ballots (zk proof nonce=2 + Λ_indv PVSS blob) — ALL members
//      ballot in-window (cover traffic: participation pattern reveals nothing)
//   4. off-chain tally by closers (verify blobs → homomorphic tally → V*)
//   5. close sigs (t distinct talliers, ed25519 over sha256 statement)
//   6. near-mock lifecycle: enroll → ballots → close → challenge → execute
//
// Scenarios: A = YES (Σv=5 ≥ t=5) · B = NO (Σv=3) on separate states.
const { buildPoseidonOpt } = require("circomlibjs");
const snarkjs = require("snarkjs");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { ed25519 } = require("@noble/curves/ed25519.js");
const P = require("./lib/pvss.js");

const DEPTH = 4;
const N = 8;
const T = 5;
const HERE = __dirname;
const WASM = path.join(HERE, "circuit_js", "circuit.wasm");
const ZKEY = path.join(HERE, "circuit_final.zkey");
const VKEY = JSON.parse(fs.readFileSync(path.join(HERE, "vkey.json")));
const WASM_C = path.join(HERE, "target", "squad-v2.wasm");

function intToLEHex(big) {
  let hex = BigInt(big).toString(16);
  if (hex.length % 2) hex = "0" + hex;
  let le = "";
  for (let i = hex.length - 2; i >= 0; i -= 2) le += hex.slice(i, i + 2);
  while (le.length < 64) le += "0";
  return le;
}
const g1 = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(BigInt(p[1]));
const g1n = (p) => intToLEHex(BigInt(p[0])) + intToLEHex(21888242871839275222246405745257275088696311157297823662689037894645226208583n - BigInt(p[1]));
const g2 = (p) =>
  intToLEHex(BigInt(p[0][0])) + intToLEHex(BigInt(p[0][1])) +
  intToLEHex(BigInt(p[1][0])) + intToLEHex(BigInt(p[1][1]));

// ed25519 tallier keypair: seed → clamped scalar (PVSS) + sign/verify (host)
function tallierKeypair(i) {
  const seed = crypto.createHash("sha256").update(`squad-v2:tallier:${i}`).digest();
  const pk = ed25519.getPublicKey(seed);          // 32B compressed
  // RFC8032 scalar expand (for PVSS ops)
  const h = crypto.createHash("sha512").update(seed).digest();
  h[0] &= 248; h[31] &= 127; h[31] |= 64;
  let sk = 0n; for (const b of h.subarray(0, 32)) sk = (sk << 8n) | BigInt(b); // BE — NO: LE
  let skle = 0n; for (let k = 31; k >= 0; k--) skle = (skle << 8n) | BigInt(h[k]);
  const pkPoint = P.G_EN.multiply(skle % P.L);   // clamped may exceed L — reduce (same point: |G|=L)
  if (pkPoint.toHex() !== Buffer.from(pk).toString("hex")) {
    throw new Error("keypair mismatch: PVSS pk != ed25519 pk");
  }
  return { seed, pkHex: Buffer.from(pk).toString("hex"), sk: skle, pkPoint };
}

async function prove(poseidon, F, input) {
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, WASM, ZKEY);
  const ok = await snarkjs.groth16.verify(VKEY, publicSignals, proof);
  if (!ok) throw new Error("local verify failed");
  return {
    negA: g1n(proof.pi_a),
    B: g2(proof.pi_b),
    C: g1(proof.pi_c),
    inputs: publicSignals.map((s) => intToLEHex(BigInt(s))),
  };
}

function treeFor(poseidon, F, secrets) {
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
  const path = (idx) => {
    const els = [], inds = [];
    for (let lvl = 0; lvl < DEPTH; lvl++) {
      const sib = idx % 2 === 0 ? idx + 1 : idx - 1;
      els.push(levels[lvl][sib]);
      inds.push(idx % 2 === 0 ? 0 : 1);
      idx = Math.floor(idx / 2);
    }
    return { els, inds };
  };
  return { root, path };
}

// blob parser (proves the bulletin-board round-trip: closers read blobs back)
function ptFromHex(ed, hex) { return ed.Point.fromHex(hex); }
function scFromHex(hex) { return BigInt("0x" + hex); }
function parseBlob(blob, n, t) {
  let o = 0;
  const take = (k) => { const s = blob.slice(o, o + k); o += k; return s; };
  const y0 = ptFromHex(ed25519, take(64));
  const ys = []; for (let i = 0; i < n; i++) ys.push(ptFromHex(ed25519, take(64)));
  const U = ptFromHex(ed25519, take(64));
  const d = scFromHex(take(64));
  const z = []; for (let i = 0; i < t + 1; i++) z.push(scFromHex(take(64)));
  const a0 = ptFromHex(ed25519, take(64)), b0 = ptFromHex(ed25519, take(64));
  const a1 = ptFromHex(ed25519, take(64)), b1 = ptFromHex(ed25519, take(64));
  const d0 = scFromHex(take(64)), r0 = scFromHex(take(64));
  const d1 = scFromHex(take(64)), r1 = scFromHex(take(64));
  return { y0, ys, U, d, z, piU: { a0, b0, a1, b1, d0, r0, d1, r1 } };
}

const { execSync } = require("child_process");
function mockCall(state, method, args, nowS, wantFail) {
  const env = { ...process.env, NEAR_MOCK_NOW: String(nowS), NEAR_MOCK_SIGNER: "demo.test.near" };
  const cmd = `near-mock ${JSON.stringify(WASM_C)} ${method} ${JSON.stringify(JSON.stringify(args)).replace(/"/g, '\\"')} --state ${state}`;
  try {
    const out = execSync(`NEAR_MOCK_NOW=${nowS} NEAR_MOCK_SIGNER=demo.test.near near-mock ${JSON.stringify(WASM_C)} ${method} ${shellArg(JSON.stringify(args))} --state ${state}`, { env, encoding: "utf8", shell: "/bin/zsh" });
    const lines = out.trim().split("\n").filter((l) => l.length);
    let line = lines[lines.length - 1] || "";
    for (let i = lines.length - 1; i >= 0; i--) if (lines[i].includes('{"')) { line = lines[i].slice(lines[i].indexOf("{")); break; }
    return { ok: true, out: line };
  } catch (e) {
    const msg = (e.stderr || e.stdout || e.message || "").toString();
    if (wantFail) return { ok: false, out: msg };
    console.error(`NEAR-MOCK ${method} FAILED:\n…${msg.slice(-600)}`);
    process.exit(1);
  }
}
function shellArg(s) { return "'" + s.replace(/'/g, "'\\''") + "'"; }

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;
  const secrets = [1001n, 1002n, 1003n, 1004n, 1005n, 1006n, 1007n, 1008n];
  const { root, path } = treeFor(poseidon, F, secrets);
  const rootLE = intToLEHex(BigInt(root));
  console.log(`roster root: ${root.slice(0, 20)}…`);

  // tallier keys (members = talliers)
  const talliers = []; for (let i = 0; i < N; i++) talliers.push(tallierKeypair(i));
  const PKs = talliers.map((t) => t.pkPoint);
  console.log(`tallier keys: ${N} (ed25519, dual-use PVSS+sign)`);

  // ── enroll proofs (nonce=1) ──
  console.log("\n[1] anonymous enrollment (nonce=1 proofs)…");
  for (let m = 0; m < N; m++) {
    const nullifier = F.toString(poseidon([secrets[m], 1]));
    const { els, inds } = path(m);
    const proof = await prove(poseidon, F, {
      identity_secret: secrets[m].toString(),
      pathElements: els.map(String), pathIndices: inds.map(String),
      merkle_root: root.toString(), nullifier, claim_value: "1", nullifier_nonce: "1",
    });
    talliers[m].enroll = { ...proof, pk_hex: talliers[m].pkHex };
    fs.writeFileSync(`/tmp/enroll_m${m}.json`, JSON.stringify(talliers[m].enroll));
    console.log(`  member ${m}: enrolled pk ${talliers[m].pkHex.slice(0, 12)}… (anon)`);
  }

  // ── scenarios ──
  for (const [name, votes, wantYES] of [
    ["A-YES", [1, 0, 1, 0, 1, 1, 0, 1], true],   // Σv=5 = t
    ["B-NO",  [1, 0, 1, 0, 0, 0, 0, 0], false],  // Σv=3 < t
  ]) {
    console.log(`\n═══ scenario ${name} (Σv=${votes.reduce((a, b) => a + b, 0)}) ═══`);
    await scenario(poseidon, F, secrets, path, root, rootLE, talliers, PKs, votes, wantYES, name);
  }
  console.log("\nALL SCENARIOS ✅");
}

async function scenario(poseidon, F, secrets, path, root, rootLE, talliers, PKs, votes, wantYES, tag) {
  const state = path_join(HERE, `state_${tag}.bin`);
  try { fs.unlinkSync(state); } catch {}
  const T0 = 1787000500, T1 = 1787000900, CHALLENGE = 100;

  // init at T0-300
  const initArgs = {
    root_hex: rootLE, threshold: T, members: N, t0: T0, t1: T1, challenge_secs: CHALLENGE,
    alpha1: g1(VKEY.vk_alpha_1), beta2: g2(VKEY.vk_beta_2),
    gamma2: g2(VKEY.vk_gamma_2), delta2: g2(VKEY.vk_delta_2),
    n: VKEY.nPublic, ic: VKEY.IC.map((p) => g1(p)),
  };
  let r = mockCall(state, "init", initArgs, T0 - 300);
  console.log(`init: ${r.out.slice(0, 60)}`);

  // enroll all 8 before window opens
  for (let m = 0; m < N; m++) {
    r = mockCall(state, "enroll", talliers[m].enroll, T0 - 100);
  }
  // dup enroll must fail (same nonce=1 nullifier)
  r = mockCall(state, "enroll", talliers[0].enroll, T0 - 100, true);
  console.log(`dup enroll rejected: ${r.ok === false ? "YES" : "NO! " + r.out}`);

  r = mockCall(state, "status", {}, T0 - 100);
  console.log(`status: ${r.out}`);

  // ballots — ALL members ballot in-window (cover traffic)
  console.log("[2] sealed ballots (nonce=2 + Λ_indv blobs)…");
  const ballots = [];
  for (let m = 0; m < N; m++) {
    const B = P.share(votes[m], PKs, T - 2 >= 2 ? T - 2 : 3);   // deg bound: robustness n≥2t+ℓ ⇒ tally-deg t_v; use 3 (t=5: n≥2·3+1 ✓)
    const blob = P.serializeBallot(B);
    // blob round-trip + off-chain acceptance (what every member/watcher runs)
    const parsed = parseBlob(blob, N, 3);
    if (!P.verifyBallot(parsed, PKs, 3)) throw new Error(`member ${m}: blob round-trip verify failed`);
    const nullifier = F.toString(poseidon([secrets[m], 2]));
    const { els, inds } = path(m);
    const proof = await prove(poseidon, F, {
      identity_secret: secrets[m].toString(),
      pathElements: els.map(String), pathIndices: inds.map(String),
      merkle_root: root.toString(), nullifier, claim_value: "1", nullifier_nonce: "2",
    });
    talliers[m].ballot = { ...proof, blob_hex: blob };
    ballots.push(parsed);
  }
  console.log(`  ${N}/${N} blobs off-chain verified`);

  for (let m = 0; m < N; m++) {
    mockCall(state, "ballot", talliers[m].ballot, T0 + 100);
  }
  // dup ballot (same nonce=2 nullifier) must fail
  r = mockCall(state, "ballot", talliers[0].ballot, T0 + 100, true);
  console.log(`dup ballot rejected: ${r.ok === false ? "YES" : "NO!"}`);
  // late ballot must fail
  r = mockCall(state, "ballot", talliers[7].ballot, T1 + 1, true);
  console.log(`late ballot rejected: ${r.ok === false ? "YES" : "NO!"}`);

  // ── close: closers tally off-chain over ACCEPTED blobs, then sign ──
  console.log("[3] quorum close…");
  const { Vstar, sum } = P.tally(ballots, PKs, talliers.map((t) => t.sk % P.L), 3);
  const vstarHex = Vstar.toHex();
  const result = sum >= T ? "YES" : "NO";
  if ((result === "YES") !== wantYES) throw new Error(`tally mismatch: sum=${sum} result=${result}`);
  console.log(`  V* = G^${sum}, result=${result}`);
  const stmt = `zk-squad-v2:close|${rootLE}|${vstarHex}|${sum}|${result}`;
  const stmtHash = crypto.createHash("sha256").update(stmt).digest("hex");
  const closers = [0, 1, 2, 3, 4];   // t distinct members
  const pks = closers.map((i) => talliers[i].pkHex);
  const sigs = closers.map((i) => Buffer.from(ed25519.sign(stmtHashBytes(), talliers[i].seed)).toString("hex"));
  function stmtHashBytes() { return Buffer.from(stmtHash, "hex"); }

  // early close must fail (window still open)
  r = mockCall(state, "close", { vstar_hex: vstarHex, sum, result, pks, sigs }, T1 - 10, true);
  console.log(`early close rejected: ${r.ok === false ? "YES" : "NO!"}`);
  // bad sig must fail
  r = mockCall(state, "close", { vstar_hex: vstarHex, sum, result, pks, sigs: sigs.map((s, i) => i === 0 ? "00".repeat(64) : s) }, T1 + 10, true);
  console.log(`bad sig close rejected: ${r.ok === false ? "YES" : "NO!"}`);
  // insufficient sigs must fail
  r = mockCall(state, "close", { vstar_hex: vstarHex, sum, result, pks: pks.slice(0, 4), sigs: sigs.slice(0, 4) }, T1 + 10, true);
  console.log(`4-of-5 close rejected: ${r.ok === false ? "YES" : "NO!"}`);
  // honest close
  fs.writeFileSync('/tmp/close_payload.json', JSON.stringify({ vstar_hex: vstarHex, sum, result, pks, sigs }));
  r = mockCall(state, "close", { vstar_hex: vstarHex, sum, result, pks, sigs }, T1 + 10);
  console.log(`close: ${r.out.slice(0, 80)}`);

  // execute during challenge → rejected; after → fires/holds
  r = mockCall(state, "execute", {}, T1 + 50, true);
  console.log(`execute during challenge rejected: ${r.ok === false ? "YES" : "NO!"}`);
  r = mockCall(state, "execute", {}, T1 + 10 + CHALLENGE + 10);
  r = mockCall(state, "status", {}, T1 + 10 + CHALLENGE + 20);
  const st = JSON.parse(r.out);
  console.log(`final: ${r.out}`);
  if (st.phase !== "executed") throw new Error("not executed");
  if (wantYES && st.fired !== 1) throw new Error("YES scenario did not fire");
  if (!wantYES && st.fired !== 0) throw new Error("NO scenario fired");
  console.log(wantYES ? "  🎯 FIRED (sealed tally YES)" : "  ⏸️ HELD (sealed tally NO)");
}
function path_join(a, b) { return a + "/" + b; }

main().catch((e) => { console.error(e); process.exit(1); });
