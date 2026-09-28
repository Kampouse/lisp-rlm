// Loop-carried state + in-loop return semantics — regression suite for the
// 2026-09-11 fixes:
//
// 1. HOIST-ORDER FIX: while-body `let/const` declarations are rewritten to
//    per-iteration (set! x e) re-inits. The old lowering inserted each set!
//    at begin-position, REVERSING them — `const ai = e1; let C = e2(reads ai);`
//    ran C's init BEFORE ai's, seeing nil→0 on iteration 1 and a stale value
//    on later iterations. The fp254 repro called it "values vanish when
//    reassigned inside nested whiles" (limbs came out 65,49, not 100,49).
//
// 2. NESTED-RETURN FIX: a `return` inside a nested while only stopped the
//    inner loop (loop-local __wl_* flags are shadowed per level, the value
//    was discarded by the enclosing body). Returns now also set the
//    function-level __fn_done/__fn_res flags; conds stop on them and
//    post-return statements are guarded — storage writes after a nested
//    return never commit.

function limb0(x: number): number { return x % 65536; }
function limb1(x: number): number { return (x / 65536) % 65536; }
function limb2(x: number): number { return (x / 65536 / 65536) % 65536; }
function limb(x: number, i: number): number {
  if (i == 0) { return limb0(x); }
  if (i == 1) { return limb1(x); }
  return limb2(x);
}

// ── THE fp254 reproducer: inner-loop array writes in nested whiles ──
// Schoolbook 2x2 limb mul with the accumulator IN an array. Correct limbs
// verified against a python simulation of this exact code.
function twoArr(m: string[], n: string[]): string[] {
  const t = strSplit("0,0", ",");
  let i = 0;
  while (i < 2) {
    const ai = strToNum(m[i]);           // dynamic index on param array
    let C = strToNum(t[0]) + ai * strToNum(n[0]);
    let j = 0;
    while (j < 2) {
      C = C + ai * strToNum(n[j]) + strToNum(t[j]);
      t[j] = toStr(C % 65536);           // inner-loop array writes
      C = C / 65536;
      j = j + 1;
    }
    i = i + 1;
  }
  return t;
}

export function repro_arr(): string {
  const t = twoArr(strSplit("3,4", ","), strSplit("5,7", ","));
  return t[0] + "," + t[1]; // 100,49
}

export function repro_arr2(): string {
  const t = twoArr(strSplit("10,20", ","), strSplit("30,40", ","));
  return t[0] + "," + t[1]; // 2400,1200
}

export function repro_arr3(): string {
  const t = twoArr(strSplit("65535,1", ","), strSplit("1,1", ","));
  return t[0] + "," + t[1]; // 65534,2
}

// ── same algorithm, NATIVE int arrays — no strSplit/strToNum/toStr ──
// Arrays are tagged-value vectors (TAG_ARRAY heap blocks, elements are
// tagged i64s): int literals, push, dynamic-index reads/writes all work.
// Storage is the only string boundary.
function twoArrN(): number[] {
  const m = [3, 4];
  const n = [5, 7];
  const t = [0, 0];
  let i = 0;
  while (i < 2) {
    const ai = m[i];
    let C = t[0] + ai * n[0];
    let j = 0;
    while (j < 2) {
      C = C + ai * n[j] + t[j];
      t[j] = C % 65536;
      C = C / 65536;
      j = j + 1;
    }
    i = i + 1;
  }
  return t;
}

export function repro_arr_int(): string {
  const t = twoArrN();
  return toStr(t[0]) + "," + toStr(t[1]); // 100,49 — same as the string version
}

// ── CIOS-shape multiply: nested whiles, loop-carried scalars ──
// a,b < 2^48 (3x16-bit limbs); returns the 5 result limbs decimal-packed.
export function mul3(): string {
  const a = near.jsonGetInt("a") ?? 0;
  const b = near.jsonGetInt("b") ?? 0;
  if (a < 0) { return "-1"; }
  if (b < 0) { return "-2"; }
  let r0 = 0; let r1 = 0; let r2 = 0; let r3 = 0; let r4 = 0;
  let i = 0;
  while (i < 3) {
    let j = 0;
    while (j < 3) {
      const p = limb(a, i) * limb(b, j);
      if (i + j == 0) { r0 = r0 + p; }
      if (i + j == 1) { r1 = r1 + p; }
      if (i + j == 2) { r2 = r2 + p; }
      if (i + j == 3) { r3 = r3 + p; }
      if (i + j == 4) { r4 = r4 + p; }
      j = j + 1;
    }
    i = i + 1;
  }
  let c = 0;
  const n0 = (r0 + c) % 65536; c = ((r0 + c) - n0) / 65536;
  const n1 = (r1 + c) % 65536; c = ((r1 + c) - n1) / 65536;
  const n2 = (r2 + c) % 65536; c = ((r2 + c) - n2) / 65536;
  const n3 = (r3 + c) % 65536; c = ((r3 + c) - n3) / 65536;
  const n4 = r4 + c;
  return toStr(n0 + n1 * 10000 + n2 * 100000000 + n3 * 1000000000000 + n4 * 100000000000000);
}

// ── nested returns ──

// THE regression: return inside a nested while must return 77, not fall
// through to `return total` (before: loops finished and returned total).
export function nested_return(): string {
  let total = 0;
  let i = 0;
  while (i < 5) {
    let j = 0;
    while (j < 5) {
      if (i == 2) {
        if (j == 3) { return "77"; }
      }
      j = j + 1;
    }
    total = total + 1;
    i = i + 1;
  }
  return toStr(total);
}

// return inside a `for` nested in a `while` (with_return For arm)
export function for_nested_return(): string {
  let acc = 0;
  let i = 0;
  while (i < 4) {
    for (let k2 = 0; k2 < 4; k2++) {
      if (i == 2) {
        if (k2 == 1) { return "55"; }
      }
      acc = acc + 1;
    }
    i = i + 1;
  }
  return toStr(acc);
}

// three-level nesting: accumulation is loop-carried through both levels
export function three_deep(): string {
  let total = 0;
  let a = 0;
  while (a < 2) {
    let b = 0;
    while (b < 2) {
      let c = 0;
      while (c < 2) {
        total = total + 1;
        c = c + 1;
      }
      b = b + 1;
    }
    a = a + 1;
  }
  return toStr(total);
}

// a nested return must NOT run later statements — the storage write after
// the loop must never commit when the return fires.
export function skip_write(): string {
  near.storageSet("lr:sk", "before");
  let i = 0;
  while (i < 3) {
    let j = 0;
    while (j < 3) {
      if (i == 1) {
        if (j == 1) { return "99"; }
      }
      j = j + 1;
    }
    i = i + 1;
  }
  near.storageSet("lr:sk", "after");
  return "0";
}

export function read_sk(): string {
  return near.storageGet("lr:sk") ?? "";
}

// early return in a mid-function if (no loop) — deep-return M2 activation
export function if_return_early(): string {
  const x = near.jsonGetInt("x") ?? 0;
  if (x == 1) { return "7"; }
  let y = x + 100;
  return toStr(y);
}

// break in an inner loop: outer body statements still run (correct TS),
// carry persists across iterations
export function carry_over_break(): string {
  let carry = 0;
  let i = 0;
  while (i < 4) {
    let j = 0;
    while (j < 8) {
      if (carry > 5) { break; }
      carry = carry + 1;
      j = j + 1;
    }
    i = i + 1;
  }
  return toStr(carry);
}