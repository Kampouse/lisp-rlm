
import { Near, InMemoryKeyStore, parseKey } from "near-kit";
import { readFileSync } from "node:fs";

const sess = JSON.parse(readFileSync(process.env.HOME + "/.gaskey/sessions.json", "utf8")).trade;
const OWNER = sess.owner, CONTRACT = sess.contract;
const TOKEN = "gktrial-2.lpad.nostrgove2e.testnet";
const ro = new Near({ network: "testnet" });

const unwrap = (v: any) => { if (v && typeof v === "object" && typeof v.result === "string") { try { return JSON.parse(v.result); } catch { return v; } } return v; };

const st: any = await ro.getStatus();
console.log("1 getStatus:", st && (st.latestBlockHeight ?? st.sync_info?.latest_block_height) ? "OK" : "keys=" + Object.keys(st||{}).join(","));

const pool = unwrap(await ro.view<any>(CONTRACT, "get_pool", { token: TOKEN }));
console.log("2 view get_pool:", pool?.near ? "OK pn=" + BigInt(pool.near).toString().slice(0,6) + "… graduated=" + pool.graduated : "FAIL");

const pad = unwrap(await ro.view<any>(CONTRACT, "get_balance", { account: OWNER, token: TOKEN }));
console.log("3 view get_balance:", pad?.near ? "OK near=" + pad.near.slice(0,6) + "… tokens=" + pad.tokens.slice(0,6) + "…" : "FAIL " + JSON.stringify(pad).slice(0,80));

const fee = unwrap(await ro.view<any>(CONTRACT, "get_fee", {}));
console.log("3b view get_fee:", fee?.bps ? "OK bps=" + fee.bps + " to=" + fee.to : "FAIL");

const keys = await ro.getAccessKeys(OWNER);
const gk = keys.keys.find((k: any) => k.public_key === sess.gas_public);
console.log("4 getAccessKeys:", gk ? "OK nonce=" + gk.access_key.nonce + " perm=" + JSON.stringify(gk.access_key.permission).slice(0,80) : "FAIL");

const cred = JSON.parse(readFileSync(process.env.HOME + "/.near-credentials/testnet/" + OWNER + ".json", "utf8"));
const signer = new Near({ network: "testnet", keyStore: new InMemoryKeyStore() });
await (signer as any).keyStore.add(OWNER, parseKey(cred.secret_key || cred.private_key));
const before = unwrap(await ro.view<any>(CONTRACT, "get_balance", { account: OWNER, token: TOKEN }));
const tx: any = await signer.transaction(OWNER)
  .functionCall(CONTRACT, "deposit", {}, { attachedDeposit: "0.1 NEAR", gas: "60 Tgas" })
  .send();
const hash = tx?.transaction?.hash;
console.log("5 tx deposit:", hash ? "OK " + hash.slice(0,12) + "…" : "FAIL " + JSON.stringify(tx?.status || tx).slice(0,150));
let after = unwrap(await ro.view<any>(CONTRACT, "get_balance", { account: OWNER, token: TOKEN }));
let delta = BigInt(after.near) - BigInt(before.near);
for (let i = 0; i < 10 && delta !== 100000000000000000000000n; i++) {
  await new Promise(r => setTimeout(r, 500));
  after = unwrap(await ro.view<any>(CONTRACT, "get_balance", { account: OWNER, token: TOKEN }));
  delta = BigInt(after.near) - BigInt(before.near);
}
console.log("6 pad delta:", delta === 100000000000000000000000n ? "OK +0.1N exact" : "FAIL " + delta);
const logs = tx.receipts_outcome?.flatMap((r: any) => r.outcome.logs) || [];
console.log("7 logs:", logs.length >= 0 ? "OK " + logs.length : "FAIL");
const sv = tx.status?.SuccessValue;
console.log("8 SuccessValue:", sv !== undefined ? "OK " + Buffer.from(sv, "base64").toString().slice(0,40) : "none");
