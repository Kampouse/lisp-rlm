// launchpad.ts — INTEAR-style launch factory (TS dialect), batch 1+2+3:
//   1. launch params: icon, decimals; excess-deposit refund
//   2. launch metadata: socials (validated), edit_token, short_id/long_id,
//      previewId, cost views
//   3. standard events: EVENT_JSON lines (indexers) on launch + edit
//
// Flow (mirrors INTEARnear/launch launch_token):
//   1. @payable: fixed cost + REFUND of the excess (was kept silently)
//   2. short_id → {symbol}.{factory} (collides → abort); long → counter
//   3. registry: immutable li: + mutable socials ls: + creator lb:
//   4. promise chain (detached):
//        create token sub-account (global FT adopt) + init w/ metadata
//        → pool.seed_pool (1 NEAR reserve)
//        → token.ft_transfer_call(full supply, msg=seed)

const LONG_COST: string = "2282500000000000000000000";  // 0.0325 fee + 1.25 funding + 1.0 seed
const SHORT_COST: string = "3282500000000000000000000"; // + 1.0 short-id premium

function hasPrefix(s: string, p: string): number {
  if (strSlice(s, 0, strLength(p)) == p) { return 1; }
  return 0;
}

// INTEAR's URL rules: ≤50 chars total, must start with the platform prefix,
// handle (suffix) must not contain '/'. twitch additionally needs a handle.
// Empty = absent (validated as "").
function validateSocial(u: string, prefix: string, needHandle: number): string {
  if (strLength(u) == 0) { return ""; }
  if (strLength(u) > 50) { near.abort("ERR_SOCIAL"); return ""; }
  if (hasPrefix(u, prefix) == 0) { near.abort("ERR_SOCIAL"); return ""; }
  let i = strLength(prefix);
  while (i < strLength(u)) {
    if (strSlice(u, i, i + 1) == "/") { near.abort("ERR_SOCIAL"); return ""; }
    i = i + 1;
  }
  if (needHandle == 1 && strLength(u) == strLength(prefix)) {
    near.abort("ERR_SOCIAL");
    return "";
  }
  return u;
}

// "" social → null (NEAR event standard Option<String> shape)
function socialJson(u: string): string {
  if (strLength(u) == 0) { return "null"; }
  return jsonQuote(u);
}

function validateLaunch(name: string, symbolRaw: string): string {
  if (strLength(name) == 0) { near.abort("ERR_NAME"); return ""; }
  if (strLength(name) > 50) { near.abort("ERR_NAME_LONG"); return ""; }
  const symbol = symbolRaw; // must ALREADY be lowercase (no to-lower builtin)
  if (strLength(symbol) < 2 || strLength(symbol) > 8) {
    near.abort("ERR_SYMBOL_LEN");
    return "";
  }
  // ticker must be lowercase alphanumerics (sub-domain rules)
  let i = 0;
  while (i < strLength(symbol)) {
    const c = strSlice(symbol, i, i + 1);
    const isDigit = c == "0" || c == "1" || c == "2" || c == "3" || c == "4" || c == "5" || c == "6" || c == "7" || c == "8" || c == "9";
    const isLetter = c == "a" || c == "b" || c == "c" || c == "d" || c == "e" || c == "f" || c == "g" || c == "h" || c == "i" || c == "j" || c == "k" || c == "l" || c == "m" || c == "n" || c == "o" || c == "p" || c == "q" || c == "r" || c == "s" || c == "t" || c == "u" || c == "v" || c == "w" || c == "x" || c == "y" || c == "z";
    if (!isLetter && !isDigit) { near.abort("ERR_SYMBOL_CHAR"); return ""; }
    i = i + 1;
  }
  return symbol;
}

function validateDecimals(dec: string): string {
  const n = strToNum(dec);
  if (n < 0 || n > 38) { near.abort("ERR_DECIMALS"); return ""; }
  return dec;
}

