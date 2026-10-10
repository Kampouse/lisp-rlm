#!/usr/bin/env python3
"""LEG 4 — arbitrary N-pool composition (N=3) on a SHARED grid.

Model & optimality (the AFP entry only defines PAIRWISE joins; the N-ary
composition is our generalization, verified pin+sweep like everything else):
  * Three pools on grid [1e9, 2e9, 4e9], SAME fee phi = 3e15/1e18
    (heterogeneous fees were leg 3; composition is this leg):
      P0 = pa  liq gen.LA  [1e23, 6e22] -- imported from gen.py, untouched
      P1 = n1  liq [3e22, 8e22]   -- new
      P2 = n2  liq [5e22, 3e22]   -- new
  * Net out is CONCAVE in y (marginal rate (1-phi)/p^2 falls with price),
    so equal-fee optimum = EQUALIZE ENDING PRICE p* across pools:
    p* solves sum_k qgross_k(p*) = y  (leg-2 theorem, N-pool form).
    y_k = min(cap_k, qgross_k(p*)).  splt3 finds p* by FLAT UNROLLED
    binary search (48 storage round-trips, no nesting); qgross sums are
    muldiv-free exact products.
  * Verification: no legal split can beat the chain allocation beyond
    VIOLWIN = 2*N out-units (per-pool per-cell muldiv floor dust).
    Sweeps: FULL fraction grid 33^3 = 39304 splits + 20k random splits,
    zero violations expected.
Run:  python3 gen3.py   (writes n1/, n2/, splt3/src/main.lisp, pins3.json)
"""
import json, os, random, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen  # HELPERS, Qceil, NUM, DEN, pool_src, swap, verify_str

BASE = HERE
G3 = [10**9, 2 * 10**9, 4 * 10**9]
W3 = [G3[i + 1] - G3[i] for i in range(2)]
LIQS = [list(gen.LA),                   # pa -- IMPORTED, never re-typed
        [3 * 10**22, 8 * 10**22],      # n1
        [5 * 10**22, 3 * 10**22]]      # n2
QQ = [[gen.Qceil(l) for l in L] for L in LIQS]
CAPS = [sum(QQ[k][c] * W3[c] for c in range(2)) for k in range(3)]
ACCTS = ("pa.clmm.test.near", "n1.clmm.test.near", "n2.clmm.test.near")
N = 3


def _derive_ys():
    total = sum(CAPS)
    return {
        "under": CAPS[0] // 2,                    # p* inside cell 0
        "two": CAPS[0] + CAPS[1] // 2,            # p* inside cell 1, n2 dry
        "star": CAPS[0] + CAPS[1] + CAPS[2] // 2,  # p* inside cell 1, all wet
        "over": total + total // 8,               # beyond total cap: clamped
    }


YS3 = _derive_ys()


def qgross(k, p):
    """Gross units pool k consumes walking 1e9 -> p (exact integers)."""
    tot = 0
    for c in range(2):
        if G3[c] >= p:
            break
        tot += QQ[k][c] * (min(p, G3[c + 1]) - G3[c])
    return tot


def alloc_chain(y):
    """EXACT mirror of splt3's on-chain allocation (same integer floors):
    binary search p*, 48 iters, lo/hi ints on the grid scale."""
    lo, hi = G3[0], G3[-1]
    for _ in range(48):
        mid = (lo + hi) // 2
        if sum(qgross(k, mid) for k in range(N)) < y:
            lo = mid + 1
        else:
            hi = mid
    out, rem = [], y
    for k in range(N):
        take = min(qgross(k, hi), rem)
        out.append(take)
        rem -= take
    return out


def mswap(k, y):
    return gen.swap(G3, LIQS[k], QQ[k], G3[0], y)[0]


def net_of(vec):
    return sum(mswap(k, vec[k]) for k in range(N))


