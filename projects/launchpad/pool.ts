// pool.ts — one-sided xyk pool (pump-style) for lisp-rlm launchpad tokens.
//
// One pool per token, keyed by the token's account id. The launchpad seeds
// it: `seed_pool` (attached NEAR = starting NEAR reserve) then pushes the
// token's full supply in via ft_transfer_call(msg="seed"). After that:
//   buy  — @payable, NEAR in → tokens out (ftTransfer to buyer)
//   sell — token.ft_transfer_call(receiver=pool, msg="sell"[:min_near_out])
//          → NEAR out (net of fee, guarded by optional min)
//
// v2 (2026-09-27, prod hardening):
//   - slippage guards: buy{min_tokens_out}, sell msg "sell:<min_near_out>"
//   - trading fee: fee_bps (set_fee, cap 1000 = 10%), taken on OUTPUT
//     (buy: tokens; sell: NEAR), accrued to per-token ledgers, claimable
//     by fee_to via claim_fees(token)
//   - pause: set_paused halts buy/sell (views + claims keep working)
//   - graduation: grad_th set at seed; crossing it emits EVENT_JSON
//     pool_graduated; migrate() (owner) then pays the NEAR reserve to the
//     configured ref:acct and halts trading for the token
//   - trade events: EVENT_JSON standard intear_launch event "trade"
//   - buy()/quote_buy()/quote_sell() return JSON: {"gross","fee","net"}
//
// xyk needs (reserve_a * amount) / (reserve_b + amount) — the numerator is
// ~170 bits and overflows u128, so bigMul/bigDiv do exact base-10 schoolbook
// on decimal strings (digit ops in i64, running remainder in u128).
// Bounds: reserves ≤ 1e30 yocto → remainder*10 ≤ 1e31, far under u128 max.

function pnKey(t: string): string { return "pn:" + t; }
function ptKey(t: string): string { return "pt:" + t; }
function tokKey(t: string): string { return "tk:" + t; }
function feeTokKey(t: string): string { return "fees_tok:" + t; }
function gradThKey(t: string): string { return "grad_th:" + t; }
function gradKey(t: string): string { return "grad:" + t; }
function migKey(t: string): string { return "migrated:" + t; }

function isOwner(): number {
  if (near.predecessorAccountId() == (near.storageGet("owner") ?? "")) { return 1; }
  return 0;
}

// fee on an output amount: floor(amount * bps / 10000), bps ≤ 1000
function feeOn(amount: string): string {
  const bps = near.storageGet("fee_bps") ?? "0";
  if (strToNum(bps) == 0) { return "0"; }
  return u128Div(u128Mul(amount, bps), "10000");
}

// ── exact decimal big-num helpers (base 10 schoolbook) ─────────────

function trimZeros(s: string): string {
  let i = 0;
  while (i < strLength(s) - 1) {
    if (strSlice(s, i, i + 1) == "0") { i = i + 1; } else { break; }
  }
  return strSlice(s, i, strLength(s));
}

// decimal-string × single digit (0..9) — exact, i64-safe (9*9+carry ≤ 90)
function mulDigit(x: string, d: number): string {
  let out = "";
  let carry = 0;
  let i = strLength(x);
  while (i > 0) {
    i = i - 1;
    const acc = strToNum(strSlice(x, i, i + 1)) * d + carry;
    out = toStr(acc % 10) + out;
    carry = acc / 10;
  }
  while (carry > 0) {
    out = toStr(carry % 10) + out;
    carry = carry / 10;
  }
  return out;
}

// decimal-string + decimal-string (schoolbook, both non-negative)
function addStr(x: string, y: string): string {
  let out = "";
  let carry = 0;
  let i = strLength(x);
  let j = strLength(y);
  while (i > 0 || j > 0) {
    let acc = carry;
    if (i > 0) { i = i - 1; acc = acc + strToNum(strSlice(x, i, i + 1)); }
    if (j > 0) { j = j - 1; acc = acc + strToNum(strSlice(y, j, j + 1)); }
    out = toStr(acc % 10) + out;
    carry = acc / 10;
  }
  if (carry > 0) { out = toStr(carry) + out; }
  return out;
}

