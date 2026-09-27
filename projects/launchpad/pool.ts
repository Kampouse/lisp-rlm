// pool.ts — one-sided xyk pool (pump-style) for lisp-rlm launchpad tokens.
//
// One pool per token, keyed by the token's account id. The launchpad seeds
// it: `seed_pool` (attached NEAR = starting NEAR reserve) then pushes the
// token's full supply in via ft_transfer_call(msg="seed"). After that:
//   buy  — @payable, NEAR in → tokens out (ftTransfer to buyer)
//   sell — token.ft_transfer_call(receiver=pool, msg="sell") → NEAR out
//
// xyk needs (reserve_a * amount) / (reserve_b + amount) — the numerator is
// ~170 bits and overflows u128, so bigMul/bigDiv do exact base-10 schoolbook
// on decimal strings (digit ops in i64, running remainder in u128).
// Bounds: reserves ≤ 1e30 yocto → remainder*10 ≤ 1e31, far under u128 max.

function pnKey(t: string): string { return "pn:" + t; }
function ptKey(t: string): string { return "pt:" + t; }
function tokKey(t: string): string { return "tk:" + t; }

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

// ── pool lifecycle ─────────────────────────────────────────────────

// init: owner = whoever deploys/initializes (the launchpad factory)
export function new_(): number {
  if ((near.storageGet("ok") ?? "") != "") { near.abort("ERR_INIT"); return 0; }
  near.storageSet("ok", "1");
  near.storageSet("owner", near.predecessorAccountId());
  near.log(`pool_init:${near.predecessorAccountId()}`);
  return 0;
}

// launchpad opens a pool: attached deposit = starting NEAR reserve
export function seed_pool(): number {
  const token = near.jsonGetStr("token") ?? "";
  const attached = near.attachedDepositU128();
  if (strLength(token) == 0) { near.abort("ERR_TOKEN"); return 0; }
  if (u128IsZero(attached)) { near.abort("ERR_SEED"); return 0; }
  if ((near.storageGet(pnKey(token)) ?? "") != "") { near.abort("ERR_EXISTS"); return 0; }
  near.storageSet(pnKey(token), attached);
  near.storageSet(ptKey(token), "0");
  // the token CONTRACT id — the predecessor here is the launchpad (the
  // caller), not the token; buy() needs the contract to call ftTransfer on
  near.storageSet(tokKey(token), token);
  near.log(`seeded:${token}:${attached}`);
  return 0;
}

// NEP-141 receiver — the only way tokens enter the pool.
// msg="sell"  → anyone: tokens in, exact xyk NEAR out (detached transfer)
// other msg   → tokens join the reserve (factory seed or plain donation)
// Returns "0" = used the full amount (NEP-141 U128 refund convention).
export function ft_on_transfer(): string {
  const token = near.predecessorAccountId();
  const sender = near.jsonGetStr("sender_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const msg = near.jsonGetStr("msg") ?? "";
  const pt = near.storageGet(ptKey(token)) ?? "";
  if (pt == "") { near.abort("ERR_NO_POOL"); return "0"; }
  const pn = near.storageGet(pnKey(token)) ?? "0";
  if (msg == "sell") {
    if (u128Gt(amount, pt)) { near.abort("ERR_LIQUIDITY"); return "0"; }
    // WORKAROUND (compiler bug, see GAPS.md 2026-09-27): a user-call result
    // passed DIRECTLY as a call argument can lose its last char depending on
    // module layout — bind intermediates to locals first (verified exact).
    const num = bigMul(pn, amount);
    const den = u128Add(pt, amount);
    const nearOut = bigDiv(num, den);
    if (u128IsZero(nearOut)) { near.abort("ERR_DUST"); return "0"; }
    near.storageSet(ptKey(token), u128Add(pt, amount));
    near.storageSet(pnKey(token), u128Sub(pn, nearOut));
    near.transferU128(sender, nearOut);
    near.log(`sell:${sender}:${amount}:${nearOut}`);
  } else {
    // seed / donation: tokens join the reserve (sender gets nothing back —
    // only "sell" pays out, so griefing is limited to donating one's own
    // tokens)
    near.storageSet(ptKey(token), u128Add(pt, amount));
    near.log(`tokens_in:${amount}`);
  }
  return "0";
}

// buy: @payable — NEAR in → tokens out to the buyer (predecessor)
export function buy(): number {
  const token = near.jsonGetStr("token") ?? "";
  const pn = near.storageGet(pnKey(token)) ?? "";
  if (pn == "") { near.abort("ERR_NO_POOL"); return 0; }
  const attached = near.attachedDepositU128();
  if (u128IsZero(attached)) { near.abort("ERR_ZERO"); return 0; }
  const pt = near.storageGet(ptKey(token)) ?? "0";
  if (u128IsZero(pt)) { near.abort("ERR_NO_TOKENS"); return 0; }
  const num = bigMul(pt, attached);
  const den = u128Add(pn, attached);
  const out = bigDiv(num, den);
  if (u128IsZero(out)) { near.abort("ERR_DUST"); return 0; }
  near.storageSet(ptKey(token), u128Sub(pt, out));
  near.storageSet(pnKey(token), u128Add(pn, attached));
  near.call(near.storageGet(tokKey(token)) ?? "", "ftTransfer",
    `{"receiver_id":${jsonQuote(near.predecessorAccountId())},"amount":${jsonQuote(out)}}`,
    30000000000000, "0");
  near.log(`buy:${near.predecessorAccountId()}:${attached}:${out}`);
  return 0;
}

// ── views ──────────────────────────────────────────────────────────
export function get_pool(): string {
  const token = near.jsonGetStr("token") ?? "";
  const pn = near.storageGet(pnKey(token)) ?? "";
  if (pn == "") { return "{}"; }
  return `{"near":${jsonQuote(pn)},"tokens":${jsonQuote(near.storageGet(ptKey(token)) ?? "0")}}`;
}
export function quote_buy(): string {
  const token = near.jsonGetStr("token") ?? "";
  const nearIn = near.jsonGetStr("near_in") ?? "0";
  const pn = near.storageGet(pnKey(token)) ?? "";
  // Mirror buy(): a missing pool is an error, not a silent 0 quote
  // (frontend footgun found in live QA 2026-09-27).
  if (pn == "") { near.abort("ERR_NO_POOL"); return "0"; }
  if (u128IsZero(nearIn)) { near.abort("ERR_ZERO"); return "0"; }
  const ptv = near.storageGet(ptKey(token)) ?? "0";
  const num = bigMul(ptv, nearIn);
  const den = u128Add(pn, nearIn);
  return bigDiv(num, den);
}