#!/usr/bin/env bun
// gaskey — session-key engine for NEAR (NEP-611 gas keys).
// One command per action; sessions persist in ~/.gaskey/sessions.json.
//
//   gaskey create <session> --contract <acct> --methods m1,m2 --lanes 4 --fund 0.5
//   gaskey call <session> <method> '<json-args>' [--deposit 0] [--lane N]
//   gaskey volley <session> <method> '<json-args>' --count 8
//   gaskey topup <session> --amount 1
//   gaskey status <session>
//   gaskey list
//   gaskey revoke <session>
//
// The first command for a session uses the owner's full key (ONE popup/one
// signature); every `call` after that is signed locally by the gas key —
// zero popups, no wallet, no relayer.

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { actions, queryAccessKey, queryGasKeyNonces, sendTx } from "@fastnear/api";
import { privateKeyFromRandom, signerFromPrivateKey } from "@fastnear/utils";

const HOME = process.env.HOME;
const DIR = `${HOME}/.gaskey`;
const FILE = `${DIR}/sessions.json`;
const NET = process.env.GASKEY_NETWORK || "testnet";
const RPC = process.env.GASKEY_RPC; // optional: fastnear/foldsep RPC with higher limits

function loadSessions() {
  if (!existsSync(FILE)) return {};
  return JSON.parse(readFileSync(FILE, "utf8"));
}
function saveSessions(s) {
  mkdirSync(DIR, { recursive: true });
  writeFileSync(FILE, JSON.stringify(s, null, 1));
}
function ownerSigner(accountId) {
  const p = `${HOME}/.near-credentials/${NET}/${accountId}.json`;
  if (!existsSync(p)) throw new Error(`no local key for ${accountId} at ${p}`);
  const cred = JSON.parse(readFileSync(p, "utf8"));
  return signerFromPrivateKey(cred.secret_key || cred.private_key);
}
function gasSignerOf(sess) {
  return signerFromPrivateKey(sess.gas_secret);
}
const arg = (name, i = 3) => {
  const k = process.argv.indexOf(name);
  return k === -1 ? undefined : process.argv[k + 1];
};
const die = (m) => { console.error("❌ " + m); process.exit(1); };

const [cmd, name, ...rest] = process.argv.slice(2);

// ── create ──────────────────────────────────────────────────────────
if (cmd === "create") {
  const contract = arg("--contract");
  if (!contract) die("usage: gaskey create <session> --contract <acct> --methods buy,sell --lanes 4 --fund 0.5");
  const methods = (arg("--methods") || "").split(",").filter(Boolean);
  const lanes = Math.min(1024, Math.max(1, parseInt(arg("--lanes") || "4", 10)));
  const fund = arg("--fund") || "0.5";
  const owner = arg("--owner") || die("--owner <accountId> required (must have a local key)");
  const sessions = loadSessions();
  if (sessions[name]) die(`session '${name}' already exists`);

  const oSigner = ownerSigner(owner);
  const gasPriv = privateKeyFromRandom("ed25519");
  const gSigner = signerFromPrivateKey(gasPriv);

  await sendTx({
    signerId: owner, signer: oSigner, receiverId: owner,
    actions: [
      actions.addLimitedAccessGasKey({
        publicKey: gSigner.publicKey, numNonces: lanes,
        accountId: contract, methodNames: methods,
      }),
      actions.transferToGasKey({ publicKey: gSigner.publicKey, deposit: `${fund} NEAR` }),
    ],
    waitUntil: "FINAL", ...(RPC ? { rpcUrl: RPC } : { network: NET }),
  });

  sessions[name] = {
    owner, contract, methods, lanes,
    gas_public: gSigner.publicKey, gas_secret: gasPriv,
    created: new Date().toISOString(), calls: 0,
  };
  saveSessions(sessions);
  console.log(`✅ session '${name}': gas key pinned to ${contract} [${methods.join(", ")}], ${lanes} lanes, ${fund}N prepaid`);
  console.log(`   call it: gaskey call ${name} <method> '<json>'`);
  process.exit(0);
}

// ── call ────────────────────────────────────────────────────────────
if (cmd === "call") {
  const sessions = loadSessions();
  const sess = sessions[name] || die(`no session '${name}' (gaskey create first)`);
  const method = rest[0];
  if (!method) die("usage: gaskey call <session> <method> '<json-args>' [--lane N]");
  if (sess.methods.length && !sess.methods.includes(method))
    die(`'${method}' not whitelisted (${sess.methods.join(", ")})`);
  let args = {};
  if (rest[1]) { try { args = JSON.parse(rest[1]); } catch { die("args must be valid JSON"); } }
  const lane = parseInt(arg("--lane") || "0", 10);
  const deposit = arg("--deposit") || "0";
  if (deposit !== "0") die("gas keys cannot attach deposits (protocol rule) — use deposit 0 methods");

  const r = await sendTx({
    signerId: sess.owner, signer: gasSignerOf(sess), receiverId: sess.contract,
    nonceIndex: lane,
    actions: [actions.functionCall({ methodName: method, args, deposit: "0", gas: `${parseInt(arg("--gas") || "120", 10)} Tgas` })],
    waitUntil: "FINAL", ...(RPC ? { rpcUrl: RPC } : { network: NET }),
  });
  sess.calls += 1;
  saveSessions(sessions);
  const body = JSON.stringify(r);
  if (body.includes('"Failure"')) die("tx failed — see explorer");
  let result = "";
  for (const ro of r.receipts_outcome || []) {
    for (const l of ro.outcome.logs || []) if (l.includes("EVENT_JSON") || l.includes("📝")) result += l + "\n";
  }
  // surface the function-call value from the result object if present
  const vc = r.receipts_outcome?.find((x) => x.outcome?.status?.SuccessValue);
  if (vc) {
    const b64 = vc.outcome.status.SuccessValue;
    if (b64) result += "→ " + Buffer.from(b64, "base64").toString();
  }
  console.log(`✅ ${sess.contract}.${method} (lane ${lane}, call #${sess.calls})`);
  if (result) console.log(result.trim());
  process.exit(0);
}

