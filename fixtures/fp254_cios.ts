// BN254 field multiply — 16-limb (16-bit) CIOS Montgomery, ALL state in
// native int arrays + locals. Zero storage writes in the hot path.
// out = a·b·R⁻¹ mod p  (feed Montgomery-form operands for field-mul).
// Operands arrive as "l0,l1,...,l15" decimal limb strings (tx-args
// boundary); parsed once at entry, then pure ints.



// CIOS core: a, b are 16-limb int arrays (each limb < 2^16); returns
// out = a·b·R⁻¹ mod p as a fresh 16-limb array. Pure locals/arrays.
function ciosMul(a: number[], b: number[]): number[] {
  const NP0 = 25481;
  const P = [64839, 55420, 35862, 15392, 51853, 26737, 27281, 38785, 22621, 33153, 17846, 47184, 41001, 57649, 20082, 12388];
  let t = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let i = 0;
  while (i < 16) {
    let C = 0;
    let j = 0;
    while (j < 16) {
      const sum = t[j] + a[j] * b[i] + C;
      C = sum / 65536;
      t[j] = sum % 65536;
      j = j + 1;
    }
    const s16 = t[16] + C;
    C = s16 / 65536;
    t[16] = s16 % 65536;
    t[17] = C;
    const m = (t[0] * NP0) % 65536;
    const s0 = t[0] + m * P[0];
    C = s0 / 65536;
    let j2 = 1;
    while (j2 < 16) {
      const sj = t[j2] + m * P[j2] + C;
      C = sj / 65536;
      t[j2 - 1] = sj % 65536;
      j2 = j2 + 1;
    }
    const s16b = t[16] + C;
    C = s16b / 65536;
    t[15] = s16b % 65536;
    t[16] = t[17] + C;
    i = i + 1;
  }
  let d = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let borrow = 0;
  let j3 = 0;
  while (j3 < 16) {
    const dd = t[j3] - P[j3] - borrow;
    if (dd < 0) {
      borrow = 1;
      d[j3] = dd + 65536;
    } else {
      borrow = 0;
      d[j3] = dd;
    }
    j3 = j3 + 1;
  }
  if (t[16] >= borrow) {
    let j4 = 0;
    while (j4 < 16) {
      t[j4] = d[j4];
      j4 = j4 + 1;
    }
  }
  return t;
}

export function mulmod(): string {
  const as = near.jsonGetStr("a") ?? "";
  const bs = near.jsonGetStr("b") ?? "";
  if (strLength(as) == 0) { return "ERR:empty"; }
  if (strLength(bs) == 0) { return "ERR:empty"; }
  const A = strSplit(as, ",");
  const B = strSplit(bs, ",");
  let a = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let b = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let k = 0;
  while (k < 16) {
    a[k] = strToNum(A[k]);
    b[k] = strToNum(B[k]);
    k = k + 1;
  }
  let bad = 0;
  let g = 0;
  while (g < 16) {
    if (a[g] > 65535) { bad = 1; }
    if (b[g] > 65535) { bad = 1; }
    g = g + 1;
  }
  if (bad == 1) { return "ERR:limb"; }
  const t = ciosMul(a, b);
  let out = toStr(t[0]);
  let k2 = 1;
  while (k2 < 16) {
    out = strCat(out, ",", toStr(t[k2]));
    k2 = k2 + 1;
  }
  return out;
}

// parse+serialize roundtrip WITHOUT the CIOS loops — measuring this lets
// callers decompose: (mulmod - roundtrip) = pure loop cost per mul.
export function roundtrip(): string {
  const as = near.jsonGetStr("a") ?? "";
  const bs = near.jsonGetStr("b") ?? "";
  if (strLength(as) == 0) { return "ERR:empty"; }
  if (strLength(bs) == 0) { return "ERR:empty"; }
  const A = strSplit(as, ",");
  const B = strSplit(bs, ",");
  let a = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let k = 0;
  while (k < 16) {
    a[k] = strToNum(A[k]);
    k = k + 1;
  }
  let out = toStr(a[0]);
  let k2 = 1;
  while (k2 < 16) {
    out = strCat(out, ",", toStr(a[k2]));
    k2 = k2 + 1;
  }
  return out;
}

// double-mul: parse once, run the FULL CIOS twice, serialize once —
// (mulTwice - mulmod) = pure CIOS loop cost, no boundary noise.
export function mulTwice(): string {
  const as = near.jsonGetStr("a") ?? "";
  const bs = near.jsonGetStr("b") ?? "";
  const A = strSplit(as, ",");
  const B = strSplit(bs, ",");
  let a = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let b = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
  let k = 0;
  while (k < 16) {
    a[k] = strToNum(A[k]);
    b[k] = strToNum(B[k]);
    k = k + 1;
  }
  let r1 = ciosMul(a, b);
  let r2 = ciosMul(a, r1);
  let out = toStr(r2[0]);
  let k2 = 1;
  while (k2 < 16) {
    out = strCat(out, ",", toStr(r2[k2]));
    k2 = k2 + 1;
  }
  return out;
}
