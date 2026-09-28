// Launchpad v1 — TypeScript contract, borsh-free (2026-09-27).
//
// Port of INTEAR launch lib.rs to the lisp-rlm TS dialect, graduation via
// Ref Finance (JSON create_pool) instead of the borsh-only INTEAR xyk DEX.
//
// launch_token(creator, symbol):
//   1. require attached deposit >= LAUNCH_FEE (1.1 NEAR)
//   2. derive token account: <symbol>.<self> (collision → abort)
//   3. promise batch on the token account:
//      create_account → fund → deploy stub-FT → mint supply to self
//   4. promise batch on the Ref router:
//      storage_deposit → create_pool(token/NEAR, fee) → register token
//      (stub-Ref = JSON passthrough; mainnet Ref = same JSON shapes)
//
// Money: `bigint` literals are u128-scale (yocto), lattice arithmetic —
// the lending.ts pattern (u128/* str-ops are not on the wasm target).
// depositGte(lo64, hi64): 1.1e24 → lo 2204140625727586304, hi 59631.

const SUPPLY: bigint = 1000000000000000000000000000n; // 1e27, whole-coin supply
const ZERO: bigint = 0n;

function tokenAccount(symbol: string): string {
  return strCat(symbol, ".", near.currentAccountId());
}

export function launch_token(creator: string, symbol: string): void {
  if (near.depositGte(2204140625727586304, 59631) == 0) {
    near.abort("insufficient deposit: need 1.1 NEAR");
  }
  let tok = tokenAccount(symbol);
  if (near.storageHasKey(strCat("lp:tok:", tok))) {
    near.abort("symbol taken");
  }
  near.storageSet(strCat("lp:tok:", tok), creator);
  near.storageSet(strCat("lp:sym:", symbol), tok);

  // batch 1: spawn the token (create account, fund, deploy code, mint here)
  let p1 = near.promiseBatchCreate(tok);
  near.promiseBatchActionCreateAccount(p1);
  near.promiseBatchActionTransfer(p1, "500000000000000000000000"); // 0.5 NEAR OID
  near.promiseBatchActionFunctionCall(
    p1,
    "deploy",
    strCat("{\"codeHash\":", jsonQuote("d69fb74efd097d7d8e6c3a3cb6c4ac3a5f24b83bdf8dbe8cbbe0e2b1e2cd0c2a"), "}"),
    ZERO,
    10000000000000,
  );
  near.promiseBatchActionFunctionCall(
    p1,
    "mint",
    strCat("{\"to\":", jsonQuote(near.currentAccountId()), ",\"amt\":\"", toStr(SUPPLY), "\"}"),
    ZERO,
    10000000000000,
  );
  near.promiseReturn(p1);

  // batch 2: open the Ref pool (JSON all the way — no borsh)
  let p2 = near.promiseBatchCreate("ref-finance.testnet.near");
  near.promiseBatchActionFunctionCall(
    p2,
    "storage_deposit",
    strCat("{\"account_id\":", jsonQuote(tok), ",\"registration_only\":true}"),
    "100000000000000000000000",
    10000000000000,
  );
  near.promiseBatchActionFunctionCall(
    p2,
    "create_pool",
    strCat(
      "{\"token_a\":", jsonQuote(tok),
      ",\"token_b\":\"wrap.testnet.near\"",
      ",\"fee\":30}",
    ),
    ZERO,
    10000000000000,
  );
  near.promiseBatchActionFunctionCall(
    p2,
    "register_tokens",
    strCat("[", jsonQuote(tok), "]"),
    ZERO,
    10000000000000,
  );
  near.promiseReturn(p2);
  near.log(strCat("launched ", tok));
}

// graduate(token, nearAmount): seed the Ref pool with real liquidity.
// v1: launchpad owns the numbers; production reads the FT balance_of.
export function graduate(token: string, nearAmount: string): void {
  let owner = near.storageGet(strCat("lp:tok:", token)) ?? "";
  if (owner == "") {
    near.abort("unknown token");
  }
  if (near.storageHasKey(strCat("lp:grad:", token))) {
    near.abort("already graduated");
  }
  near.storageSet(strCat("lp:grad:", token), nearAmount);
  // fund the pool: wrap deposit carrying the NEAR in (JSON, no borsh)
  let p = near.promiseBatchCreate("ref-finance.testnet.near");
  near.promiseBatchActionFunctionCall(
    p,
    "deposit",
    strCat("{\"token\":\"wrap.testnet.near\",\"amount\":\"", nearAmount, "\"}"),
    nearAmount,
    10000000000000,
  );
  near.promiseReturn(p);
  near.log(strCat("graduated ", token, " with ", nearAmount, " yocto"));
}