// ── volley (N parallel, one per lane) ───────────────────────────────
if (cmd === "volley") {
  const sessions = loadSessions();
  const sess = sessions[name] || die(`no session '${name}'`);
  const method = rest[0];
  let args = rest[1] ? JSON.parse(rest[1]) : {};
  const count = parseInt(arg("--count") || String(sess.lanes), 10);
  sess.busy = sess.busy || {};
  const now = Date.now();
  const free = [];
  for (let l = 0; l < sess.lanes && free.length < count; l++) {
    if (!sess.busy[l] || now - sess.busy[l] > 6000) free.push(l);
  }
  if (free.length < count) die(`only ${free.length} free lanes (6s busy window) — wait or add lanes`);
  free.forEach((l) => { sess.busy[l] = now; });
  saveSessions(sessions);
  const t0 = Date.now();
  const rs = await Promise.all(free.map(async (lane, i) => {
    if (i > 0) await new Promise((res) => setTimeout(res, i * 250)); // stagger past RPC rate limits
    return sendTx({
      signerId: sess.owner, signer: gasSignerOf(sess), receiverId: sess.contract,
      nonceIndex: lane,
      actions: [actions.functionCall({ methodName: method, args, deposit: "0", gas: "120 Tgas" })],
      waitUntil: "FINAL", ...(RPC ? { rpcUrl: RPC } : { network: NET }),
    }).then(() => ({ lane, ok: true })).catch((e) => ({ lane, ok: false, err: String(e).slice(0, 120) }));
  }));
  const ms = Date.now() - t0;
  const okN = rs.filter((r) => r.ok).length;
  sess.calls += okN;
  free.forEach((l) => { sess.busy[l] = Date.now(); });
  saveSessions(sessions);
  console.log(`✅ volley: ${okN}/${count} landed in ${ms}ms (${(ms / count).toFixed(0)}ms/trade effective)`);
  rs.filter((r) => !r.ok).forEach((r) => console.log(`  lane ${r.lane}: ${r.err}`));
  process.exit(0);
}

// ── topup ───────────────────────────────────────────────────────────
if (cmd === "topup") {
  const sessions = loadSessions();
  const sess = sessions[name] || die(`no session '${name}'`);
  const amount = arg("--amount") || "0.5";
  await sendTx({
    signerId: sess.owner, signer: ownerSigner(sess.owner), receiverId: sess.owner,
    actions: [actions.transferToGasKey({ publicKey: sess.gas_public, deposit: `${amount} NEAR` })],
    waitUntil: "FINAL", ...(RPC ? { rpcUrl: RPC } : { network: NET }),
  });
  console.log(`✅ +${amount}N prepaid gas on '${name}'`);
  process.exit(0);
}

// ── status / list / revoke ──────────────────────────────────────────
if (cmd === "status") {
  const sessions = loadSessions();
  const sess = sessions[name] || die(`no session '${name}'`);
  const k = await queryAccessKey({ accountId: sess.owner, publicKey: sess.gas_public, blockId: "final", network: NET });
  const perm = k.result?.permission?.GasKeyFunctionCall;
  const nonces = await queryGasKeyNonces({ accountId: sess.owner, publicKey: sess.gas_public, blockId: "final", network: NET });
  const rawArr = nonces.result?.nonces || nonces.nonces || [];
  const arr = rawArr.map((x) => `L${x.nonce_index ?? x.nonceIndex}:${x.nonce}`).join(" ");
  console.log(`session '${name}' → ${sess.contract}`);
  console.log(`  methods: ${sess.methods.join(", ") || "ALL"}  lanes: ${sess.lanes}  calls made: ${sess.calls}`);
  if (perm) console.log(`  prepaid: ${(Number(BigInt(perm.balance)) / 1e24).toFixed(4)} N`);
  if (arr) console.log(`  lanes:   ${arr}`);
  process.exit(0);
}
if (cmd === "list") {
  const sessions = loadSessions();
  for (const [n, s] of Object.entries(sessions))
    console.log(`${n.padEnd(14)} → ${s.contract.padEnd(34)} [${s.methods.join(",")}] calls:${s.calls}`);
  process.exit(0);
}
if (cmd === "revoke") {
  const sessions = loadSessions();
  const sess = sessions[name] || die(`no session '${name}'`);
  await sendTx({
    signerId: sess.owner, signer: ownerSigner(sess.owner), receiverId: sess.owner,
    actions: [actions.deleteKey({ publicKey: sess.gas_public })],
    waitUntil: "FINAL", ...(RPC ? { rpcUrl: RPC } : { network: NET }),
  });
  delete sessions[name];
  saveSessions(sessions);
  console.log(`✅ revoked '${name}'`);
  process.exit(0);
}

die(`unknown command '${cmd}' — try create | call | volley | topup | status | list | revoke`);
