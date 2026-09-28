// LIVE E2E: gas key spends the pool's internal balance (2026-09-28).
// Pool: nostrgove2e.testnet (v3), token: tk.nostrgove2e.testnet
// deposit(full key) -> add scoped gas key -> fund key -> gas-key buy(attach 0)
// -> sell_internal -> pad grew -> gas-key deposit() DENIED.
import { readFileSync } from "node:fs";
import { actions, queryAccessKey, queryGasKeyNonces, sendTx } from "@fastnear/api";
import { privateKeyFromRandom, signerFromPrivateKey } from "@fastnear/utils";

const NET = "testnet";
const RPC = "https://rpc.testnet.near.org";
const OWNER = "nostrgove2e.testnet";
const POOL = "nostrgove2e.testnet";
const TOKEN = "tk.nostrgove2e.testnet";
const LANE = 2;

const cred = JSON.parse(readFileSync(`${process.env.HOME}/.near-credentials/testnet/${OWNER}.json`, "utf8"));
const ownerSigner = signerFromPrivateKey(cred.private_key);
const log = (...a) => console.log(...a);

const view = async (method, args) => {
  const r = await fetch(RPC, { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: "v", method: "query",
      params: { request_type: "call_function", account_id: POOL, method_name: method,
        args_base64: Buffer.from(JSON.stringify(args)).toString("base64"), finality: "final" } }) }).then(r => r.json());
  if (r.error) throw new Error(method + ": " + JSON.stringify(r.error).slice(0, 200));
  const body = Buffer.from(r.result.result).toString();
  let parsed;
  try { parsed = JSON.parse(body); } catch { parsed = body; }
  if (typeof parsed === "string") {
    try { parsed = JSON.parse(parsed); } catch { /* leave as string */ }
  }
  if (parsed && typeof parsed === "object" && typeof parsed.result === "string") {
    try { parsed = JSON.parse(parsed.result); } catch { /* keep */ }
  }
  return parsed;
};

// ── phase 1: deposit 2 NEAR into the pad (FULL key) — delta-based ──
const pad0 = await view("get_balance", { account: OWNER });
log("pad before deposit:", pad0.near);
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: POOL,
  actions: [actions.functionCall({ methodName: "deposit", args: {}, deposit: "2 NEAR", gas: "60 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad1 = await view("get_balance", { account: OWNER });
log("pad after deposit:", pad1.near);
if (BigInt(pad1.near) !== BigInt(pad0.near) + 2000000000000000000000000n)
  throw new Error("deposit did not land exactly +2 NEAR");

// ── phase 2: scoped gas key (receiver=pool, buy/sell_internal only) + fund 0.5N ──
const gasPriv = privateKeyFromRandom("ed25519");
const gasSigner = signerFromPrivateKey(gasPriv);
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: OWNER,
  actions: [
    actions.addLimitedAccessGasKey({ publicKey: gasSigner.publicKey, numNonces: 4,
      accountId: POOL, methodNames: ["buy", "sell_internal"] }),
    actions.transferToGasKey({ publicKey: gasSigner.publicKey, deposit: "0.5 NEAR" }),
  ],
  waitUntil: "FINAL", network: NET,
});
const perm = (await queryAccessKey({ accountId: OWNER, publicKey: gasSigner.publicKey, blockId: "final", network: NET })).result.permission;
log("gas key permission:", JSON.stringify(perm).slice(0, 260));

const gkNonce = async () => {
  const q = await queryGasKeyNonces({ accountId: OWNER, publicKey: gasSigner.publicKey, blockId: "final", network: NET });
  const arr = q.result ? q.result.nonces : q.nonces;
  const e = (arr || []).find((x) => x.nonce_index === LANE);
  return (e ? e.nonce : 0) + 1;
};

// ── phase 3: gas-key BUY, attach = 0 ──
await sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "buy",
    args: { token: TOKEN, near_in: "1000000000000000000000000" }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad2 = await view("get_balance", { account: OWNER, token: TOKEN });
log("after gas buy:", pad2);
if (BigInt(pad2.near) >= BigInt(pad1.near)) throw new Error("pad was not debited");
if (pad2.tokens === "0") throw new Error("internal token ledger not credited");

// ── phase 4: gas-key SELL_INTERNAL, attach = 0 ──
await sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "sell_internal",
    args: { token: TOKEN, tokens_in: pad2.tokens }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad3 = await view("get_balance", { account: OWNER, token: TOKEN });
log("after sell:", pad3);
if (BigInt(pad3.near) <= BigInt(pad2.near)) throw new Error("pad did not grow from sell");
if (pad3.tokens !== "0") throw new Error("ledger not emptied");

// ── phase 5: negative — gas key cannot call deposit() (not whitelisted) ──
let denied = false;
try {
  await sendTx({
    signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
    actions: [actions.functionCall({ methodName: "deposit", args: {}, deposit: "0", gas: "60 Tgas" })],
    waitUntil: "FINAL", network: NET,
  });
} catch (e) {
  denied = true;
  log("gas-key deposit rejected:", String(e).slice(0, 140));
}
if (!denied) throw new Error("SECURITY HOLE: gas key called deposit()");

log("\n=== LIVE E2E VERDICT: gas key bought + sold via internal balances on mainline testnet; deposit denied; pad reconciled ===");