// exact product of two u128-scale decimal strings (may exceed u128)
function bigMul(x: string, y: string): string {
  let out = "0";
  let k = strLength(y);
  let shift = "";
  while (k > 0) {
    k = k - 1;
    const d = strToNum(strSlice(y, k, k + 1));
    if (d != 0) {
      const p = mulDigit(x, d) + shift;
      out = addStr(out, p);
    }
    shift = shift + "0";
  }
  return trimZeros(out);
}

// exact floor division: big decimal ÷ u128-scale decimal (long division
// base 10; remainder < y ≤ ~1e30 in our bounds → rem*10+d ≤ 1e31, u128-safe)
function bigDiv(x: string, y: string): string {
  let out = "";
  let rem = "0";
  let i = 0;
  const n = strLength(x);
  while (i < n) {
    // flat statements, no nested u128 chains — nested u128 calls inside
    // loop-carried operand positions hit the limb-scratch clobber (trap in
    // __h_u128_parse); splitting into per-step locals avoids it entirely
    const r10 = u128Mul(rem, "10");
    const cur = u128Add(r10, strSlice(x, i, i + 1));
    const q = u128Div(cur, y);
    rem = u128Mod(cur, y);
    out = out + q;
    i = i + 1;
  }
  if (strLength(out) == 0) { return "0"; }
  return trimZeros(out);
}

// xyk output for `inAmt` added to reserve `rIn`, taking from reserve `rOut`
function xykOut(rOut: string, rIn: string, inAmt: string): string {
  const num = bigMul(rOut, inAmt);
  const den = u128Add(rIn, inAmt);
  return bigDiv(num, den);
}

// standard trade event (intear_launch standard, event "trade")
function tradeEvent(token: string, side: string, trader: string,
                    nearAmt: string, tokAmt: string, fee: string): number {
  const ev = jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(jsonSet(`{}`, "token_account_id", jsonQuote(token)),
            "side", jsonQuote(side)),
          "trader", jsonQuote(trader)),
        "near", jsonQuote(nearAmt)),
      "tokens", jsonQuote(tokAmt)),
    "fee", jsonQuote(fee));
  near.log("EVENT_JSON:" + `{"standard":"intear_launch","version":"1.0.0","event":"trade","data":${ev}}`);
  return 0;
}

// ── pool lifecycle ─────────────────────────────────────────────────

// init: owner = whoever deploys/initializes (the launchpad factory)
export function new_(): number {
  if ((near.storageGet("ok") ?? "") != "") { near.abort("ERR_INIT"); return 0; }
  near.storageSet("ok", "1");
  near.storageSet("owner", near.predecessorAccountId());
  near.storageSet("fee_bps", "100");            // 1% default
  near.storageSet("fees_near", "0");
  near.log(`pool_init:${near.predecessorAccountId()}`);
  return 0;
}

// launchpad opens a pool: attached deposit = starting NEAR reserve
// args: { token, grad_th? }  (grad_th in yoctoNEAR; "0" = never graduates)
export function seed_pool(): number {
  const token = near.jsonGetStr("token") ?? "";
  const attached = near.attachedDepositU128();
  if (strLength(token) == 0) { near.abort("ERR_TOKEN"); return 0; }
  if (u128IsZero(attached)) { near.abort("ERR_SEED"); return 0; }
  if ((near.storageGet(pnKey(token)) ?? "") != "") { near.abort("ERR_EXISTS"); return 0; }
  // only the launchpad (factory = owner) may seed — an open seed would let
  // anyone front-run the factory with a dust reserve and own the curve
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  near.storageSet(pnKey(token), attached);
  near.storageSet(ptKey(token), "0");
  // the token CONTRACT id — the predecessor here is the launchpad (the
  // caller), not the token; buy() needs the contract to call ftTransfer on
  near.storageSet(tokKey(token), token);
  near.storageSet(gradThKey(token), near.jsonGetStr("grad_th") ?? "0");
  near.log(`seeded:${token}:${attached}`);
  return 0;
}

// ── owner controls ─────────────────────────────────────────────────

export function set_fee(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const bps = near.jsonGetStr("bps") ?? "";
  const n = strToNum(bps);
  if (n < 0 || n > 1000) { near.abort("ERR_BPS"); return 0; }
  near.storageSet("fee_bps", toStr(n));
  near.log(`fee_set:${n}`);
  return 0;
}

export function set_fee_to(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const acct = near.jsonGetStr("account") ?? "";
  if (strLength(acct) == 0) { near.abort("ERR_ACCT"); return 0; }
  near.storageSet("fee_to", acct);
  near.log(`fee_to_set:${acct}`);
  return 0;
}

