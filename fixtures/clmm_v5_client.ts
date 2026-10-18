// ─── clmm_v5_client.ts — plumbing: live chain ⇄ TS twin ───
//
// 1. fetch live get_state → Pool.fromState (the twin runs on REAL state)
// 2. live parity check: twin.quoteSwap vs the on-chain quote verb —
//    byte-equal or abort before any money moves
// 3. derive mo from the slippage % (mo = out × (1 − tol), floored)
// 4. execute ft_transfer_call via the near CLI (keys from ~/.near-credentials)
// 5. parse the resolve receipt's used:<kept>, cross-check against the preview
// 6. custody pin: 6|ESC == ft_balance_of for both tokens
//
// Usage:
//   tsx clmm_v5_client.ts --tok y --amt 1e21 --slippage 0.005           (dry run)
//   tsx clmm_v5_client.ts ... --execute                                  (swap)
import { Pool } from "./clmm_v5.ts";
import * as https from "https";
import { execFileSync } from "child_process";

const RPC = "https://rpc.testnet.near.org";
const POOL = "pool3.clmm5-51fe5a.testnet";
const TRADER = "clmm5-51fe5a.testnet";
const TOK = { x: "tokx.clmm5-51fe5a.testnet", y: "toky.clmm5-51fe5a.testnet" };
const GAS = "350000000000000"; // ≥285T per ft leg (250T attach + resolve)

function rpc(method: string, params: unknown): Promise<any> {
  const body = JSON.stringify({ jsonrpc: "2.0", id: "c", method, params });
  return new Promise((resolve, reject) => {
    const req = https.request(RPC, { method: "POST", headers: { "Content-Type": "application/json" } },
      res => {
        let d = "";
        res.on("data", c => (d += c));
        res.on("end", () => {
          const j = JSON.parse(d);
          j.error ? reject(new Error(JSON.stringify(j.error))) : resolve(j.result);
        });
      });
    req.on("error", reject);
    req.write(body);
    req.end();
  });
}

async function view(contract: string, method: string, args: unknown): Promise<string> {
  const res = await rpc("query", {
    request_type: "call_function", finality: "final",
    account_id: contract, method_name: method,
    args_base64: Buffer.from(JSON.stringify(args)).toString("base64"),
  });
  return (res.result as number[]).map(c => String.fromCharCode(c)).join("");
}

async function balanceOf(token: string, account: string): Promise<bigint> {
  return BigInt((await view(token, "ft_balance_of", { account_id: account })).replace(/"/g, ""));
}

function nearCall(contract: string, method: string, args: unknown): string {
  // signing via near CLI — keys live in ~/.near-credentials, never here
  return execFileSync("near",
    ["call", contract, method, JSON.stringify(args),
     "--accountId", TRADER, "--gas", GAS, "--depositYocto", "1"],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

function parseUsed(cliOut: string): string | null {
  // near CLI prints receipt tree + final result — the resolve's used: is
  // the LAST occurrence in the output
  const ms = cliOut.match(/used:[0-9]+/g);
  return ms ? ms[ms.length - 1] : null;
}

async function main(): Promise<void> {
  const arg = (n: string, d?: string): string => {
    const i = process.argv.indexOf("--" + n);
    return i > 0 ? process.argv[i + 1] : d!;
  };
  const execute = process.argv.includes("--execute");
  const dir = (arg("tok", "y") as "x" | "y");
  const amt = BigInt(arg("amt", "1000000000000000000000"));
  const tolPct = arg("slippage", "0.005");
  const tolNum = BigInt(Math.round(Number(tolPct) * 1000));
  const tol = tolNum > 1000n ? 1000n : tolNum; // cap at 100%

  // 1. live state → twin
  const stateStr = await view(POOL, "get_state", {});
  const pool = Pool.fromState(stateStr);
  const live = Pool.parseState(stateStr);
  console.log(`pool  : P=${live.P} S=${live.S} range=[${live.imin},${live.imax}]`);
  console.log(`last  : ${live.last}`);

  // 2. LIVE PARITY: twin quote vs on-chain quote verb (pre-skip the fee)
  const fee = amt >> BigInt(live.shift);
  const v = amt - fee;
  const prev = pool.quoteSwap(amt.toString(), dir);
  const chainQ = await view(POOL, "quote", { v: (dir === "x" ? v.toString() : "-" + v.toString()) });
  const chainOut = chainQ.replace(/"/g, "").split(":")[1];
  if (chainOut !== prev.out) {
    throw new Error(`LIVE PARITY FAIL: twin out=${prev.out} chain out=${chainOut} — do not trade`);
  }
  console.log(`quote : out=${prev.out} (twin==chain ✓) kept=${prev.kept} fee=${prev.fee}`);

  if (prev.out === "0") {
    console.log(`range ${dir === "x" ? "exhausted up" : "exhausted down"}: swap would refund-all — aborting`);
    return;
  }

  // 3. mo from tolerance: mo = out × (1000 − tol) / 1000, floor (BigInt div)
  const mo = (BigInt(prev.out) * (1000n - tol)) / 1000n;
  console.log(`guard : mo=${mo} (tolerance ${(Number(tol) / 10).toFixed(1)}%)`);

  if (!execute) {
    console.log("dry run — pass --execute to send");
    return;
  }

  // 4. execute (guarded)
  const escBefore = { x: live.escx, y: live.escy };
  const balBefore = await balanceOf(TOK[dir], TRADER);
  const msg = JSON.stringify({ a: "swap", mo: mo.toString() });
  let cli: string;
  try {
    cli = nearCall(TOK[dir], "ft_transfer_call",
      { receiver_id: POOL, amount: amt.toString(), msg });
  } catch (e) {
    console.error("tx failed:", (e as Error).message.slice(-400));
    process.exit(1);
  }
  const used = parseUsed(cli);
  if (!used) throw new Error("no used: in resolve result — inspect manually");
  console.log(`result: ${used}`);

  // 5. used = CONSUMED slice: used:amt = full fill (rem=0), used:0 = full
  // revert (ERR_MO / degenerate / sentinel)
  const res = Pool.parseSwapResult(amt.toString(), used);
  const kept = BigInt(res.kept);
  const outBefore = await balanceOf(TOK[dir === "x" ? "y" : "x"], TRADER);
  if (kept === 0n) {
    console.log("FULL REVERT (ERR_MO or degenerate) — nothing consumed, fee returned");
  } else {
    console.log(`filled: kept=${res.kept} refunded=${res.refunded}`);
  }

  // 6. custody pin — live
  const st2raw = await view(POOL, "get_state", {});
  const escx = st2raw.match(/ESCX:([0-9]+)/)![1];
  const escy = st2raw.match(/ESCY:([0-9]+)/)![1];
  const bx = await balanceOf(TOK.x, POOL);
  const by = await balanceOf(TOK.y, POOL);
  const okx = BigInt(escx) === bx, oky = BigInt(escy) === by;
  console.log(`custody: ESCX==tokx ${okx}  ESCY==toky ${oky}`);
  if (!okx || !oky) throw new Error("CUSTODY LAW VIOLATION");
  const balAfter = await balanceOf(TOK[dir], TRADER);
  const spent = balBefore - balAfter;
  console.log(`trader ${dir}: before=${balBefore} after=${balAfter} net=${spent}`);
  if (spent !== kept) throw new Error(`net spent ${spent} != used ${kept}`);
  if (kept > 0n) {
    const outAfter = await balanceOf(TOK[dir === "x" ? "y" : "x"], TRADER);
    console.log(`trader received out-token: +${outAfter - outBefore}`);
  }
  void escBefore;
}
void main();
