/// Permissionless RFQ Orderbook ///
///
/// Anyone can deposit balances and sign limit orders off-chain.
/// Anyone can fill a signed order on-chain.
/// Contract verifies nonce, checks balances, swaps atomically.
///
/// Storage layout:
///   "balances/{account}/{base}" -> stringified number
///   "nonce/{account}"               -> number (anti-replay)
///
/// Convention: near_xxx() maps to near/xxx lisp builtin

// -- Storage helpers --

function balKey(account: string, base: string): string {
  return strCat("balances/", strCat(account, strCat("/", base)));
}

function nonceKey(account: string): string {
  return strCat("nonce/", account);
}

function readBalance(account: string, base: string): number {
  const raw = near_storage_get(balKey(account, base));
  if (raw === "") {
    return 0;
  }
  return strToNum(raw);
}

function writeBalance(account: string, base: string, amount: number): void {
  near_storage_set(balKey(account, base), toStr(amount));
}

// -- Nonce helpers --

function getNonce(account: string): number {
  const raw = near_storage_get(nonceKey(account));
  if (raw === "") {
    return 0;
  }
  return strToNum(raw);
}

function setNonce(account: string, n: number): void {
  near_storage_set(nonceKey(account), toStr(n));
}

// -- Views --

export function getBalance(account: string, base: string): number {
  return readBalance(account, base);
}

export function getNonceView(account: string): number {
  return getNonce(account);
}

// -- Deposit / Withdraw --

export function deposit(base: string, amount: string): string {
  const caller = near_predecessor_account_id();
  const amt = strToNum(amount);
  const current = readBalance(caller, base);
  const new_bal = current + amt;
  writeBalance(caller, base, new_bal);
  return "ok";
}

export function withdraw(base: string, amount: string): string {
  const caller = near_predecessor_account_id();
  const amt = strToNum(amount);
  const current = readBalance(caller, base);
  if (current < amt) {
    return "insufficient balance";
  }
  writeBalance(caller, base, current - amt);
  return "ok";
}

// -- Fill order --

export function fill_order(
  maker: string,
  base: string,
  side: string,
  price: string,
  size: string,
  nonce: string
): string {
  // 1. Anti-replay: nonce must match
  const stored = getNonce(maker);
  const order_nonce = strToNum(nonce);
  if (order_nonce !== stored) {
    return "invalid nonce";
  }
  setNonce(maker, stored + 1);

  // 2. Parse order params
  const px = strToNum(price);
  const sz = strToNum(size);
  const taker = near_predecessor_account_id();
  const quote = strCat(base, "-USDC");
  const total_cost = sz * px;

  if (side === "sell") {
    // Maker sells base to taker. Maker needs base, taker needs quote.
    const maker_base = readBalance(maker, base);
    const taker_quote = readBalance(taker, quote);

    if (maker_base < sz) {
      return "maker insufficient base";
    }
    if (taker_quote < total_cost) {
      return "taker insufficient quote";
    }

    // Atomic swap
    writeBalance(maker, base, maker_base - sz);
    writeBalance(taker, base, readBalance(taker, base) + sz);
    writeBalance(taker, quote, taker_quote - total_cost);
    writeBalance(maker, quote, readBalance(maker, quote) + total_cost);

    return "filled";
  } else {
    // Maker buys base from taker. Taker needs base, maker needs quote.
    const taker_base = readBalance(taker, base);
    const maker_quote = readBalance(maker, quote);

    if (taker_base < sz) {
      return "taker insufficient base";
    }
    if (maker_quote < total_cost) {
      return "maker insufficient quote";
    }

    // Atomic swap
    writeBalance(taker, base, taker_base - sz);
    writeBalance(maker, base, readBalance(maker, base) + sz);
    writeBalance(maker, quote, maker_quote - total_cost);
    writeBalance(taker, quote, readBalance(taker, quote) + total_cost);

    return "filled";
  }
}