export function set_paused(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const on = near.jsonGetStr("on") ?? "0";
  let flag = "0";
  if (on == "1") { flag = "1"; }
  near.storageSet("paused", flag);
  near.log(`paused:${flag}`);
  return 0;
}

export function set_ref_acct(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const acct = near.jsonGetStr("account") ?? "";
  if (strLength(acct) == 0) { near.abort("ERR_ACCT"); return 0; }
  near.storageSet("ref:acct", acct);
  near.log(`ref_acct_set:${acct}`);
  return 0;
}

// two-step ownership handover (no single-tx rug of the fee/migrate keys)
export function transfer_ownership(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const nxt = near.jsonGetStr("new_owner") ?? "";
  if (strLength(nxt) == 0) { near.abort("ERR_ACCT"); return 0; }
  near.storageSet("pending:owner", nxt);
  near.log(`ownership_pending:${nxt}`);
  return 0;
}

export function accept_ownership(): number {
  const pending = near.storageGet("pending:owner") ?? "";
  if (strLength(pending) == 0) { near.abort("ERR_NO_PENDING"); return 0; }
  if (near.predecessorAccountId() != pending) { near.abort("ERR_NOT_PENDING"); return 0; }
  near.storageSet("owner", pending);
  near.storageRemove("pending:owner");
  near.log(`ownership_accepted:${pending}`);
  return 0;
}

// claim accrued fees for one token: NEAR fees + that token's token fees
export function claim_fees(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const token = near.jsonGetStr("token") ?? "";
  const to = near.storageGet("fee_to") ?? (near.storageGet("owner") ?? "");
  const feesTok = near.storageGet(feeTokKey(token)) ?? "0";
  const feesNear = near.storageGet("fees_near") ?? "0";
  if (!u128IsZero(feesNear)) {
    near.storageSet("fees_near", "0");
    near.transferU128(to, feesNear);
  }
  if (!u128IsZero(feesTok)) {
    near.storageSet(feeTokKey(token), "0");
    near.call(near.storageGet(tokKey(token)) ?? "", "ftTransfer",
      `{"receiver_id":${jsonQuote(to)},"amount":${jsonQuote(feesTok)}}`,
      30000000000000, "0");
  }
  near.log(`claimed:${token}:${feesNear}:${feesTok}`);
  return 0;
}

// graduation migration: pay the NEAR reserve to the configured Ref target,
// halt trading for the token (fee ledgers remain claimable)
export function migrate(): number {
  if (isOwner() == 0) { near.abort("ERR_OWNER_ONLY"); return 0; }
  const token = near.jsonGetStr("token") ?? "";
  const pn = near.storageGet(pnKey(token)) ?? "";
  if (pn == "") { near.abort("ERR_NO_POOL"); return 0; }
  if ((near.storageGet(gradKey(token)) ?? "") != "1") { near.abort("ERR_NOT_GRADUATED"); return 0; }
  const ref = near.storageGet("ref:acct") ?? "";
  if (strLength(ref) == 0) { near.abort("ERR_NO_REF"); return 0; }
  near.storageSet(pnKey(token), "0");
  near.storageSet(ptKey(token), "0");
  near.storageSet(migKey(token), "1");
  near.transferU128(ref, pn);
  const ev = jsonSet(
    jsonSet(jsonSet(`{}`, "token_account_id", jsonQuote(token)),
      "near", jsonQuote(pn)),
    "ref", jsonQuote(ref));
  near.log("EVENT_JSON:" + `{"standard":"intear_launch","version":"1.0.0","event":"pool_migrated","data":${ev}}`);
  near.log(`migrated:${token}:${pn}`);
  return 0;
}

// ── trading ────────────────────────────────────────────────────────

function tradingHalt(token: string): number {
  if ((near.storageGet("paused") ?? "") == "1") { near.abort("ERR_PAUSED"); return 1; }
  if ((near.storageGet(migKey(token)) ?? "") == "1") { near.abort("ERR_MIGRATED"); return 1; }
  return 0;
}