export function launchToken(): number {
  // args: { name, symbol, total_supply, icon?, decimals?, short_id?,
  //         telegram?, x?, twitch?, website?, description? }
  const name = near.jsonGetStr("name") ?? "";
  const symbolRaw = near.jsonGetStr("symbol") ?? "";
  const totalSupply = near.jsonGetStr("total_supply") ?? "0";
  const icon = near.jsonGetStr("icon") ?? "";
  const dec = validateDecimals(near.jsonGetStr("decimals") ?? "18");
  const short = near.jsonGetStr("short_id") ?? "0";
  const telegram = validateSocial(near.jsonGetStr("telegram") ?? "", "https://t.me/", 0);
  const xUrl = validateSocial(near.jsonGetStr("x") ?? "", "https://x.com/", 0);
  const twitch = validateSocial(near.jsonGetStr("twitch") ?? "", "https://twitch.tv/", 1);
  const website = validateSocial(near.jsonGetStr("website") ?? "", "https://", 0);
  const description = near.jsonGetStr("description") ?? "";
  if (strLength(description) > 200) { near.abort("ERR_DESCRIPTION"); return 0; }
  const symbol = validateLaunch(name, symbolRaw);

  const attached = near.attachedDepositU128();
  let cost = LONG_COST;
  if (short == "1") { cost = SHORT_COST; }
  if (u128Lt(attached, cost)) {
    near.abort("ERR_DEPOSIT");
    return 0;
  }
  // refund the excess (the old code computed it and kept it silently)
  const refund = u128Sub(attached, cost);
  const poolAcct = near.storageGet("pool:acct") ?? "";
  if (strLength(poolAcct) == 0) { near.abort("ERR_NO_POOL_ACCT"); return 0; }
  const ftHash = near.storageGet("ft:global") ?? "";
  if (strLength(ftHash) == 0) {
    near.abort("ERR_NO_FT_GLOBAL");
    return 0;
  }

  // per-symbol id: short = {symbol}.{factory} (registry collision check);
  // long = {symbol}-{n}.{factory} via the counter (n=1 → no suffix)
  let tokenAcct = "";
  if (short == "1") {
    tokenAcct = symbol + "." + near.currentAccountId();
    if (strLength(near.storageGet("li:" + tokenAcct) ?? "") > 0) {
      near.abort("ERR_TAKEN");
      return 0;
    }
  } else {
    const counterKey = "lc:" + symbol;
    const next = strToNum(near.storageGet(counterKey) ?? "0") + 1;
    near.storageSet(counterKey, toStr(next));
    if (next == 1) {
      tokenAcct = symbol + "." + near.currentAccountId();
    } else {
      tokenAcct = symbol + "-" + toStr(next) + "." + near.currentAccountId();
    }
  }

  if (!u128IsZero(refund)) { near.transferU128(near.predecessorAccountId(), refund); }

  // registry: li (immutable facts) + ls (editable socials) + lb (creator)
  // NOTE: json-set value operands must be DIRECT jsonQuote(...) calls — a
  // local holding a jsonQuote result gets re-escaped (t2/t6 repro, GAPS.md
  // 2026-09-27). Empty icon → "" (NEP-148 prefers null; wallets tolerate "").
  const infoKey = "li:" + tokenAcct;
  near.storageSet(infoKey, jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(
            jsonSet(
              jsonSet(
                jsonSet(`{}`, "name", jsonQuote(name)),
                "symbol", jsonQuote(symbol)),
              "total_supply", jsonQuote(totalSupply)),
            "launched_by", jsonQuote(near.predecessorAccountId())),
          "launched_at", jsonQuote(near.blockTimestamp())),
        "token_account", jsonQuote(tokenAcct)),
      "icon", jsonQuote(icon)),
    "decimals", dec));
  near.storageSet("ls:" + tokenAcct, jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(`{}`, "telegram", jsonQuote(telegram)),
          "x", jsonQuote(xUrl)),
        "twitch", jsonQuote(twitch)),
      "website", jsonQuote(website)),
    "description", jsonQuote(description)));
  near.storageSet("lb:" + tokenAcct, near.predecessorAccountId());

  // standard event (near EVENT_JSON convention; standard matches INTEAR's
  // intear_launch so existing indexers can pick launches up). Flat social
  // fields — nested-object values via json-set are the known-local bug.
  const ev = jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(
            jsonSet(jsonSet(`{}`, "token_account_id", jsonQuote(tokenAcct)),
              "launched_by", jsonQuote(near.predecessorAccountId())),
            "name", jsonQuote(name)),
          "symbol", jsonQuote(symbol)),
        "total_supply", jsonQuote(totalSupply)),
      "icon", jsonQuote(icon)),
    "decimals", dec);
  near.log("EVENT_JSON:" + `{"standard":"intear_launch","version":"1.0.0","event":"token_launched","data":${ev}}`);

  // ── the promise chain (detached) ──
  const p1 = near.promiseBatchCreate(tokenAcct);
  near.promiseBatchActionCreateAccount(p1);
  near.promiseBatchActionUseGlobalContract(p1, ftHash);
  near.promiseBatchActionTransfer(p1, "1250000000000000000000000");
  near.promiseBatchActionFunctionCall(
    p1, "new",
    `{"owner_id":"${near.currentAccountId()}","total_supply":"${totalSupply}","name":${jsonQuote(name)},"symbol":${jsonQuote(symbol)},"icon":${jsonQuote(icon)},"decimals":"${dec}"}`,
    "0", 35000000000000);

  const p2 = near.promiseBatchThen(p1, poolAcct);
  near.promiseBatchActionFunctionCall(
    p2, "seed_pool",
    `{"token":"${tokenAcct}"}`,
    "1000000000000000000000000", 40000000000000);

  const p3 = near.promiseBatchThen(p2, tokenAcct);
  near.promiseBatchActionFunctionCall(
    p3, "ft_transfer_call",
    `{"receiver_id":"${poolAcct}","amount":"${totalSupply}","memo":null,"msg":"seed"}`,
    // 100T: the entry pays for its own exec (~2T) PLUS the gas it attaches
    // to callAwait's sub-promises (40T ft_on_transfer + 40T resolve) — the
    // 2026-09-27 meme launch OOG'd at 40T exactly on the second attach
    // (69BZLn5L "Exceeded the prepaid gas", 1T burnt before the 40T+40T
    // deduction overflowed the envelope)
    "1", 100000000000000);
  near.promiseReturn(p3);

  near.log(`launched:${tokenAcct}:${symbol}:${refund}`);
  return 0;
}

