// Double-lock proof: a gas key that IS whitelisted for withdraw() must still
// fail — restricted keys can never attach the 1-yoctoNEAR gate.
import { readFileSync } from "node:fs";
import { actions, queryAccessKey, sendTx } from "@fastnear/api";
import { privateKeyFromRandom, signerFromPrivateKey } from "@fastnear/utils";

const NET = "testnet";
const OWNER = "nostrgove2e.testnet";
const POOL = "nostrgove2e.testnet";
const LANE = 0;
const cred = JSON.parse(readFileSync(`${process.env.HOME}/.near-credentials/testnet/${OWNER}.json`, "utf8"));
const ownerSigner = signerFromPrivateKey(cred.private_key);

// key WITH withdraw whitelisted (simulates a misconfiguration)
const wk = privateKeyFromRandom("ed25519");
const wSigner = signerFromPrivateKey(wk);
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: OWNER,
  actions: [
    actions.addLimitedAccessGasKey({ publicKey: wSigner.publicKey, numNonces: 2,
      accountId: POOL, methodNames: ["buy", "sell_internal", "withdraw"] }),
    actions.transferToGasKey({ publicKey: wSigner.publicKey, deposit: "0.2 NEAR" }),
  ],
  waitUntil: "FINAL", network: NET,
});
const perm = (await queryAccessKey({ accountId: OWNER, publicKey: wSigner.publicKey, blockId: "final", network: NET })).result.permission;
console.log("withdraw-capable gas key:", JSON.stringify(perm).slice(0, 220));

let body;
try {
  const r = await sendTx({
    signerId: OWNER, signer: wSigner, receiverId: POOL, nonceIndex: LANE,
    actions: [actions.functionCall({ methodName: "withdraw", args: {}, deposit: "0", gas: "60 Tgas" })],
    waitUntil: "FINAL", network: NET,
  });
  body = JSON.stringify(r);
} catch (e) {
  body = "THREW: " + String(e).slice(0, 300);
}
const yocto = body.includes("ERR_YOCTO");
const padAfter = await (await fetch("https://rpc.testnet.near.org", { method: "POST", headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ jsonrpc: "2.0", id: "v", method: "query",
    params: { request_type: "call_function", account_id: POOL, method_name: "get_balance",
      args_base64: Buffer.from(JSON.stringify({ account: OWNER })).toString("base64"), finality: "final" } }) }).then(r => r.json()));
const padStr = Buffer.from(padAfter.result.result).toString();
console.log("contract-level ERR_YOCTO observed:", yocto);
console.log("pad after attempted withdraw:", padStr);
if (!yocto) throw new Error("expected ERR_YOCTO on the withdraw receipt; got: " + body.slice(0, 300));
console.log("\n=== DOUBLE LOCK PROVEN: whitelisted gas key still cannot withdraw ===");
