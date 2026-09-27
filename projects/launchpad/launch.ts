// launchpad.ts — INTEAR-style launch factory (TS dialect)
//
// Flow (mirrors INTEARnear/launch launch_token, simplified v1):
//   1. @payable: caller attaches NEAR (fixed cost + refund of the excess)
//   2. per-symbol launch counter → token account id
//      (launch #1: {symbol}.{factory}, later: {symbol}-{n}.{factory})
//   3. promise chain (detached):
//        create token sub-account
//        adopt a pre-deployed global FT contract (ft:global hash)
//        fund it (transfer)
//        initialize it (function_call "new": owner, supply, metadata)
//
// The token code lives in the GLOBAL CONTRACT registry (deployed once
// via `near-compile deploy --global`); every launch adopts it — no
// per-launch deploy cost, no storage duplication.

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

export function launchToken(): number {
  // args: { name, symbol, total_supply (decimal string) }
  const name = near.jsonGetStr("name") ?? "";
  const symbolRaw = near.jsonGetStr("symbol") ?? "";
  const totalSupply = near.jsonGetStr("total_supply") ?? "0";
  const symbol = validateLaunch(name, symbolRaw);

  // cost: 0.0325 launch fee + 1.25 token funding + 1.0 pool NEAR seed
  const cost: string = "2282500000000000000000000";
  const seed: string = "1000000000000000000000000";
  const attached = near.attachedDepositU128();
  if (u128Lt(attached, cost)) {
    near.abort("ERR_DEPOSIT");
    return 0;
  }
  const refund = u128Sub(attached, cost);
  const poolAcct = near.storageGet("pool:acct") ?? "";
  if (strLength(poolAcct) == 0) { near.abort("ERR_NO_POOL_ACCT"); return 0; }

  // per-symbol launch counter → token account id
  const counterKey = "lc:" + symbol;
  const next = strToNum(near.storageGet(counterKey) ?? "0") + 1;
  near.storageSet(counterKey, toStr(next));
  const tokenAcct = next == 1
    ? symbol + "." + near.currentAccountId()
    : symbol + "-" + toStr(next) + "." + near.currentAccountId();

  // registry entry (indexed storage)
  const infoKey = "li:" + tokenAcct;
  near.storageSet(infoKey, jsonSet(
    jsonSet(
      jsonSet(
        jsonSet(
          jsonSet(
            jsonSet(`{}`, "name", jsonQuote(name)),
            "symbol", jsonQuote(symbol)),
          "total_supply", jsonQuote(totalSupply)),
        "launched_by", jsonQuote(near.predecessorAccountId())),
      "launched_at", jsonQuote(near.blockTimestamp())),
    "token_account", jsonQuote(tokenAcct)));

  // ── the promise chain (detached; INTEAR-launch shape) ──
  //   leg 1: create token account + adopt global FT + fund + init
  //   leg 2: pool.seed_pool({token}) — attached 1.0 NEAR = starting reserve
  //   leg 3: token.ft_transfer_call(pool, full supply, msg="seed")
  const ftHash = near.storageGet("ft:global") ?? "";
  if (strLength(ftHash) == 0) {
    near.abort("ERR_NO_FT_GLOBAL");
    return 0;
  }

  const p1 = near.promiseBatchCreate(tokenAcct);
  near.promiseBatchActionCreateAccount(p1);
  near.promiseBatchActionUseGlobalContract(p1, ftHash);
  near.promiseBatchActionTransfer(p1, "1250000000000000000000000");
  near.promiseBatchActionFunctionCall(
    p1, "new",
    `{"owner_id":"${near.currentAccountId()}","total_supply":"${totalSupply}","metadata":{"spec":"ft-1.0.0","name":"${name}","symbol":"${symbol}","decimals":18}}`,
    "0", 35000000000000);

  const p2 = near.promiseBatchThen(p1, poolAcct);
  near.promiseBatchActionFunctionCall(
    p2, "seed_pool",
    `{"token":"${tokenAcct}"}`,
    seed, 40000000000000);

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