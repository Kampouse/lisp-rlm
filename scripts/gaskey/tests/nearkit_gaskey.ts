
import { Near, InMemoryKeyStore, parseKey, generateKey } from "near-kit";
import { readFileSync } from "node:fs";
const sess = JSON.parse(readFileSync(process.env.HOME + "/.gaskey/sessions.json", "utf8")).trade;
const OWNER = sess.owner, CONTRACT = sess.contract;
const TOKEN = "gktrial-2.lpad.nostrgove2e.testnet";
const unwrap = (v: any) => v && typeof v === "object" && typeof v.result === "string" ? JSON.parse(v.result) : v;
const cred = JSON.parse(readFileSync(process.env.HOME + "/.near-credentials/testnet/" + OWNER + ".json", "utf8"));
const owner = new Near({ network: "testnet", keyStore: new InMemoryKeyStore() });
await (owner as any).keyStore.add(OWNER, parseKey(cred.secret_key || cred.private_key));
const gk = generateKey();
await owner.transaction(OWNER)
  .addKey(String(gk.publicKey), { type: "gasKeyFunctionCall", numNonces: 4, receiverId: CONTRACT, methodNames: ["buy", "sell_internal"] })
  .transferToGasKey(String(gk.publicKey), "0.3 NEAR")
  .send();
const gasKit = new Near({ network: "testnet", keyStore: new InMemoryKeyStore() });
await (gasKit as any).keyStore.add(OWNER, parseKey(String(gk.secretKey)));
const t2: any = await gasKit.transaction(OWNER)
  .useGasKey(2)
  .functionCall(CONTRACT, "buy", { token: TOKEN, near_in: "20000000000000000000000" }, { gas: "120 Tgas" })
  .send();
console.log("hash:", t2.transaction?.hash);
console.log("logs:", JSON.stringify(t2.receipts_outcome?.flatMap((r: any) => r.outcome.logs)));
console.log("statuses:", JSON.stringify((t2.receipts_outcome||[]).map((r:any)=>Object.keys(r.outcome.status||{}))));
await new Promise(r => setTimeout(r, 1500));
const padPost = unwrap(await gasKit.view(CONTRACT, "get_balance", { account: OWNER, token: TOKEN }));
console.log("pad after:", padPost.near, padPost.tokens);
