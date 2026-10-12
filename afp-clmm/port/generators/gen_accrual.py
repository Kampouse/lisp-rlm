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
(define (bal u) (sget (str-cat "a:bal:" u) "0"))
(define (charged u) (sget (str-cat "a:charged:" u) "0"))
(define (credit u a) (sput (str-cat "a:bal:" u) (add (bal u) a)))
(define (debit u a)
  (if (ult (bal u) a) (fail "ERR_FUNDS") "ok")
  (sput (str-cat "a:bal:" u) (sub (bal u) a)))
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
function debit(u: string, a: string): void {
  if (u128.lt(bal(u), a)) fail("ERR_FUNDS");
  sput("a:bal:" + u, u128.sub(bal(u), a));
}
function u128MulDiv(a: string, b: string, d: string): string {
  // (a * b) / d, truncating; d != 0
  if (d === "0") fail("ERR_DIV0");
  const A = BigInt(a), B = BigInt(b), D = BigInt(d);
  return ((A * B) / D).toString();
}
function u128Shr64(a: string): string {
  // a div 2^64  (exact transcription of `div 2^64` in accruef)
  return (BigInt(a) >> 64n).toString();
}
"""


def lisp_src():
    return r'''; Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
; accruef Lf c []       = 0
; accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
; Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).

(define (sget k) (if (defined? k) k "0"))
''' + LISP_HELPERS + r'''
; owed fold: checkpoint-threaded sum of (g - c) * Lf g div 2^64
; gs: "g1,g2,..." per-op growth deltas; Lf: "rate1,rate2,..." (per-op);
; c: starting checkpoint (must be <= every g, else ERR_ORDER)
; -> owed string, then checkpoint advances to last g
(define (accrue gs-lfs c)
  (let loop ((gs (map str->num (str-split (car gs-lfs) ",")))
             (ls (map str->num (str-split (cadr gs-lfs) ",")))
             (ck (str->num c))
             (acc 0))
    (if (null? gs) acc
        (begin
          (if (< (car gs) ck) (fail "ERR_ORDER") "ok")
          (loop (cdr gs) (cdr ls) (car gs)
                (+ acc (div-floor64 (- (car gs) ck) (car ls)))))))

; entry: accrue { "gs": "g1,g2", "ls": "l1,l2", "c": "ck" }
; charges owed into caller's cell, preserves invariant owed <= charged
(define (handle arg)
  (let* ((gs (get-arg arg "gs"))
         (ls (get-arg arg "ls"))
         (c  (get-arg arg "c"))
         (u  (get-arg arg "user"))
         (owed (accrue (list gs ls) c))
         (new-charged (+ (str->num (charged u)) owed)))
    (credit u (num->str owed))
    (sput (str-cat "a:charged:" u) (num->str new-charged))
    (num->str owed)))

(define (get-arg arg k)
  (let ((v (assoc k arg)))
    (if v (cdr v) (fail "ERR_ARG"))))
'''


def ts_src():
    return r'''// Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
// accruef Lf c []       = 0
// accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
// Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).
''' + TS_HELPERS + r'''
// owed fold: checkpoint-threaded sum of (g - c) * Lf g div 2^64.
// gs: per-op growth deltas (charged book entries, Aggregate_Book);
// ls: per-op liquidity Lf; c: starting checkpoint (<= every g, else ERR_ORDER).
// Returns owed as string; checkpoint advances to last g.
function accrue(gs: string[], ls: string[], c: string): string {
  let ck = BigInt(c);
  let acc = 0n;
  for (let i = 0; i < gs.length; i++) {
    const g = BigInt(gs[i]);
    if (g < ck) fail("ERR_ORDER");
    acc += BigInt(divFloor64((g - ck).toString(), ls[i]));
    ck = g;
  }
  return acc.toString();
}

// entry: accrue { gs: "g1,g2,...", ls: "l1,l2,...", c: "ck", user: "alice" }
// charges owed into the caller's cell; preserves owed <= charged.
export function accrue_entry(arg: string): string {
  const o = JSON.parse(arg);
  const gs: string[] = String(o.gs).split(",");
  const ls: string[] = String(o.ls).split(",");
  if (gs.length !== ls.length) fail("ERR_LEN");
  const u = String(o.user);
  const owed = accrue(gs, ls, String(o.c));
  credit(u, owed);
  sput("a:charged:" + u, u128.add(charged(u), owed));
  return owed;
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
