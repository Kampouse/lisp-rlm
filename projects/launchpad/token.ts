// token.ts — the pre-deployed global FT contract (minimal, ~8 KB)
//
// Deployed ONCE as a global contract; launches adopt it via
// use_global_contract, then initialize with new_().
// Storage: u128 money ledger (near/storage), plain strings.

function balKey(a: string): string { return "b:" + a; }

// init: mints total_supply to owner
export function new_(): number {
  if ((near.storageGet("ok") ?? "") != "") { near.abort("ERR_INIT"); return 0; }
  const ownerId = near.jsonGetStr("owner_id") ?? "";
  const totalSupply = near.jsonGetStr("total_supply") ?? "0";
  if (strLength(ownerId) == 0 || u128IsZero(totalSupply)) { near.abort("ERR_ARGS"); return 0; }
  near.storageSet("supply", totalSupply);
  near.storageSet(balKey(ownerId), totalSupply);
  near.storageSet("ok", "1");
  return 0;
}

export function ftTransfer(): number {
  const to = near.jsonGetStr("receiver_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const from = near.predecessorAccountId();
  const fromBal = near.storageGet(balKey(from)) ?? "0";
  if (u128Lt(fromBal, amount)) { near.abort("ERR_INSUFFICIENT"); return 0; }
  near.storageSet(balKey(from), u128Sub(fromBal, amount));
  near.storageSet(balKey(to), u128Add(near.storageGet(balKey(to)) ?? "0", amount));
  return 0;
}

// ── NEP-141 ft_transfer_call — escrow → ft_on_transfer → resolve ──
//
// Standard flow: deduct the FULL amount from the sender (escrow), promise
// receiver.ft_on_transfer({sender_id, amount, msg}), then resolve on self:
// credit the receiver only the amount it did NOT return, refund the rest.
// The resolve callback reads the callee's return value via promiseResult(0)
// ("" on failure → full refund; the emitter returns "" fail-closed).
export function ft_transfer_call(): number {
  const receiver = near.jsonGetStr("receiver_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const msg = near.jsonGetStr("msg") ?? "";
  const sender = near.predecessorAccountId();
  if (strLength(receiver) == 0) { near.abort("ERR_receiver"); return 0; }
  if (receiver == near.currentAccountId()) { near.abort("ERR_SELF"); return 0; }
  if (u128IsZero(amount)) { near.abort("ERR_ZERO"); return 0; }
  const fromBal = near.storageGet(balKey(sender)) ?? "0";
  if (u128Lt(fromBal, amount)) { near.abort("ERR_INSUFFICIENT"); return 0; }
  near.storageSet(balKey(sender), u128Sub(fromBal, amount));
  near.callAwait(receiver, "ft_on_transfer",
    `{"sender_id":${jsonQuote(sender)},"amount":${jsonQuote(amount)},"msg":${jsonQuote(msg)}}`,
    40000000000000, "ft_resolve_transfer", 40000000000000,
    `{"sender_id":${jsonQuote(sender)},"receiver_id":${jsonQuote(receiver)},"amount":${jsonQuote(amount)}}`);
  return 0;
}

// resolve: callee returned the UNUSED amount as a U128 string ("0" = used
// fully, empty string = the call failed → refund everything).
export function ft_resolve_transfer(): number {
  const sender = near.jsonGetStr("sender_id") ?? "";
  const receiver = near.jsonGetStr("receiver_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  let refund = amount;
  if (near.promiseSucceeded(0) == 1) {
    const ret = near.promiseResult(0);
    if (strLength(ret) > 0) {
      if (u128Gt(ret, amount)) {
        refund = amount; // callee returned nonsense → fail safe, refund all
      } else {
        refund = ret;
      }
    }
  }
  const used = u128Sub(amount, refund);
  if (!u128IsZero(used)) {
    near.storageSet(balKey(receiver), u128Add(near.storageGet(balKey(receiver)) ?? "0", used));
  }
  if (!u128IsZero(refund)) {
    near.storageSet(balKey(sender), u128Add(near.storageGet(balKey(sender)) ?? "0", refund));
  }
  near.log(`resolve:${receiver}:${used}:${refund}`);
  return 0;
}

export function storageDeposit(): number {
  const who = near.jsonGetStr("account_id") ?? near.predecessorAccountId();
  if ((near.storageGet(balKey(who)) ?? "") == "") { near.storageSet(balKey(who), "0"); }
  return 0;
}

export function ftBalanceOf(): string {
  return near.storageGet(balKey(near.jsonGetStr("account_id") ?? "")) ?? "0";
}
export function ftTotalSupply(): string { return near.storageGet("supply") ?? "0"; }