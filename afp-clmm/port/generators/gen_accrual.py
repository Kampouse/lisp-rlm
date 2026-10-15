#!/usr/bin/env python3
"""Generate the accrual contract from the op table.

Twin sources (lisp + TS), both transcribing `accruef` from
isabelle/Aggregate_Accrual.thy exactly:

    accruef Lf c []       = 0
    accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs

Per-op growth deltas are the charged book (Aggregate_Book); owed is the
fold over them. Theorem `accrue_within_charged_book`: owed <= charged.
"""
import json
import os
import sys

# Layout: port/contracts/<group>/<dirname>/ (src/ + colocated ts/)
DEFAULT_ROOT = os.path.join(os.path.dirname(__file__), os.pardir, "contracts")

ACCRUAL_ACCT = "accrual.test.near"

LISP_HELPERS = """(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))
(define (sget k) (default (near/storage_get k) "0"))
(define (sput k v) (near/storage_set k v))
(define (fail m) (near/log m) (near/abort m))
(define (bal u) (sget (str-cat "a:bal:" u)))
(define (charged u) (sget (str-cat "a:charged:" u)))
(define (credit u a) (sput (str-cat "a:bal:" u) (add (bal u) a)))
(define (debit u a)
  (if (ult (bal u) a) (fail "ERR_FUNDS") "ok")
  (sput (str-cat "a:bal:" u) (sub (bal u) a)))
; div-floor64 a b = (a * b) div 2^64, truncating (floor on nat).
; Mirrors Isabelle div_floor64: owed side rounds DOWN, fail-closed.
; b = 0 yields 0 (accruef semantics), no error.
(define (div-floor64 a b) (u128/muldiv a b "18446744073709551616"))
"""

TS_HELPERS = """function sget(k: string): string {
  return near.storageGet(k) ?? "0";
}
function sput(k: string, v: string): void {
  near.storageSet(k, v);
}
function fail(m: string): void {
  near.log(m);
  near.abort(m);
}
function bal(u: string): string {
  return sget("a:bal:" + u);
}
function charged(u: string): string {
  return sget("a:charged:" + u);
}
function credit(u: string, a: string): void {
  sput("a:bal:" + u, u128.add(bal(u), a));
}
// divFloor64 a b = (a * b) div 2^64, truncating (floor on nat).
// Mirrors Isabelle div_floor64: owed side rounds DOWN, fail-closed.
// Same 2^64 literal as the LISP twin and the .thy.
function divFloor64(a: string, b: string): string {
  return u128MulDiv(a, b, "18446744073709551616");
}
"""


def lisp_src():
    return r'''; Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
; accruef Lf c []       = 0
; accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
; Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).

''' + LISP_HELPERS + r'''
; Cons step of the owed fold: accruef Lf c (g # gs) instantiated at the
; head -- owed(g,c) = (g - c) * Lf g div 2^64. The checkpoint a:ck:<u>
; carries the fold's c across calls (monotone; g < c aborts ERR_ORDER,
; exactly accruef's precondition). One exported step = one equation
; instance; the scenario's step sequence IS the list gs.
; One call per let* binding (compiler call-sequence discipline: sibling
; calls in one primitive application clobber -- see probe contract).
(define (accrue_step)
  (let* ((g (default (near/json_get_str "g") "0"))
         (l (default (near/json_get_str "l") "0"))
         (u (near/predecessor_account_id))
         (ck-key (str-cat "a:ck:" u))
         (c (default (near/storage_get ck-key) "0"))
         (lt (ult g c))
         (guard (if lt (fail "ERR_ORDER") "ok"))
         (d (u128/sub g c))
         (term (div-floor64 d l)))
    (begin
      (credit u term)
      (let ((ch (charged u)))
        (let ((ch2 (u128/add ch term)))
          (begin
            (sput (str-cat "a:charged:" u) ch2)
            (sput ck-key g)
            (near/return term)))))))

(export "accrue_step" accrue_step)
'''


def ts_src():
    return r'''// Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
// accruef Lf c []       = 0
// accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
// Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).
''' + TS_HELPERS + r'''
// Cons step of the owed fold: owed(g, c) = (g - c) * Lf g div 2^64.
// Checkpoint a:ck:<u> carries the fold's c across calls (monotone;
// g < c aborts ERR_ORDER = accruef's precondition). Scenario steps are
// the list gs. String layer only (u128.*), like the LISP twin.
export function accrueStep(): string {
  const g = near.jsonGetStr("g") ?? "0";
  const l = near.jsonGetStr("l") ?? "0";
  const u = near.predecessorAccountId();
  const ckKey = "a:ck:" + u;
  const c = sget(ckKey);
  if (u128.lt(g, c)) fail("ERR_ORDER");
  const d = u128.sub(g, c);
  const term = divFloor64(d, l);
  credit(u, term);
  sput("a:charged:" + u, u128.add(charged(u), term));
  sput(ckKey, g);
  return term;
}
'''


def write(path, src):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(src)
    print(f"wrote {path} ({len(src)} B)")


def near_json(name, src, out):
    return json.dumps({"name": name, "src": src, "account": ACCRUAL_ACCT,
                       "network": "local", "output": out}, indent=1) + "\n"


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_ROOT
    base = os.path.join(root, "accrual", "accrual")
    write(os.path.join(base, "src", "main.lisp"), lisp_src())
    write(os.path.join(base, "ts", "main.ts"), ts_src())
    nj = near_json("accrual", "src/main.lisp", "target/accrual.wasm")
    with open(os.path.join(base, "near.json"), "w") as f:
        f.write(nj)
    print(f"wrote {os.path.join(base, 'near.json')}")
    with open(os.path.join(base, "ts", "near.json"), "w") as f:
        f.write(json.dumps({"name": "accrual-ts", "src": "main.ts",
                            "output": "target/accrual-ts.wasm"}) + "\n")
    print(f"wrote {os.path.join(base, 'ts', 'near.json')}")


if __name__ == "__main__":
    main()