// graduated? emits the standard event once (checked after buys — the NEAR
// reserve only grows there)
function checkGrad(token: string, pn: string): number {
  if ((near.storageGet(gradKey(token)) ?? "") == "1") { return 0; }
  const th = near.storageGet(gradThKey(token)) ?? "0";
  if (strToNum(th) == 0) { return 0; }
  if (u128Lt(pn, th)) { return 0; }
  near.storageSet(gradKey(token), "1");
  const ev = jsonSet(
    jsonSet(jsonSet(`{}`, "token_account_id", jsonQuote(token)),
      "near", jsonQuote(pn)),
    "threshold", jsonQuote(th));
  near.log("EVENT_JSON:" + `{"standard":"intear_launch","version":"1.0.0","event":"pool_graduated","data":${ev}}`);
  return 0;
}

// buy: @payable — NEAR in → tokens out to the buyer (predecessor)
// args: { token, min_tokens_out? }  (0/omitted = no slippage guard)
// returns JSON {"gross","fee","net"} (net = what the buyer receives)
export function buy(): string {
  const token = near.jsonGetStr("token") ?? "";
  const pn = near.storageGet(pnKey(token)) ?? "";
  if (pn == "") { near.abort("ERR_NO_POOL"); return ""; }
  if (tradingHalt(token) == 1) { return ""; }
  const attached = near.attachedDepositU128();
  if (u128IsZero(attached)) { near.abort("ERR_ZERO"); return ""; }
  const pt = near.storageGet(ptKey(token)) ?? "0";
  if (u128IsZero(pt)) { near.abort("ERR_NO_TOKENS"); return ""; }
  const gross = xykOut(pt, pn, attached);
  const fee = feeOn(gross);
  const net = u128Sub(gross, fee);
  if (u128IsZero(net)) { near.abort("ERR_DUST"); return ""; }
  const minOut = near.jsonGetStr("min_tokens_out") ?? "0";
  if (!u128IsZero(minOut) && u128Lt(net, minOut)) { near.abort("ERR_SLIPPAGE"); return ""; }
  near.storageSet(ptKey(token), u128Sub(pt, gross));
  near.storageSet(pnKey(token), u128Add(pn, attached));
  if (!u128IsZero(fee)) {
    near.storageSet(feeTokKey(token), u128Add(near.storageGet(feeTokKey(token)) ?? "0", fee));
  }
  near.call(near.storageGet(tokKey(token)) ?? "", "ftTransfer",
    `{"receiver_id":${jsonQuote(near.predecessorAccountId())},"amount":${jsonQuote(net)}}`,
    30000000000000, "0");
  tradeEvent(token, "buy", near.predecessorAccountId(), attached, net, fee);
  near.log(`buy:${near.predecessorAccountId()}:${attached}:${net}`);
  checkGrad(token, u128Add(pn, attached));
  return `{"gross":${jsonQuote(gross)},"fee":${jsonQuote(fee)},"net":${jsonQuote(net)}}`;
}