// creator edits the socials/description of THEIR token; refunds the attached
// deposit (registry storage delta is negligible at these field caps)
export function editToken(): number {
  const token = near.jsonGetStr("token") ?? "";
  const by = near.storageGet("lb:" + token) ?? "";
  if (strLength(by) == 0) { near.abort("ERR_NO_TOKEN"); return 0; }
  if (by != near.predecessorAccountId()) { near.abort("ERR_NOT_CREATOR"); return 0; }
  // Partial-edit safe (2026-09-27): omitted args keep their stored value
  // (explicit "" still clears). Previously omitting a field WIPED it —
  // editing just telegram nuked website/description (found in e2e replay).
  // Fallback chain: ls: (edit record) → li: (launch record) → "{}".
  // NOTE: ?? fallback must be a definite str — default the nullable
  // middles to "" first, then `in ?? prevResolved`.
  const li = near.storageGet("li:" + token) ?? "{}";
  const prev = near.storageGet("ls:" + token) ?? li;
  const tPrev = near.jsonGetStr("telegram", prev) ?? "";
  const telegram = validateSocial(near.jsonGetStr("telegram") ?? tPrev, "https://t.me/", 0);
  const xPrev = near.jsonGetStr("x", prev) ?? "";
  const xUrl = validateSocial(near.jsonGetStr("x") ?? xPrev, "https://x.com/", 0);
  const twPrev = near.jsonGetStr("twitch", prev) ?? "";
  const twitch = validateSocial(near.jsonGetStr("twitch") ?? twPrev, "https://twitch.tv/", 1);
  const wPrev = near.jsonGetStr("website", prev) ?? "";
  const website = validateSocial(near.jsonGetStr("website") ?? wPrev, "https://", 0);
  const dPrev = near.jsonGetStr("description", prev) ?? "";
  const description = near.jsonGetStr("description") ?? dPrev;
  if (strLength(description) > 200) { near.abort("ERR_DESCRIPTION"); return 0; }

  near.storageSet("ls:" + token, jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(`{}`, "telegram", jsonQuote(telegram)),
          "x", jsonQuote(xUrl)),
        "twitch", jsonQuote(twitch)),
      "website", jsonQuote(website)),
    "description", jsonQuote(description)));

  const attached = near.attachedDepositU128();
  if (!u128IsZero(attached)) { near.transferU128(near.predecessorAccountId(), attached); }

  const ev = jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(jsonSet(`{}`, "token_account_id", jsonQuote(token)),
            "edited_by", jsonQuote(near.predecessorAccountId())),
          "telegram", jsonQuote(telegram)),
        "x", jsonQuote(xUrl)),
      "twitch", jsonQuote(twitch)),
    "website", jsonQuote(website));
  const ev2 = jsonSet(ev, "description", jsonQuote(description));
  near.log("EVENT_JSON:" + `{"standard":"intear_launch","version":"1.0.0","event":"token_edited","data":${ev2}}`);
  near.log(`edited:${token}`);
  return 0;
}

