// ── loopSwap AMM: N ping-pong swaps inside ONE call ──
// Same constant-product math as fixtures/amm.ts, plus loopSwap(n):
// pool+wallet members held in u128-pure locals, pure math per
// iteration, ONE wallet+pool write at the end.
// Fuel: init mints 30000/30000 to the caller.

const FEE_NUM = 997n;   // 0.3% swap fee
const FEE_DEN = 1000n;
const ZERO = 0n;
const ONE = 1n;
const AMT_IN = 10n;

const POOL0 = '{"ra":"0","rb":"0","ts":"0"}';
const WAL0 = '{"a":"0","b":"0"}';

function pool() {
  return near.storageGet("amm:pool") ?? POOL0;
}

function savePool(p: any): string {
  near.storageSet("amm:pool", p);
  return p;
}

function wallet(who: string) {
  return near.storageGet("amm:w:" + who) ?? WAL0;
}

function saveWallet(who: string, w: any): string {
  near.storageSet("amm:w:" + who, w);
  return w;
}

export function init(): string {
  saveWallet(
    near.signerAccountId(),
    jsonSet(jsonSet(WAL0, "a", 30000n), "b", 30000n),
  );
  saveWallet("alice.test.near", jsonSet(jsonSet(WAL0, "a", 2500n), "b", 2500n));
  saveWallet("bob.test.near", jsonSet(jsonSet(WAL0, "a", 2500n), "b", 2500n));
  savePool(POOL0);
  return "minted 30000/30000";
}

export function addLiquidity(amtA: bigint, amtB: bigint): string {
  let who = near.signerAccountId();
  let w = wallet(who);
  let p = pool();
  if (amtA <= ZERO || amtB <= ZERO) {
    near.abort("zero add");
  }
  if (w.a < amtA || w.b < amtB) {
    near.abort("insufficient balance");
  }
  let ts: any = u128Add(p.ts, u128Mul(amtA, amtB));
  saveWallet(who, jsonSet(jsonSet(w, "a", u128Sub(w.a, amtA)), "b", u128Sub(w.b, amtB)));
  savePool(jsonSet(jsonSet(p, "ra", u128Add(p.ra, amtA)), "rb", u128Add(p.rb, amtB)));
  return "added; total shares " + toStr(ts);
}

// THE EXPERIMENT: n ping-pong swaps in ONE call.
export function loopSwap(n: bigint): string {
  let who = near.signerAccountId();
  if (n <= ZERO) {
    near.abort("zero n");
  }
  let p = pool();
  let w = wallet(who);
  let ra: any = p.ra;
  let rb: any = p.rb;
  let wa: any = w.a;
  let wb: any = w.b;
  let i: bigint = ZERO;
  let dirA: boolean = true;
  let lastOut: any = ZERO;
  let rounds: any = ZERO;
  while (i < n) {
    let rIn: any = dirA ? ra : rb;
    let rOut: any = dirA ? rb : ra;
    let out: any = rOut * AMT_IN * FEE_NUM / (rIn * FEE_DEN + AMT_IN * FEE_NUM);
    if (out <= ZERO) {
      near.abort("dust");
    }
    if (dirA) {
      if (wa < AMT_IN) {
        near.abort("insufficient A");
      }
      wa = u128Sub(wa, AMT_IN);
      wb = u128Add(wb, out);
      ra = u128Add(ra, AMT_IN);
      rb = u128Sub(rb, out);
    } else {
      if (wb < AMT_IN) {
        near.abort("insufficient B");
      }
      wb = u128Sub(wb, AMT_IN);
      wa = u128Add(wa, out);
      rb = u128Add(rb, AMT_IN);
      ra = u128Sub(ra, out);
    }
    lastOut = out;
    rounds = u128Add(rounds, ONE);
    i = i + ONE;
    dirA = !dirA;
  }
  saveWallet(who, jsonSet(jsonSet(w, "a", wa), "b", wb));
  savePool(jsonSet(jsonSet(p, "ra", ra), "rb", rb));
  return "swaps=" + toStr(rounds) + " a=" + toStr(wa) + " b=" + toStr(wb) +
    " k=" + toStr(u128Mul(ra, rb)) + " lastOut=" + toStr(lastOut);
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
