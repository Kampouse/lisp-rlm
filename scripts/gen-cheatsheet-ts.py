#!/usr/bin/env python3
"""Regenerate data/rlm/dream/cheatsheet-ts.txt — the TS-surface cheatsheet
the @ts arm sees in every prompt (same role as cheatsheet.txt for the Lisp
arm, but TS-syntax truth instead of Lisp builtins).

The surface set below is enforced by tests/ts_math_array_vm.rs (lowering +
all-backend compile) and tests/ts_surface_dts_parity.rs (d.ts ↔ frontend
sync). Edit the surface → update those tests + the d.ts FIRST, then this.
"""
import os
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
dst = os.path.join(REPO, "data/rlm/dream/cheatsheet-ts.txt")

BODY = """TS SURFACE CHEATSHEET (exact forms that work):

STORE RESULTS / FINISH:
  rlm_set("answer", <expr>);      // store the task answer
  rlm_set("Final", true);         // finish — only when verified

MATH (integer):
  Math.abs(x)  Math.max(a, b)  Math.min(a, b)
  Math.pow(a, b)                // exponentiation
  Math.sqrt(x)  Math.floor(x)  Math.ceil(x)  Math.round(x)
  // anything else in Math.* is a HARD ERROR — do not use it

ARRAYS:
  const a: number[] = [1, 2, 3];
  a[i]          // index read
  a.length      // element count (strings: use strLength(s), NOT s.length)
  a.push(v)     // functional — rebinds: a = a.push(v) style update is automatic
  a.map(x => x * 2)              // arrow callbacks allowed
  a.filter(x => x > 1)
  a.reduce((acc, x) => acc + x, 0)
  a.join(",")

STRINGS:
  strSplit(s, ","), strJoin(",", parts), strLength(s),
  strSubstring(s, i, j), strIndexOf(s, sub), strContains(s, sub)

CONTROL:
  function f(x: number): number { return x; }   // recursion works
  const / let, if / else, template literals `...${x}...`, ternary x ? y : z
  for (let i = 0; i < n; i = i + 1) { ... }   // counted loops
  while (i < n) { i = i + 1; }                // while loops
  for (const x of arr) { ... }                // array iteration
  // top-level loops: NO break/continue/return inside (hard error) —
  // stop with a flag: for (let i=0; i<n; i=i+1) { if (done) { i = n; } }
  // inside exported functions break/continue/return all work

LOGGING:
  console.log(...)   // visible in the execution log you receive back

FORBIDDEN (hard errors):
  imports, classes, async/await, destructuring, optional chaining (?.),
  spread (...), try/catch, switch, Math.* beyond the list above

COMPLETE EXAMPLE:
  const xs: number[] = [3, 1, 4, 1, 5];
  const total: number = xs.reduce((a: number, b: number): number => a + b, 0);
  const answer: number = Math.pow(2, xs.length) + Math.floor(total / 3);
  rlm_set("answer", answer);
  rlm_set("Final", true);
"""

if __name__ == "__main__":
    open(dst, "w").write(BODY)
    print(f"cheatsheet-ts: {len(BODY.splitlines())} lines -> {dst}")