// NEP-141 receiver — the only way tokens enter the pool.
// msg="sell" or "sell:<min_near_out>" → tokens in, xyk NEAR out net of fee
// other msg   → tokens join the reserve (factory seed or plain donation)
// Returns "0" = used the full amount (NEP-141 U128 refund convention).
// Any abort traps → the token's resolve refunds the full escrow.
export function ft_on_transfer(): string {
  const token = near.predecessorAccountId();
  const sender = near.jsonGetStr("sender_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const msg = near.jsonGetStr("msg") ?? "";
  const pt = near.storageGet(ptKey(token)) ?? "";
  if (pt == "") { near.abort("ERR_NO_POOL"); return "0"; }
  // sell intent? "sell" or "sell:<min>" (anything else = donation/seed)
  const ci = strIndexOf(msg, ":");
  const head = ci >= 0 ? strSlice(msg, 0, ci) : msg;
  if (head == "sell") {
    if (tradingHalt(token) == 1) { return "0"; }
    if (u128Gt(amount, pt)) { near.abort("ERR_LIQUIDITY"); return "0"; }
    const pn = near.storageGet(pnKey(token)) ?? "0";
    if (u128IsZero(pn)) { near.abort("ERR_EMPTY"); return "0"; }
    // WORKAROUND (compiler bug, see GAPS.md 2026-09-27): a user-call result
    // passed DIRECTLY as a call argument can lose its last char depending on
    // module layout — bind intermediates to locals first (verified exact).
    const gross = xykOut(pn, pt, amount);
    const fee = feeOn(gross);
    const net = u128Sub(gross, fee);
    if (u128IsZero(net)) { near.abort("ERR_DUST"); return "0"; }
    const minOut = ci >= 0 ? strSlice(msg, ci + 1, strLength(msg)) : "0";
    if (!u128IsZero(minOut) && u128Lt(net, minOut)) { near.abort("ERR_SLIPPAGE"); return "0"; }
    near.storageSet(ptKey(token), u128Add(pt, amount));
    near.storageSet(pnKey(token), u128Sub(pn, gross));
    if (!u128IsZero(fee)) {
      near.storageSet("fees_near", u128Add(near.storageGet("fees_near") ?? "0", fee));
    }
    near.transferU128(sender, net);
    tradeEvent(token, "sell", sender, net, amount, fee);
    near.log(`sell:${sender}:${amount}:${net}`);
  } else {
    // seed / donation: tokens join the reserve (sender gets nothing back —
    // only "sell" pays out, so griefing is limited to donating one's own
    // tokens)
    near.storageSet(ptKey(token), u128Add(pt, amount));
    near.log(`tokens_in:${amount}`);
  }
  return "0";
}

// ── views ──────────────────────────────────────────────────────────
export function get_pool(): string {
  const token = near.jsonGetStr("token") ?? "";
  const pn = near.storageGet(pnKey(token)) ?? "";
  if (pn == "") { return "{}"; }
  const gradTh = near.storageGet(gradThKey(token)) ?? "0";
  let grad = "false";
  if ((near.storageGet(gradKey(token)) ?? "") == "1") { grad = "true"; }
  let mig = "false";
  if ((near.storageGet(migKey(token)) ?? "") == "1") { mig = "true"; }
  return `{"near":${jsonQuote(pn)},"tokens":${jsonQuote(near.storageGet(ptKey(token)) ?? "0")},"grad_th":${jsonQuote(gradTh)},"graduated":${grad},"migrated":${mig},"fees_tok":${jsonQuote(near.storageGet(feeTokKey(token)) ?? "0")}}`;
}
export function get_fee(): string {
  let paused = "false";
  if ((near.storageGet("paused") ?? "") == "1") { paused = "true"; }
  return `{"bps":${jsonQuote(near.storageGet("fee_bps") ?? "0")},"to":${jsonQuote(near.storageGet("fee_to") ?? (near.storageGet("owner") ?? ""))},"fees_near":${jsonQuote(near.storageGet("fees_near") ?? "0")},"paused":${paused}}`;
}
// quote: NEAR in → tokens out net of fee {"gross","fee","net"}
export function quote_buy(): string {
  const token = near.jsonGetStr("token") ?? "";
  const nearIn = near.jsonGetStr("near_in") ?? "0";
  const pn = near.storageGet(pnKey(token)) ?? "";
  // Mirror buy(): a missing pool is an error, not a silent 0 quote
  // (frontend footgun found in live QA 2026-09-27).
  if (pn == "") { near.abort("ERR_NO_POOL"); return ""; }
  if (u128IsZero(nearIn)) { near.abort("ERR_ZERO"); return ""; }
  const ptv = near.storageGet(ptKey(token)) ?? "0";
  const gross = xykOut(ptv, pn, nearIn);
  const fee = feeOn(gross);
  const net = u128Sub(gross, fee);
  return `{"gross":${jsonQuote(gross)},"fee":${jsonQuote(fee)},"net":${jsonQuote(net)}}`;
}
// quote: tokens in → NEAR out net of fee {"gross","fee","net"}
export function quote_sell(): string {
  const token = near.jsonGetStr("token") ?? "";
  const tokensIn = near.jsonGetStr("tokens_in") ?? "0";
  const pt = near.storageGet(ptKey(token)) ?? "";
  if (pt == "") { near.abort("ERR_NO_POOL"); return ""; }
  if (u128IsZero(tokensIn)) { near.abort("ERR_ZERO"); return ""; }
  const pn = near.storageGet(pnKey(token)) ?? "0";
  if (u128IsZero(pn)) { near.abort("ERR_EMPTY"); return ""; }
  const gross = xykOut(pn, pt, tokensIn);
  const fee = feeOn(gross);
  const net = u128Sub(gross, fee);
  return `{"gross":${jsonQuote(gross)},"fee":${jsonQuote(fee)},"net":${jsonQuote(net)}}`;
}
