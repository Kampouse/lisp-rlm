// ── Surface Tour 2 — every under-tested TS surface item, one file ──
//
// Complements the NEAR API Tour playground example: this fixture exercises
// host calls and syntax with ZERO prior TS-corpus coverage (2026-09-01
// audit): randomSeed, input, usedGas, storageUsage, attachedDepositHigh,
// signerAccountPk, depositGte, iterPrefix/iterNext, storeU128/loadU128,
// transferU128, jsonGetArr, string methods, for...of, i++, +=, arrows,
// template literals.
//
// Exotic crypto (keccak512/ripemd160/p256/ecrecover/altBn128/bls12381)
// lives in surface_tour2_exotic.ts — those hosts aren't in the mock.

const S = "hello lisp-rlm";

export function strMethods(): string {
  // string method surface — 0 prior uses in the whole TS corpus
  // (.length/.indexOf are NUM → stringified via template literal)
  let acc = "";
  if (S.startsWith("hello")) { acc += "S"; }
  if (S.endsWith("rlm")) { acc += "E"; }
  if (S.includes("lisp")) { acc += "I"; }
  acc += `(${S.indexOf("lisp")})`;    // (6)
  acc += S.charAt(1);                 // e
  acc += S.slice(0, 5);               // hello
  acc += S.concat("!");               // hello lisp-rlm!
  return `${acc}[${S.length}]`;
}

export function syntaxTour(): string {
  // for...of + i++ + += — claimed in d.ts, previously unused
  let items = ["a", "b", "c"];
  let out = "";
  for (const x of items) {
    // NOTE: `out + x` (sym ⊕ sym) lowers to num-only (+) — the frontend
    // can't see str-ness through plain identifiers. Use .concat (or a
    // stringy literal operand) — audit 2026-09-01.
    out = out.concat(x);
  }
  let n = 0;
  for (let i = 0; i < 3; i++) {
    n += 10;
  }
  n++;
  return `${out}:${n}`;   // abc:31
}

export function ctx(): string {
  // context host calls — all previously TS-untested
  let seed = near.randomSeed();          // 32B hex str in mock
  let pk = near.signerAccountPk();       // hex str
  let gas = near.usedGas();
  let prepaid = near.prepaidGas();
  let usage = near.storageUsage();
  let depHi = near.attachedDepositHigh();
  let ok = near.depositGte(0, 0);        // 0-deposit call: 1
  return `${seed.length}:${pk.length}:${gas >= 0}:${prepaid > 0}:${usage >= 0}:${depHi >= 0}:${ok == 1}`;
}

export function inputEcho(): string {
  // raw tx input — returns the full args JSON the mock passes
  return near.input();
}

export function iterProbe(): string {
  // storage iteration — mock hosts are noops; must not trap
  let a = near.iterPrefix("st2:");
  let b = near.iterNext(a);
  return `iter:${b}`;
}

export function numStorage(): string {
  // u128 numeric storage discipline path
  near.storeU128("st2:u128", "340282366920938463463374607431768211455");
  let back = near.loadU128("st2:u128");
  return back;
}

export function money(): string {
  // u128 transfer + batch account creation (promise noops in mock)
  let p = near.promiseBatchCreate(near.currentAccountId());
  near.promiseBatchActionCreateAccount(p);
  near.promiseBatchActionTransfer(p, "0");
  near.transferU128(near.currentAccountId(), "0");
  return "money-ok";
}

export function jsonArr(): string {
  let arr = near.jsonArr("ks");
  let first = arr[0];
  return `jsonArr:${first}`;
}
