// ─── exchange.ts — Ref-shaped atomic router: pools + custody ledger ───
// Mirrors the real Ref UX: deposit once, swap ALL hops in ONE receipt with
// per-hop min_amount_out; any hop misses → near.panic → the WHOLE receipt
// reverts (near-mock restores the callee's state partition on trap), so a
// failed trade strands nothing. This is the atomicity the demo dex chain
// lacked (there, legs settled forward and a mid-chain fail stranded tokens).
//
// Mock seam: NEP-141 ft_transfer_call is replaced by deposit() — a bare
// ledger credit. Everything downstream (debit → hop walk → credit, revert
// on miss) is the real custody flow shape.
//
// Storage (pools follow dex.ts conventions so scouts work on both):
//   s:<id>            "base|quote|fee_bps"
//   r:<id>:base/quote integer reserves
//   b:<account>:<token> token balance ledger
//   n:<id>            "inBase,outQuote" cumulative flows (asserts)
//
// Exports:
//   init_pool(id, base, quote, fee_bps)
//   fund_pool(id, side, amount)            LP seed (setup-only seam)
//   deposit(account, token, amount)        ft seam (setup-only)
//   balance_of(account, token) → str       view
//   quote(id, side, amount) → str          view: out for in
//   swap_router(c0,d0,a0,m0, c1,d1,a1,m1, c2,d2,a2,m2, min_total) → str
//     a_i = 0 → hop absent (2-leg cycle). Entry token debited from the
//     CALLER's ledger (predecessor), final token credited back to it.

function outFor(ain: number, rIn: number, rOut: number, feeBps: number): number {
  const ainFee = ain - (ain * feeBps) / 10000;
  return (ainFee * rOut) / (rIn + ainFee);
}

function specOf(id: string): string {
  return near.storageGet("s:" + id) ?? "||30";
}

function reserves(id: string, side: string): number {
  return strToNum(near.storageGet("r:" + id + ":" + side) ?? "0");
}

function feeOf(id: string): number {
  return strToNum(strSplit(specOf(id), "|")[2] ?? "30");
}

function balOf(acct: string, token: string): number {
  return strToNum(near.storageGet("b:" + acct + ":" + token) ?? "0");
}

function ledgerAdd(acct: string, token: string, delta: number): void {
  near.storageSet("b:" + acct + ":" + token, toStr(balOf(acct, token) + delta));
}

// one hop: side "base" sells pool base for quote, "quote" the reverse.
// returns output amount; panics (reverting the whole receipt) under min.
function hop(id: string, side: string, ain: number, minOut: number): number {
  let rIn = reserves(id, "base");
  let rOut = reserves(id, "quote");
  if (side == "quote") {
    rIn = reserves(id, "quote");
    rOut = reserves(id, "base");
  }
  const out = outFor(ain, rIn, rOut, feeOf(id));
  if (out < minOut) {
    near.panic("slippage: hop output under min_amount_out");
  }
  let kIn = "r:" + id + ":base";
  let kOut = "r:" + id + ":quote";
  if (side == "quote") {
    kIn = "r:" + id + ":quote";
    kOut = "r:" + id + ":base";
  }
  near.storageSet(kIn, toStr(rIn + ain));
  near.storageSet(kOut, toStr(rOut - out));
  return out;
}

// ---- entry points ----

export function init_pool(id: string, base: string, quote: string, fee_bps: number): number {
  near.storageSet("s:" + id, base + "|" + quote + "|" + toStr(fee_bps));
  near.storageSet("r:" + id + ":base", "0");
  near.storageSet("r:" + id + ":quote", "0");
  near.storageSet("n:" + id, "0,0");
  return 1;
}

export function fund_pool(id: string, side: string, amount: number): number {
  const k = "r:" + id + ":" + side;
  const cur = strToNum(near.storageGet(k) ?? "0");
  near.storageSet(k, toStr(cur + amount));
  return cur + amount;
}

export function deposit(account: string, token: string, amount: number): number {
  ledgerAdd(account, token, amount);
  return balOf(account, token);
}

export function balance_of(account: string, token: string): string {
  return toStr(balOf(account, token));
}

export function quote(id: string, side: string, amount: number): string {
  let rIn = reserves(id, "base");
  let rOut = reserves(id, "quote");
  if (side == "quote") {
    rIn = reserves(id, "quote");
    rOut = reserves(id, "base");
  }
  return toStr(outFor(amount, rIn, rOut, feeOf(id)));
}

export function swap_router(
  c0: string, d0: string, a0: number, m0: number,
  c1: string, d1: string, a1: number, m1: number,
  c2: string, d2: string, a2: number, m2: number,
  min_total: number
): string {
  const me = near.predecessorAccountId();
  const p0 = strSplit(specOf(c0), "|");
  let entry = p0[1] ?? "";
  if (d0 == "base") {
    entry = p0[0] ?? "";
  }
  if (balOf(me, entry) < a0) {
    near.panic("insufficient ledger balance");
  }
  // hop 0
  let amt = hop(c0, d0, a0, m0);
  const pk0 = strSplit(specOf(c0), "|");
  // side "base" sells base → output is QUOTE (parts[1]); "quote" → base
  let tok = pk0[1] ?? "";
  if (d0 == "quote") {
    tok = pk0[0] ?? "";
  }
  // hop 1 (a1 == 0 → absent)
  if (a1 > 0) {
    amt = hop(c1, d1, amt, m1);
    const po1 = strSplit(specOf(c1), "|");
    let t1 = po1[1] ?? "";
    if (d1 == "quote") {
      t1 = po1[0] ?? "";
    }
    tok = t1;
  }
  // hop 2 (a2 == 0 → absent)
  if (a2 > 0) {
    amt = hop(c2, d2, amt, m2);
    const po2 = strSplit(specOf(c2), "|");
    let t2 = po2[1] ?? "";
    if (d2 == "quote") {
      t2 = po2[0] ?? "";
    }
    tok = t2;
  }
  if (amt < min_total) {
    near.panic("cycle output under min_total");
  }
  // custody settle: debit entry, credit final — same receipt, atomic
  ledgerAdd(me, entry, 0 - a0);
  ledgerAdd(me, tok, amt);
  return toStr(amt);
}
