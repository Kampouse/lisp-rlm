// Latency/throughput bench: gas-key internal trading vs classic NEP-141 path.
// Measures send->FINAL wall time via the same RPC for both, plus gas burnt.
import { readFileSync } from "node:fs";
import { actions, queryAccessKey, sendTx } from "@fastnear/api";
import { privateKeyFromRandom, signerFromPrivateKey } from "@fastnear/utils";

const NET = "testnet";
const OWNER = "nostrgove2e.testnet";
const POOL = "nostrgove2e.testnet";
const TOKEN = "tk.nostrgove2e.testnet";
const BUY = "20000000000000000000000"; // 0.02 NEAR per trade
const LANE = 0;

const cred = JSON.parse(readFileSync(`${process.env.HOME}/.near-credentials/testnet/${OWNER}.json`, "utf8"));
const ownerSigner = signerFromPrivateKey(cred.private_key);
const view = async (method, args) => {
  const r = await fetch("https://rpc.testnet.near.org", { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: "v", method: "query",
      params: { request_type: "call_function", account_id: POOL, method_name: method,
        args_base64: Buffer.from(JSON.stringify(args)).toString("base64"), finality: "final" } }) }).then(r => r.json());
  return JSON.parse(Buffer.from(r.result.result).toString());
};
const balances = async () => {
  let p = await view("get_balance", { account: OWNER });
  if (p && typeof p.result === "string") p = JSON.parse(p.result);
  return p;
};
const pool = async () => {
  let p = await view("get_pool", { token: TOKEN });
  if (p && typeof p.result === "string") p = JSON.parse(p.result);
  return p;
};

// gas key: 8 lanes, buy+sell_internal
const gasPriv = privateKeyFromRandom("ed25519");
const gasSigner = signerFromPrivateKey(gasPriv);
await sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: OWNER,
  actions: [
    actions.addLimitedAccessGasKey({ publicKey: gasSigner.publicKey, numNonces: 8,
      accountId: POOL, methodNames: ["buy", "sell_internal"] }),
    actions.transferToGasKey({ publicKey: gasSigner.publicKey, deposit: "1 NEAR" }),
  ],
  waitUntil: "FINAL", network: NET,
});

const timed = async (fn) => {
  const t0 = Date.now();
  const r = await fn();
  const ms = Date.now() - t0;
  const body = JSON.stringify(r);
  let gas = 0;
  try {
    const o = typeof r === "object" ? r : JSON.parse(body);
    const agg = (x) => (x?.outcome?.gas_burnt ? Number(x.outcome.gas_burnt) : 0);
    gas = agg(o.transaction_outcome) + (o.receipts_outcome || []).reduce((a, x) => a + agg(x), 0);
  } catch {}
  return { ms, gas, ok: !body.includes("Failure") };
};
const internalBuy = () => sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "buy",
    args: { token: TOKEN, near_in: BUY }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const classicBuy = () => sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: POOL,
  actions: [actions.functionCall({ methodName: "buy",
    args: { token: TOKEN }, deposit: "0.02 NEAR", gas: "150 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const ftBal = async () => {
  const r = await fetch("https://rpc.testnet.near.org", { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: "f", method: "query",
      params: { request_type: "call_function", account_id: TOKEN, method_name: "ftBalanceOf",
        args_base64: Buffer.from(JSON.stringify({ account_id: OWNER })).toString("base64"), finality: "final" } }) }).then(r => r.json());
  return JSON.parse(Buffer.from(r.result.result).toString());
};
const classicSell = (amt) => sendTx({
  signerId: OWNER, signer: ownerSigner, receiverId: TOKEN,
  actions: [actions.functionCall({ methodName: "ft_transfer_call",
    args: { receiver_id: POOL, amount: amt, msg: "sell" }, deposit: "1", gas: "150 Tgas" })],
  waitUntil: "FINAL", network: NET,
});
const internalSell = (amt) => sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: LANE,
  actions: [actions.functionCall({ methodName: "sell_internal",
    args: { token: TOKEN, tokens_in: amt }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
});

const med = (a) => a.slice().sort((x, y) => x - y)[Math.floor(a.length / 2)];
const N = 5;

console.log("pool reserves:", JSON.stringify(await pool()));

// ── single-trade latency ──
const cLat = [], iLat = [], cSellLat = [], iSellLat = [];
let cGas = 0, iGas = 0;
const cTokens = [];
for (let k = 0; k < N; k++) {
  const r = await timed(classicBuy); cLat.push(r.ms); cGas += r.gas;
}
let b = await ftBal(); cTokens.push(b);
for (let k = 0; k < N; k++) {
  const r = await timed(internalBuy); iLat.push(r.ms); iGas += r.gas;
}
const ledger = await balances();
for (let k = 0; k < N; k++) {
  const amt = (BigInt(ledger.tokens) / BigInt(N)).toString();
  const r = await timed(() => internalSell(amt)); iSellLat.push(r.ms);
}
for (let k = 0; k < N; k++) {
  const amt = (BigInt(cTokens[0]) / BigInt(N)).toString();
  const r = await timed(() => classicSell(amt)); cSellLat.push(r.ms);
}

// ── throughput: 8 parallel internal vs 8 sequential classic ──
const t0 = Date.now();
await Promise.all(Array.from({ length: 8 }, (_, lane) => sendTx({
  signerId: OWNER, signer: gasSigner, receiverId: POOL, nonceIndex: lane,
  actions: [actions.functionCall({ methodName: "buy",
    args: { token: TOKEN, near_in: BUY }, deposit: "0", gas: "120 Tgas" })],
  waitUntil: "FINAL", network: NET,
})));
const volleyMs = Date.now() - t0;

const t1 = Date.now();
for (let k = 0; k < 8; k++) await classicBuy();
const seqMs = Date.now() - t1;

console.log(JSON.stringify({
  single_latency_ms: {
    classic_buy: { runs: cLat, median: med(cLat) },
    internal_buy: { runs: iLat, median: med(iLat) },
    classic_sell: { runs: cSellLat, median: med(cSellLat) },
    internal_sell: { runs: iSellLat, median: med(iSellLat) },
  },
  gas_burnt_total_tgas: { classic_buy: Math.round(cGas / 1e12), internal_buy: Math.round(iGas / 1e12) },
  throughput_8_trades_ms: { parallel_internal_volley: volleyMs, sequential_classic: seqMs },
  speedup: (seqMs / volleyMs).toFixed(1) + "x",
}, null, 1));
