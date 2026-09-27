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

  // fixed launch cost: 0.0325 NEAR (INTEAR's ID_COST shape)
  const cost: string = "32500000000000000000000";
  const attached = near.attachedDepositU128();
  if (u128Lt(attached, cost)) {
    near.abort("ERR_DEPOSIT");
    return 0;
  }
  const refund = u128Sub(attached, cost);

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

  // ── the promise chain (detached; mirrors INTEAR launch_token) ──
  const ftHash = near.storageGet("ft:global") ?? "";
  if (strLength(ftHash) == 0) {
    near.abort("ERR_NO_FT_GLOBAL");
    return 0;
  }

  const p = near.promiseBatchCreate(tokenAcct);
  near.promiseBatchActionCreateAccount(p);
  near.promiseBatchActionUseGlobalContract(p, ftHash);
  near.promiseBatchActionTransfer(p, "1250000000000000000000000");
  near.promiseBatchActionFunctionCall(
    p, "new",
    `{"owner_id":"${near.currentAccountId()}","total_supply":"${totalSupply}","metadata":{"spec":"ft-1.0.0","name":"${name}","symbol":"${symbol}","decimals":18}}`,
    "0", 35000000000000);
  near.promiseReturn(p);

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

// ── views ──────────────────────────────────────────────────────────
export function getToken(): string {
  return near.storageGet("li:" + (near.jsonGetStr("token") ?? "")) ?? "{}";
}
export function count(): number {
  return strToNum(near.storageGet("lc:" + (near.jsonGetStr("symbol") ?? "")) ?? "0");
}