# ── oracle ──────────────────────────────────────────────────────────────────
def oracle():
    pins = {"G3": G3, "CAPS": CAPS,
            "pools": [{"acct": ACCTS[k], "liq": LIQS[k], "Q": QQ[k],
                       "cap": CAPS[k]} for k in range(N)]}
    VIOLWIN = 2 * N  # per-pool per-cell floor dust, out-units

    print("# caps: " + " ".join(f"{ACCTS[k].split('.')[0]}={CAPS[k]}"
                                for k in range(N)) + f"  total={sum(CAPS)}")
    for tag in ("under", "two", "star", "over"):
        y = YS3[tag]
        vec = alloc_chain(y)
        n = net_of(vec)
        pins[tag] = {"y": y, "vec": vec, "net": n,
                     "excess": max(0, y - sum(vec))}
        print(f"[{tag:5}] y={y} alloc={[int(v) for v in vec]} net={n} "
              f"excess={pins[tag]['excess']}")

    # sweep 1: FULL fraction grid 33^3 (cache optimum per distinct total)
    R = 33
    cache, worst, worst_v, cnt = {}, None, None, 0
    for a in range(R):
        for b in range(R):
            for c in range(R):
                v = [YS3["star"] * a // R, YS3["star"] * b // R,
                     YS3["star"] * c // R]
                tot = sum(v)
                if tot not in cache:
                    cache[tot] = net_of(alloc_chain(tot))
                gap = net_of(v) - cache[tot]
                cnt += 1
                if worst is None or gap > worst:
                    worst, worst_v = gap, v
    assert worst <= VIOLWIN, f"SWEEP VIOLATION {worst} at {worst_v}"
    print(f"[sweep-grid] {cnt} splits / {len(cache)} totals, worst gap = "
          f"{worst} (<= VIOLWIN {VIOLWIN})")

    # sweep 2: 20k random continuous splits
    random.seed(20261009)
    worst2, cnt2 = None, 0
    for _ in range(20000):
        r = [random.random(), random.random(), random.random()]
        s = sum(r)
        v = [int(YS3["star"] * x / s) for x in r]
        tot = sum(v)
        if tot not in cache:
            cache[tot] = net_of(alloc_chain(tot))
        gap = net_of(v) - cache[tot]
        cnt2 += 1
        if worst2 is None or gap > worst2:
            worst2 = gap
    assert worst2 <= VIOLWIN, f"SWEEP VIOLATION {worst2}"
    print(f"[sweep-rand] {cnt2} splits, worst gap = {worst2}")

    # headline counterexample for opt-n: all-in on n2 (least liquidity)
    y = YS3["star"]
    allin2 = net_of([0, 0, y])
    opt = net_of(alloc_chain(y))
    assert opt > allin2, "expected composition to beat single-pool"
    pins["opt-n"] = {"y": y, "allin_n2": allin2, "opt": opt}
    print(f"[opt-n] all-in-n2 net={allin2} < composed net={opt}")
    return pins


# ── splt3 contract ──────────────────────────────────────────────────────────
def splt3_src():
    gdefs = "\n".join(f'(define G{c} "{G3[c]}")' for c in range(3))
    qdefs = "\n".join(f'(define Q{k}{c} "{QQ[k][c]}")'
                      for k in range(N) for c in range(2))
    wdefs = "\n".join(f'(define W{c} "{W3[c]}")' for c in range(2))
    acct_defs = "\n".join(f'(define POOL{k} "{ACCTS[k]}")' for k in range(N))
    alloc = '''(let* ((p (sget "s:p" G2))
             (y0 (mincap (qg "0" p) y))
             (r1 (sub y y0))
             (y1 (mincap (qg "1" p) r1))
             (r2 (sub r1 y1))
             (y2 (mincap (qg "2" p) r2))
             (cb (str-cat (str-cat "{\\"who\\":\\"") (str-cat who "\\"}"))))'''
    return f''';; splt3: N-pool composition (N=3, shared grid {G3}, fee {gen.NUM}/{gen.DEN}).
;; Net is concave in y => equal-fee optimum EQUALIZES ENDING PRICE p*
;; across pools (leg-2 theorem, N-pool form): p* solves
;;   sum_k qgross_k(p*) = y,   y_k = min(cap_k, qgross_k(p*)).
;; Flat unrolled binary search over p* (48 iters); qgross sums are
;; muldiv-free exact products.  No split beats this beyond per-cell
;; muldiv dust (oracle: 33^3 grid + 20k random splits, zero violations).
{gdefs}
{qdefs}
{wdefs}
{acct_defs}
(define SELF "splt3.clmm.test.near")
(define TGAS {gen.TGAS})
(define SWAP_ARGS "{{\\\\"min_out\\\\":\\\\"0\\\\"}}")

{gen.HELPERS}

;; gross units pool k consumes walking G0 -> p (exact: mdiv by "1")
;; MIRRORS model qgross: cell0 full below G1... NO: below G1 only the
;; G0->p part of cell 0; at/above G1 cell0 full + cell1 PART (p-G1),
;; never + full W1 (cap+partial overcounts by Q_k1*(G2-p) -- caught by
;; npool-two pins: search collapsed to G1+1, n2 attach 0, ERR_ZERO).
(define (qg k p)
  (if (ult G1 p)
      (add (mdiv (pickk k) W0 "1")
           (mdiv (pickk1 k) (sub p G1) "1"))
      (mdiv (pickk k) (sub p G0) "1")))

(define (pickk k)
  (if (u128/eq k "0") Q00 (if (u128/eq k "1") Q10 Q20)))

(define (pickk1 k)
  (if (u128/eq k "0") Q01 (if (u128/eq k "1") Q11 Q21)))

(define (sumq p)
  (add (qg "0" p) (add (qg "1" p) (qg "2" p))))

(define (mincap a b) (if (ult a b) a b))

(define (split)
  (let* ((y (near/attached_deposit_u128))
         (who (near/predecessor_account_id)))
    (begin
      (if (u128/eq y "0") (fail "ERR_ZERO") "ok")
      (sput "s:lo" G0)
      (sput "s:hi" G2)
      _BINSEARCH_
      (sput "s:p" (sget "s:hi" G2))
      {alloc}
        (begin
          (sput "sp:who" who)
          (sput "s:s0" y0)
          (sput "s:s1" y1)
          (sput "s:s2" y2)
          (near/log (str-cat "splt3 p=" p))
          (let ((p0 (near/promise_create POOL0 "swap" SWAP_ARGS y0 TGAS)))
            (near/promise_then p0 SELF "on0" cb "0" TGAS)
            (let ((p1 (near/promise_create POOL1 "swap" SWAP_ARGS y1 TGAS)))
              (near/promise_then p1 SELF "on1" cb "0" TGAS)
              (let ((p2 (near/promise_create POOL2 "swap" SWAP_ARGS y2 TGAS)))
                (near/promise_then p2 SELF "on2" cb "0" TGAS)
                (near/return "queued")))))))))

(define (on0)
  (let* ((who (sget "sp:who" ""))
         (out (near/promise_result 0))
         (pk (str-cat "paid:" who)))
    (if (= out "")
        (fail "ERR_P0")
        (begin
          (sput pk (add (sget pk "0") out))
          (near/return out)))))

(define (on1)
  (let* ((who (sget "sp:who" ""))
         (out (near/promise_result 0))
         (pk (str-cat "paid:" who)))
    (if (= out "")
        (fail "ERR_P1")
        (begin
          (sput pk (add (sget pk "0") out))
          (near/return out)))))

(define (on2)
  (let* ((who (sget "sp:who" ""))
         (out (near/promise_result 0))
         (pk (str-cat "paid:" who))
         (total (add (sget pk "0") out)))
    (if (= out "")
        (fail "ERR_P2")
        (begin
          (sput pk total)
          (near/log (str-cat "splt3 total=" total))
          (near/return out)))))

(define (get-paid)
  (near/return (sget (str-cat "paid:" (near/predecessor_account_id)) "0")))

(define (get-split)
  (near/return (str-cat (sget "s:s0" "0")
                        (str-cat "," (str-cat (sget "s:s1" "0")
                                              (str-cat "," (sget "s:s2" "0")))))))

(export "split" split)
(export "on0" on0)
(export "on1" on1)
(export "on2" on2)
(export "get-paid" get-paid)
(export "get-split" get-split)
'''


def binsearch_src():
    """48 FLAT steps: each reads lo/hi from storage, writes back. No nesting.
    Derived programmatically, balance verified by verify_str downstream."""
    step = ('(let* ((lo (sget "s:lo" G0))'
            ' (hi (sget "s:hi" G2))'
            ' (mid (mdiv (add lo hi) "1" "2"))'
            ' (s (sumq mid)))'
            ' (begin'
            ' (sput "s:lo" (if (ult s y) (add mid "1") lo))'
            ' (sput "s:hi" (if (ult s y) hi mid))))')
    return "\n      ".join([step] * 48)


def main():
    pins = oracle()
    src = splt3_src()
    marker = "_BINSEARCH_"
    assert marker in src, "binsearch marker missing"
    src = src.replace(marker, binsearch_src())
    gen.verify_str(src, "splt3")
    assert "str-cat who" in src and "sumq mid" in src
    assert '{\\"who\\":\\"' in src, "cb json key wrong (brace/escape desync)"
    assert src.count('(define (split)') == 1 and 'let*lohi' not in src

    import paths; d = os.path.join(paths.cdir("splt3"), "src")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "main.lisp"), "w") as f:
        f.write(src)
    with open(os.path.join(paths.cdir("splt3"), "near.json"), "w") as f:
        json.dump({"name": "splt3", "src": "src/main.lisp",
                   "output": f"target/{paths.MAP["splt3"][1]}.wasm"}, f)
    # new pool contracts (same fee as pa -- pool_src defaults)
    for k, name in ((1, "n1"), (2, "n2")):
        pd_ = os.path.join(paths.cdir(name), "src")
        os.makedirs(pd_, exist_ok=True)
        with open(os.path.join(pd_, "main.lisp"), "w") as f:
            f.write(gen.pool_src(G3, LIQS[k], f" composition pool {name}: "
                                 f"liq {LIQS[k]}, grid = pa's, fee = 0.3%"))
        with open(os.path.join(paths.cdir(name), "near.json"), "w") as f:
            json.dump({"name": name, "src": "src/main.lisp",
                       "output": f"target/{paths.MAP[name][1]}.wasm"}, f)
    with open(os.path.join(BASE, "pins3.json"), "w") as f:
        json.dump(pins, f, indent=1)
    print("# n1/n2/splt3 + pins3.json written, sources balanced")


if __name__ == "__main__":
    main()