// owner: register the FT global contract hash (once)
export function setFtGlobal(): number {
  if (near.predecessorAccountId() != near.currentAccountId()) {
    near.abort("ERR_OWNER_ONLY");
    return 0;
  }
  const hash = near.jsonGetStr("hash") ?? "";
  if (strLength(hash) != 64) { near.abort("ERR_HASH"); return 0; }
  near.storageSet("ft:global", hash);
  near.log(`ft_global_set:${hash}`);
  return 0;
}

// owner: register the pool contract account (once)
export function setPoolAccount(): number {
  if (near.predecessorAccountId() != near.currentAccountId()) {
    near.abort("ERR_OWNER_ONLY");
    return 0;
  }
  const acct = near.jsonGetStr("account") ?? "";
  if (strLength(acct) == 0) { near.abort("ERR_ACCT"); return 0; }
  near.storageSet("pool:acct", acct);
  near.log(`pool_acct_set:${acct}`);
  return 0;
}

// ── views ──────────────────────────────────────────────────────────
export function getToken(): string {
  return near.storageGet("li:" + (near.jsonGetStr("token") ?? "")) ?? "{}";
}
export function count(): number {
  return strToNum(near.storageGet("lc:" + (near.jsonGetStr("symbol") ?? "")) ?? "0");
}

// merged registry entry: immutable li + editable socials (ls) — string merge
// of the two JSON objects (no arbitrary-JSON parser in the dialect)
export function getLaunchData(): string {
  const token = near.jsonGetStr("token") ?? "";
  const li = near.storageGet("li:" + token) ?? "{}";
  const ls = near.storageGet("ls:" + token) ?? "{}";
  if (ls == "{}") { return li; }
  const liBody = strSlice(li, 0, strLength(li) - 1); // strip closing brace
  const lsBody = strSlice(ls, 1, strLength(ls));     // strip opening brace
  return liBody + "," + lsBody;
}

// what account WOULD a launch with these params get? (no registration)
export function previewId(): string {
  const symbol = near.jsonGetStr("symbol") ?? "";
  const short = near.jsonGetStr("short_id") ?? "0";
  if (short == "1") {
    return symbol + "." + near.currentAccountId();
  }
  const next = strToNum(near.storageGet("lc:" + symbol) ?? "0") + 1;
  if (next == 1) { return symbol + "." + near.currentAccountId(); }
  return symbol + "-" + toStr(next) + "." + near.currentAccountId();
}

export function longIdCost(): string { return LONG_COST; }
export function shortIdCost(): string { return SHORT_COST; }