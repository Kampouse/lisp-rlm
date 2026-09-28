// token.ts — the pre-deployed global FT contract (minimal, ~8 KB)
//
// Deployed ONCE as a global contract; launches adopt it via
// use_global_contract, then initialize with new_().
// Storage: u128 money ledger (near/storage), plain strings.
//
// v2 (2026-09-27, prod hardening):
//   - NEP-141 events: EVENT_JSON standard "nep141" ft_transfer on plain
//     transfers AND on ft_transfer_call resolution (net amount, one event)
//   - storage API: storage_balance_of / storage_deposit (attach-backed
//     ledger s:acct) + storage_minimum_balance — wallets/indexers expect
//     them; NOTE registration is LENIENT (balance auto-appears on first
//     receipt, no hard gate) because the launch promise chain transfers
//     the full supply to the pool without a separate registration hop.
//     storage_deposit exists so wallets can pre-register the standard way.
//   - "0"-amount transfers still abort; resolve refunds stay fail-closed.

function balKey(a: string): string { return "b:" + a; }
function storKey(a: string): string { return "s:" + a; }

const STORAGE_MIN: string = "100000000000000000000000"; // 0.01 N (display only)

// nep141 standard transfer event (flat JSON hand-built — json-set values
// are always strings here; memo is emitted as null per the standard)
function transferEvent(oldOwner: string, newOwner: string, amount: string): number {
  near.log("EVENT_JSON:" + `{"standard":"nep141","version":"1.0.0","event":"ft_transfer","data":{"old_owner_id":${jsonQuote(oldOwner)},"new_owner_id":${jsonQuote(newOwner)},"amount":${jsonQuote(amount)},"memo":null}}`);
  return 0;
}

// init: mints total_supply to owner; stores NEP-148 metadata fields
// (name/symbol/icon/decimals arrive flat from the launchpad's new call)
export function new_(): number {
  if ((near.storageGet("ok") ?? "") != "") { near.abort("ERR_INIT"); return 0; }
  const ownerId = near.jsonGetStr("owner_id") ?? "";
  const totalSupply = near.jsonGetStr("total_supply") ?? "0";
  const name = near.jsonGetStr("name") ?? "";
  const symbol = near.jsonGetStr("symbol") ?? "";
  const icon = near.jsonGetStr("icon") ?? "";
  const dec = near.jsonGetStr("decimals") ?? "18";
  if (strLength(ownerId) == 0 || u128IsZero(totalSupply)) { near.abort("ERR_ARGS"); return 0; }
  if (strLength(name) == 0 || strLength(symbol) == 0) { near.abort("ERR_META"); return 0; }
  near.storageSet("supply", totalSupply);
  near.storageSet(balKey(ownerId), totalSupply);
  near.storageSet("name", name);
  near.storageSet("sym", symbol);
  near.storageSet("icon", icon);
  near.storageSet("dec", dec);
  near.storageSet("ok", "1");
  return 0;
}

// NEP-148 metadata view — wallets/DEX UIs call this. icon emitted as null
// when absent (the standard's Option<String> shape).
export function ft_metadata(): string {
  const icon = near.storageGet("icon") ?? "";
  let iconJson = "null";
  if (strLength(icon) > 0) { iconJson = jsonQuote(icon); }
  return `{"spec":"ft-1.0.0","name":${jsonQuote(near.storageGet("name") ?? "")},"symbol":${jsonQuote(near.storageGet("sym") ?? "")},"decimals":${near.storageGet("dec") ?? "18"},"icon":${iconJson},"reference":null,"reference_hash":null}`;
}

export function ftTransfer(): number {
  const to = near.jsonGetStr("receiver_id") ?? "";
  const amount = near.jsonGetStr("amount") ?? "0";
  const from = near.predecessorAccountId();
  const fromBal = near.storageGet(balKey(from)) ?? "0";
  if (u128Lt(fromBal, amount)) { near.abort("ERR_INSUFFICIENT"); return 0; }
  if (u128IsZero(amount)) { near.abort("ERR_ZERO"); return 0; }
  near.storageSet(balKey(from), u128Sub(fromBal, amount));
  near.storageSet(balKey(to), u128Add(near.storageGet(balKey(to)) ?? "0", amount));
  transferEvent(from, to, amount);
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
  if (!u128IsZero(used)) { transferEvent(sender, receiver, used); }
  near.log(`resolve:${receiver}:${used}:${refund}`);
  return 0;
}

// ── storage registration (standard views; LENIENT model, see header) ──

export function storageDeposit(): number {
  const who = near.jsonGetStr("account_id") ?? near.predecessorAccountId();
  if ((near.storageGet(balKey(who)) ?? "") == "") { near.storageSet(balKey(who), "0"); }
  const attached = near.attachedDepositU128();
  if (!u128IsZero(attached)) {
    near.storageSet(storKey(who), u128Add(near.storageGet(storKey(who)) ?? "0", attached));
  }
  return 0;
}

export function storageWithdraw(): number {
  const who = near.predecessorAccountId();
  const amount = near.jsonGetStr("amount") ?? STORAGE_MIN;
  const st = near.storageGet(storKey(who)) ?? "0";
  if (u128Gt(amount, st)) { near.abort("ERR_STORAGE"); return 0; }
  near.storageSet(storKey(who), u128Sub(st, amount));
  near.transferU128(who, amount);
  return 0;
}

export function storageBalanceOf(): string {
  const who = near.jsonGetStr("account_id") ?? "";
  return near.storageGet(storKey(who)) ?? "0";
}

export function storageMinimumBalance(): string { return STORAGE_MIN; }

export function storage_unregister(): number {
  const who = near.predecessorAccountId();
  const bal = near.storageGet(balKey(who)) ?? "0";
  if (!u128IsZero(bal)) {
    const force = near.jsonGetStr("force") ?? "0";
    if (force != "1") { near.abort("ERR_BALANCE"); return 0; }
    // burn the leftover supply so total stays consistent
    near.storageSet("supply", u128Sub(near.storageGet("supply") ?? "0", bal));
  }
  near.storageRemove(balKey(who));
  const st = near.storageGet(storKey(who)) ?? "0";
  if (!u128IsZero(st)) { near.transferU128(who, st); }
  near.storageRemove(storKey(who));
  near.log(`unregistered:${who}`);
  return 0;
}

// ── views ──────────────────────────────────────────────────────────
export function ftBalanceOf(): string {
  return near.storageGet(balKey(near.jsonGetStr("account_id") ?? "")) ?? "0";
}
export function ftTotalSupply(): string { return near.storageGet("supply") ?? "0"; }
