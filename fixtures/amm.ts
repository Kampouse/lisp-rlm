// ── AMM — constant product x·y=k, LP shares, 0.3% swap fee ──
//
// Protocol #14. Self-contained pool: two internal token ledgers
// ("amm:w:<who>" {a,b}) minted at init, so the suite can verify
// CONSERVATION — every token minted is in a wallet or in the reserves,
// after every operation.
//
//   addLiquidity(a, b)   first LP: shares = mira, rb
//                        later:    shares = ts * aIn / ra  (ratio-locked)
//   removeLiquidity(s)   out = reserve * s / ts  (floor)
//   swapExactIdir, in out = rOut * in*997 / (rIn*1000 + in*997)  (floor)
//
// TYPE DISCIPLINE (learned the hard way): records/members/let-locals are
// STR; only literals, bigint params, and strToNum() are NUM. Every
// arithmetic operand goes through  (= strToNum) so no generic op ever
// sees mixed types.

const FEE_NUM = 997n;   // 0.3% swap fee
const FEE_DEN = 1000n;
const ZERO = 0n;

const POOL0 = '{"ra":"0","rb":"0","ts":"0"}';
const WAL0 = '{"a":"0","b":"0"}';

function pool() {
  return near.storageGet("amm:pool") ?? POOL0;
}

function savePool(p: string): string {
  near.storageSet("amm:pool", p);
  return p;
}

function wallet(who: string) {
  return near.storageGet("amm:w:" + who) ?? WAL0;
}

function saveWallet(who: string, w: string): string {
  near.storageSet("amm:w:" + who, w);
  return w;
}

export function init(): string {
  if ((near.storageGet("amm:init") ?? "0") == "0") {
    near.storageSet("amm:init", "1");
  } else {
    near.abort("already initialized");
  }
  if (near.signerAccountId() == "mallory.test.near") {
    // (no != on strings — invert; the mock's owner is signer-agnostic,
    // this guard just proves the branch exists)
    near.abort("no");
  }
  saveWallet("alice.test.near", jsonSet(jsonSet(WAL0, "a", 2000n), "b", 2000n));
  saveWallet("bob.test.near", jsonSet(jsonSet(WAL0, "a", 500n), "b", 500n));
  savePool(POOL0);
  return "minted 2500/2500";
}

export function addLiquidity(amtA: bigint, amtB: bigint): string {
  let who = near.signerAccountId();
  let w = wallet(who);
  let p = pool();
  let minted = "0";
  if (amtA <= ZERO || amtB <= ZERO) {
    near.abort("zero amount");
  }
  if (p.ts == ZERO) {
    // first LP anchors the share supply at mira', rb'
    if (amtA < amtB) {
      minted = toStr(amtA);
    } else {
      minted = toStr(amtB);
    }
  } else {
    // ratio lock: aIn * rb == bIn * ra (exact integer cross-products)
    if (amtA * p.rb == amtB * p.ra) {
      minted = toStr(p.ts * amtA / p.ra);
    } else {
      near.abort("ratio");
    }
  }
  if (w.a < amtA || w.b < amtB) {
    near.abort("insufficient wallet");
  }
  let w2 = jsonSet(jsonSet(w, "a", w.a - amtA), "b", w.b - amtB);
  saveWallet(who, w2);
  let p2 = jsonSet(
    jsonSet(
      jsonSet(p, "ra", p.ra + amtA),
      "rb", p.rb + amtB,
    ),
    "ts", u128Add(p.ts, minted),
  );
  savePool(p2);
  let lpKey = "amm:lp:" + who;
  let lp = near.storageGet(lpKey) ?? "0";
  near.storageSet(lpKey, toStr(u128Add(lp, minted)));
  return p2;
}

export function removeLiquidity(shares: bigint): string {
  let who = near.signerAccountId();
  let lpKey = "amm:lp:" + who;
  let lp = near.storageGet(lpKey) ?? "0";
  if (shares <= ZERO || shares > lp) {
    near.abort("over-withdraw");
  }
  let p = pool();
  let outA = toStr(p.ra * shares / p.ts);
  let outB = toStr(p.rb * shares / p.ts);
  let w = wallet(who);
  let w2 = jsonSet(jsonSet(w, "a", u128Add(w.a, outA)), "b", u128Add(w.b, outB));
  saveWallet(who, w2);
  near.storageSet(lpKey, toStr(lp - shares));
  let p2 = jsonSet(
    jsonSet(
      jsonSet(p, "ra", u128Sub(p.ra, outA)),
      "rb", u128Sub(p.rb, outB),
    ),
    "ts", p.ts - shares,
  );
  savePool(p2);
  return p2;
}

export function swapExactIn(dir: string, amtIn: bigint): string {
  // dir "a" = pay A get B; "b" = pay B get A
  let who = near.signerAccountId();
  if (amtIn <= ZERO) {
    near.abort("zero swap");
  }
  let p = pool();
  if (p.ts == ZERO || p.ra <= ZERO || p.rb <= ZERO) {
    near.abort("empty pool");
  }
  let rIn: any = "0";
  let rOut: any = "0";
  if (dir == "a") {
    rIn = p.ra;
    rOut = p.rb;
  } else {
    rIn = p.rb;
    rOut = p.ra;
  }
  let out = toStr(rOut * amtIn * FEE_NUM / (rIn * FEE_DEN + amtIn * FEE_NUM));
  // @ts-expect-error — legal lattice compare (STR vs NUM) in the dialect
  if (out <= ZERO) {
    near.abort("dust");
  }
  let w = wallet(who);
  if (dir == "a") {
    if (w.a < amtIn) {
      near.abort("insufficient A");
    }
    saveWallet(who, jsonSet(jsonSet(w, "a", w.a - amtIn), "b", u128Add(w.b, out)));
    let p2 = jsonSet(jsonSet(p, "ra", p.ra + amtIn), "rb", u128Sub(p.rb, out));
    savePool(p2);
    return p2;
  } else {
    if (w.b < amtIn) {
      near.abort("insufficient B");
    }
    saveWallet(who, jsonSet(jsonSet(w, "a", u128Add(w.a, out)), "b", w.b - amtIn));
    let p2 = jsonSet(jsonSet(p, "ra", u128Sub(p.ra, out)), "rb", p.rb + amtIn);
    savePool(p2);
    return p2;
  }
}

export function k(): string {
  let p = pool();
  return toStr(u128Mul(p.ra, p.rb));
}

export function reserves(): string {
  return pool();
}

export function walletOf(who: string): string {
  return wallet(who);
}

export function lpOf(who: string): string {
  return near.storageGet("amm:lp:" + who) ?? "0";
}
