// Full product loop on-chain: factory-launched token (gktrial-2) traded
// with a scoped gas key over internal balances — deposit, buy, sell.
import { readFileSync } from "node:fs";
import { actions, queryAccessKey, sendTx } from "@fastnear/api";
import { privateKeyFromRandom, signerFromPrivateKey } from "@fastnear/utils";

const NET = "testnet";
const RPC = "https://rpc.testnet.near.org";
const OWNER = "nostrgove2e.testnet";
const POOL = "pool2.nostrgove2e.testnet";
const TOKEN = "gktrial-2.lpad.nostrgove2e.testnet";
const LANE = 3;

const cred = JSON.parse(readFileSync(`${process.env.HOME}/.near-credentials/testnet/${OWNER}.json`, "utf8"));
const ownerSigner = signerFromPrivateKey(cred.secret_key || cred.private_key);
const log = (...a) => console.log(...a);

const view = async (method, args) => {
  const r = await fetch(RPC, { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: "v", method: "query",
      params: { request_type: "call_function", account_id: POOL, method_name: method,
        args_base64: Buffer.from(JSON.stringify(args)).toString("base64"), finality: "final" } }) }).then(r => r.json());
  let p = JSON.parse(Buffer.from(r.result.result).toString());
  if (typeof p === "string") p = JSON.parse(p);
  if (p && typeof p.result === "string") p = JSON.parse(p.result);
  return p;
};

const pad0 = await view("get_balance", { account: OWNER, token: TOKEN });
log("pad before:", pad0);

// 1. deposit (full key)
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: POOL,
  actions: [actions.functionCall({ methodName: "deposit", args: {}, deposit: "1 NEAR", gas: "60 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad1 = await view("get_balance", { account: OWNER, token: TOKEN });
log("after deposit:", pad1.near);
if (BigInt(pad1.near) !== BigInt(pad0.near) + 1000000000000000000000000n) throw new Error("deposit delta wrong");

// 2. scoped gas key for THIS pool
const gasPriv = privateKeyFromRandom("ed25519");
const gasSigner = signerFromPrivateKey(gasPriv);
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: OWNER,
  actions: [
    actions.addLimitedAccessGasKey({ publicKey: gasSigner.publicKey, numNonces: 8,
      accountId: POOL, methodNames: ["buy", "sell_internal"] }),
    actions.transferToGasKey({ publicKey: gasSigner.publicKey, deposit: "0.3 NEAR" }),
  ],
  waitUntil: "FINAL", network: NET,
});
const perm = (await queryAccessKey({ accountId: OWNER, publicKey: gasSigner.publicKey, blockId: "final", network: NET })).result.permission;
log("gas key:", JSON.stringify(perm).slice(0, 200));

const gkNonce = async () => {
  const q = await queryGasKeyNonces({ accountId: OWNER, publicKey: gasSigner.publicKey, blockId: "final", network: NET });
  const arr = q.result ? q.result.nonces : q.nonces;
  const e = (arr || []).find((x) => x.nonce_index === LANE);
  return (e ? e.nonce : 0) + 1;
};

// 3. gas-key buy on the factory-launched curve (attach = 0)
await sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "buy",
    args: { token: TOKEN, near_in: "500000000000000000000000" }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad2 = await view("get_balance", { account: OWNER, token: TOKEN });
log("after gas-key buy:", pad2);
if (BigInt(pad2.near) !== BigInt(pad1.near) - 500000000000000000000000n) throw new Error("buy debit wrong");
if (BigInt(pad2.tokens) === 0n) throw new Error("no tokens credited");

// 4. gas-key sell back
await sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "sell_internal",
    args: { token: TOKEN, tokens_in: pad2.tokens }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const pad3 = await view("get_balance", { account: OWNER, token: TOKEN });
log("after gas-key sell:", pad3);
if (BigInt(pad3.near) <= BigInt(pad2.near)) throw new Error("pad did not grow");
if (pad3.tokens !== "0") throw new Error("ledger not emptied");

log("\n=== FULL PRODUCT LOOP PROVEN ON-CHAIN: factory launch -> gas-key trade -> round trip ===");
