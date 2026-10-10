#!/usr/bin/env python3
"""AFP pool_combination port, leg 2: refine/slice on DIFFERENT grids + fees.

Pools (deliberately different grids — refine is no longer identity):
  pa: grid [1,2,4]e9  liq [1e23, 6e22]
  pb: grid [1,3,4]e9  liq [3e22, 9e22]
  pd: pool_join on the common refinement [1,2,3,4]e9 — B's [1,3] cell is
      CUT at 2e9 (refine's proportional split, exact: constants divide);
      per-cell liq = sum of both pools' density on the cell; each cell gets
      its own gross book Q = ceil(L*DEN/(DEN-NUM)) (cst_fee, phi=NUM/DEN).
  splt: combined walk over pd -> y1 = qA_gross(sqpC) capped -> legs to pa/pb.
Interior entry: swap takes optional "start" (default bottom tick); pins the
refine/slice identity — refined pool quoted from sqp = originals from sqp.

Emits: pa/ pb/ pd/ splt/ projects + pins.json (oracle ground truth) so
run_scen.py never re-types constants. Every floor mirrored oracle-side.
"""
import os, json

ROOT = os.path.dirname(os.path.abspath(__file__))
NUM, DEN = 3 * 10**15, 10**18
DPR = DEN - NUM
def Qceil(L, num=None, den=None):
    num = NUM if num is None else num
    den = DEN if den is None else den
    return -(-(L * den) // (den - num))
TGAS = "50000000000"
s = lambda n: f'"{n}"'

GA = [10**9, 2 * 10**9, 4 * 10**9]
GB = [10**9, 3 * 10**9, 4 * 10**9]
LA = [10**23, 6 * 10**22]
LB = [3 * 10**22, 9 * 10**22]
GJ = sorted(set(GA) | set(GB))

def liq_slice(grid, liq, lo, hi):
    """Liquidity of [lo,hi) for a constant-per-cell pool (refine's split)."""
    for i in range(len(grid) - 1):
        a, b = grid[i], grid[i + 1]
        if a <= lo and hi <= b:
            return liq[i] * (hi - lo) // (b - a)
    raise ValueError(f"cell [{lo},{hi}) not inside one grid cell")

LJ = [liq_slice(GA, LA, GJ[i], GJ[i + 1]) + liq_slice(GB, LB, GJ[i], GJ[i + 1])
      for i in range(len(GJ) - 1)]
QA = [Qceil(l) for l in LA]

# heterogeneous-fee leg (pool_fee_join): pc = P2, same grid as pa, fee 0.5%
CNUM, CDEN = 5 * 10**15, 10**18
CDPR = CDEN - CNUM
LC = [4 * 10**22, 9 * 10**22]
QC = [Qceil(l, CNUM, CDEN) for l in LC]
QB = [Qceil(l) for l in LB]
QJ = [Qceil(l) for l in LJ]
assert LA[0] * (GJ[1] - GJ[0]) % (GA[1] - GA[0]) == 0, "refine split must be exact"

HELPERS = '''(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (fail m) (near/log m) (near/abort m))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (mdiv a b d) (u128/muldiv a b d))
(define (ult a b) (u128/lt a b))'''

STEP = ''';; one step across the cell starting at sqp: net liq l, gross book q, bound
;; gb. dp = min(y, cap)/q (floor); only dp*q is consumed — dust stays for
;; refund; dp=0 cannot advance price, the dust guard prevents calling so.
(define (step sqp y l q gb out)
  (let* ((width (sub gb sqp))
         (cap (mdiv q width "1"))
         (take (if (ult cap y) cap y))
         (dp (u128/div take q))
         (used (mdiv dp q "1"))
         (p2 (add sqp dp))
         (gained (if (ult "0" dp) (mdiv l dp (mdiv sqp p2 "1")) "0")))
    (begin
      (sput "s:out" (add out gained))
      (sput "s:y" (sub y used))
      (sput "s:sqp" p2))))'''

CSTEP = ''';; combined-curve step from sqp consuming y on gross book q (no out track)
(define (cstep sqp y q gb)
  (let* ((width (sub gb sqp))
         (cap (mdiv q width "1"))
         (take (if (ult cap y) cap y))
         (dp (u128/div take q))
         (used (mdiv dp q "1")))
    (begin
      (sput "s:sqp" (add sqp dp))
      (sput "s:y" (sub y used)))))'''

def emit_walk(n, body):
    """Nested cell dispatch: outermost guard sqp<Gn is the caller's job."""
    expr = None
    for i in range(n - 1, -1, -1):
        branch = body(i)
        expr = branch if expr is None else f"(if (ult sqp G{i + 1}) {branch} {expr})"
    return expr

def walk_swap(n, qpat, lpat=None):
    """Walk + swap body shared by every pool (single source of floors).
    qpat: i -> book-constant name; lpat: i -> liq-constant name (default L{i})."""
    lpat = lpat or (lambda i: f"L{i}")
    walk_body = emit_walk(n, lambda i:
        f'(if (ult y {qpat(i)}) "done" (begin (step sqp y {lpat(i)} {qpat(i)} G{i + 1} out) (walk)))')
    return f'''(define (walk)
  (let* ((sqp (sget "s:sqp" "0"))
         (y (sget "s:y" "0"))
         (out (sget "s:out" "0")))
    (if (u128/eq y "0")
        "done"
        (if (ult sqp G{n})
            {walk_body}
            "done"))))

(define (swap)
  (let* ((y (near/attached_deposit_u128))
         (caller (near/predecessor_account_id))
         (minout (default (near/json_get_str "min_out") "0"))
         (st0 (default (near/json_get_str "start") "0"))
         (start (if (u128/eq st0 "0") G0 st0)))
    (begin
      (if (u128/eq y "0") (fail "ERR_ZERO") "ok")
      (if (ult start G0) (fail "ERR_START") "ok")
      (sput "s:sqp" start)
      (sput "s:y" y)
      (sput "s:out" "0")
      (walk)
      (let* ((out (sget "s:out" "0"))
             (yleft (sget "s:y" "0"))
             (pk (str-cat "paid:" caller))
             (newpaid (add (sget pk "0") out)))
        (begin
          (if (ult out minout) (fail "ERR_SLIP") "ok")
          (sput pk newpaid)
          (sput (str-cat "left:" caller) yleft)
          (near/log (str-cat "swap out=" out))
          (near/return out))))))

(define (get-paid)
  (near/return (sget (str-cat "paid:" (near/predecessor_account_id)) "0")))

(export "swap" swap)
(export "get-paid" get-paid)'''


def pool_src(grid, Ls, note, num=None, den=None):
    num = NUM if num is None else num
    den = DEN if den is None else den
    n = len(grid) - 1
    gdefs = "\n".join(f'(define G{i} "{grid[i]}")' for i in range(n + 1))
    ldefs = "\n".join(f'(define L{i} "{Ls[i]}")' for i in range(n))
    qdefs = "\n".join(f'(define Q{i} "{Qceil(Ls[i], num, den)}")' for i in range(n))
    body = walk_swap(n, lambda i: f"Q{i}")
    return f''';; CLMM grid pool (AFP CLMM_Operations port) —{note}
;; grid {" / ".join(str(g) for g in grid)}   net liq [{", ".join(str(l) for l in Ls)}]
;; fee phi = {num}/{den}: gross book Q=ceil(L*den/(den-num)) per cell
;; (AFP gross_fct L/(1-phi), fixed-point ceil, precomputed by generator).
;; swap: deposit=y, optional "start" (default G0; interior entry = the
;; refine/slice identity pin); out += l*dp/(sqp*p2) via u128/muldiv.
;; ledger: paid:<caller> accumulates out; left:<caller> = refunded dust.
{gdefs}
{ldefs}
{qdefs}

{HELPERS}

{STEP}

{body}'''

def qaux_src(grid):
    """Cumulative GROSS quote from G0 to p over this pool's cells."""
    n = len(grid) - 1
    expr = None
    for i in range(n - 1, -1, -1):
        acc = f'(mdiv Q{i} (sub p G{i}) "1")'
        for j in range(i - 1, -1, -1):
            acc = f'(add (mdiv Q{j} (sub G{j + 1} G{j}) "1") {acc})'
        expr = acc if expr is None else f"(if (ult p G{i + 1}) {acc} {expr})"
    return expr

def splitter_src():
    na, nc = len(GA) - 1, len(GJ) - 1
    # equalized-split zone data (merged breakpoints 1e9 | 2e9=A | 3e9=B | 4e9)
    w1, w2, w3 = GA[1] - GA[0], GB[1] - GA[1], GA[2] - GB[1]
    RS0, RS1, RS2 = QA[0] + QB[0], QA[1] + QB[0], QA[1] + QB[1]
    CUM1 = RS0 * w1                      # combined gross consumption to 2e9
    YA1 = QA[0] * w1                     # qA(2e9)
    CUM2 = CUM1 + RS1 * w2               # to 3e9
    YA2 = YA1 + QA[1] * w2               # qA(3e9)
    ag = "\n".join(f'(define G{i} "{GA[i]}")' for i in range(len(GA)))
    aq = "\n".join(f'(define Q{i} "{QA[i]}")' for i in range(na))
    zone = (f'(define CUM1 "{CUM1}")\n(define CUM2 "{CUM2}")\n'
            f'(define RS0 "{RS0}")\n(define RS1 "{RS1}")\n(define RS2 "{RS2}")\n'
            f'(define YA1 "{YA1}")\n(define YA2 "{YA2}")')
    return f''';; Splitter: AFP pool_combination optimality on DIFFERENT grids.
;; pd = pool_join(refine pa, refine pb) on {GJ} (equality: see pd).
;; OPTIMAL split = equalize the legs' ending price: p* solves
;; qA(p*) + qB(p*) = y piecewise over merged breakpoints {GA[1]}(A) / {GB[1]}(B);
;; y1 = qA(p*), leg B gets y-y1 = qB(p*). (Shared-grid shortcut
;; y1 = qA(sqpC) is SUBOPTIMAL here — the opt-sweep caught it.)
;; Fee phi = {NUM}/{DEN} via gross books; all floors mirrored in the oracle.
{ag}
{aq}
{zone}
(define TGAS {TGAS})
(define POOLA "pa.clmm.test.near")
(define POOLB "pb.clmm.test.near")
(define SELF "splt.clmm.test.near")
(define SWAP_ARGS "{{\\"min_out\\":\\"0\\"}}")

{HELPERS}

;; equalized split: zone-solve p*, y1 = qA(p*) (muldiv floors, mirrored)
(define (y1-for y)
  (if (ult y CUM1)
      (mdiv Q0 y RS0)
      (if (ult y CUM2)
          (add YA1 (mdiv Q1 (sub y CUM1) RS1))
          (add YA2 (mdiv Q1 (sub y CUM2) RS2)))))

(define (split)
  (let* ((y (near/attached_deposit_u128))
         (who (near/predecessor_account_id)))
    (begin
      (if (u128/eq y "0") (fail "ERR_ZERO") "ok")
      (if (u128/eq (y1-for y) "0") (fail "ERR_DUST") "ok")
      (let* ((y1raw (y1-for y))
             (y1 (if (ult y y1raw) y y1raw))
             (yb (sub y y1))
             (cb (str-cat (str-cat "{{\\"who\\":\\"") (str-cat who "\\"}}")))
             (pa (near/promise_create POOLA "swap" SWAP_ARGS y1 TGAS)))
        (begin
          (near/log (str-cat "split y1=" y1))
          (sput "sp:who" who)
          (near/promise_then pa SELF "on-a" cb "0" TGAS)
          (let ((pb (near/promise_create POOLB "swap" SWAP_ARGS yb TGAS)))
            (near/promise_then pb SELF "on-b" cb "0" TGAS)
            (near/return "queued")))))))

(define (on-a)
  (let* ((who (sget "sp:who" ""))
         (out (near/promise_result 0))
         (pk (str-cat "paid:" who)))
    (if (= out "")
        (fail "ERR_LEG_A")
        (begin
          (sput pk (add (sget pk "0") out))
          (near/log (str-cat "leg-a out=" out))
          (near/return out)))))

(define (on-b)
  (let* ((who (sget "sp:who" ""))
         (out (near/promise_result 0))
         (pk (str-cat "paid:" who))
         (total (add (sget pk "0") out)))
    (if (= out "")
        (fail "ERR_LEG_B")
        (begin
          (sput pk total)
          (near/log (str-cat "leg-b out=" out))
          (near/log (str-cat "split total=" total))
          (near/return out)))))

(define (get-paid)
  (near/return (sget (str-cat "paid:" (near/predecessor_account_id)) "0")))

(export "split" split)
(export "on-a" on-a)
(export "on-b" on-b)
(export "get-paid" get-paid)
'''

# ── heterogeneous-fee join (pool_fee_join P1 P2, CST-FEE hypotheses) ────────
def union_fee_chain(l1, l2, f1n, f1d, f2n, f2d):
    """Fhat_floor = union_fee_chain(...) per cell: fee-scale (1e15) nested
    floors mirroring the contract muldivs. Args: NET liqs, fee fractions.
    Fhat = num/den; gross = l*den/(den-num) EXACT on-chain (Qceil).
    Overflow audited: max intermediate = lmax*FS ≈ 1.3e38 < 2^127."""
    FS = 10**15
    a1n, a1d = f1d - f1n, f1d            # 1 - f1
    a2n, a2d = f2d - f2n, f2d            # 1 - f2
    n1 = (l1 * a2n) // a2d               # l1*(1-f2)   <- CROSS pairing
    n2 = (l2 * a1n) // a1d               # l2*(1-f1)   <- CROSS pairing
    df  = n1 + n2                        # fee-union denominator
    tn1 = (n1 * f1n) // f1d              # (l1*(1-f2))*f1
    tn2 = (n2 * f2n) // f2d              # (l2*(1-f1))*f2
    tn  = tn1 + tn2
    num = (tn * FS) // df                # tn*FS/df   (≤ 1.3e38)
    den = (df * FS) // df                # == FS exactly
    return num, den


def join_src(note):
    """pj = pool_fee_join of pa (phi=3e-4) and pc (phi=5e-3) on grid GA.
    Gross book QBK{i} = QA{i} + QC{i} (join_gross_fct: EXACT add); net
    LK{i} = LA{i} + LC{i}; per-cell union fee Fh{i}=num/den via the
    fee-scale nested-floor chain (the ONLY rounded quantity on-chain).
    Qhat must satisfy Qceil(That, num, den) <= QBK{i} — the decomposition
    bound direction — asserted here at generation time."""
    cells = []
    for i in range(len(GA) - 1):
        num, den = union_fee_chain(LA[i], LC[i], NUM, DEN, CNUM, CDEN)
        qbk = QA[i] + QC[i]
        that = Qceil(LA[i] + LC[i], num, den)
        assert that <= qbk, f"cell {i}: Qhat {that} > QBK {qbk} — bound direction violated"
        cells.append((i, num, den, qbk, that))
    gdefs = "\n".join(f'(define G{i} "{GA[i]}")' for i in range(len(GA)))
    ldefs = "\n".join(f'(define LK{i} "{LA[i] + LC[i]}")' for i in range(len(LA)))
    bk = "\n".join(f'(define QBK{i} "{c[3]}")' for i, c in enumerate(cells))
    fh = "\n".join(f'(define Fh{i} "{c[1]}")' for i, c in enumerate(cells))
    fd = "\n".join(f'(define Fd{i} "{c[2]}")' for i, c in enumerate(cells))
    fee_defs = "\n".join(
        f'(define (fee{i}) (mdiv Fh{i} FS Fd{i}))' for i, c in enumerate(cells))
    body = walk_swap(len(LA), lambda i: f"QBK{i}", lambda i: f"LK{i}")
    return f''';; pool_fee_join P1 P2 (AFP CLMM_Transformation) —{note}
;; P1 = pa (phi 3e15/1e18) + P2 = pc (5e15/1e18), joint on grid GA.
;; gross_fct additivity holds EXACTLY for heterogeneous fees (join_gross_fct):
;;   QBK{{i}} = QA{{i}} + QC{{i}}  (integer add, no rounding)
;; fee P i = fee_union l1 l2 f1 f2 = (l1 f1 (1-f2) + l2 f2 (1-f1)) /
;;   (l1 (1-f2) + l2 (1-f1)) — fee-scale (FS=1e15) nested floors, the ONLY
;;   rounded quantity here; get-fee exports the per-cell Fhat.
{gdefs}
(define FS "1000000000000000")
{ldefs}
{bk}
{fh}
{fd}
{fee_defs}

{HELPERS}

{STEP}

(define (get-fee)
  (near/return (fee0)))

(export "get-fee" get-fee)''' + "\n\n" + body


def near_json(name):
    import paths
    return json.dumps({"name": name, "src": "src/main.lisp",
                       "account": f"{name}.clmm.test.near", "network": "local",
                       "output": f"target/{paths.MAP[name][1]}.wasm"}, indent=1)

def main():
    projects = [
        ("pa", pool_src(GA, LA, " pool A, grid has tick 2e9")),
        ("pb", pool_src(GB, LB, " pool B, grid has tick 3e9 (DIFFERENT from A)")),
        ("pd", pool_src(GJ, LJ, " pool_comb = join on common refinement of A,B")),
        ("splt", splitter_src()),
        ("pc", pool_src(GA, LC, " pool C: fee-union leg P2, grid = A's, fee 0.5%",
                        CNUM, CDEN)),
        ("pj", join_src(" joint pool = pool_fee_join(pa, pc)")),
    ]
    assert "str-cat who" in dict(projects)["splt"], "cb construction missing from template"
    # gate BEFORE writing: every rendered contract must balance in memory
    allok = True
    for d, src in projects:
        ok = verify_str(d + "/src/main.lisp", src)
        allok &= ok
    if not allok:
        raise SystemExit("IMBALANCE — fix generator, nothing written")
    for d, src in projects:
        import paths; p = paths.cdir(d)
        os.makedirs(os.path.join(p, "src"), exist_ok=True)
        with open(os.path.join(p, "src", "main.lisp"), "w") as f:
            f.write(src)
        with open(os.path.join(p, "near.json"), "w") as f:
            f.write(near_json(d))
        print("wrote", p)
    print("ALL-BALANCED")
    oracle()

def verify_str(name, text):
    depth, instr, line, first_bad = 0, False, 1, None
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if instr:
            if c == "\\":
                i += 2; continue
            if c == '"':
                instr = False
        elif c == '"':
            instr = True
        elif c == ";":
            while i < n and text[i] != "\n":
                i += 1
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth < 0 and first_bad is None:
                first_bad = line
            depth = max(depth, 0)
        if c == "\n":
            line += 1
        i += 1
    ok = depth == 0 and first_bad is None
    print(("OK  " if ok else "BAD ") + name + ("" if ok else f"  enddepth={depth} neg@{first_bad}"))
    return ok

def verify(path):
    return verify_str(path, open(path).read())

# ── oracle (cell-based; mirrors the contracts floor-for-floor) ──────────────
def swap(grid, Ls, Qs, sqp, y):
    out = 0
    top = grid[-1]
    while y > 0 and sqp < top:
        i = next(k for k in range(len(grid) - 1) if grid[k] <= sqp < grid[k + 1])
        l, q, hi = Ls[i], Qs[i], grid[i + 1]
        cap = (hi - sqp) * q
        if y >= cap:
            dp = hi - sqp
            out += (l * dp) // (sqp * hi); y -= cap; sqp = hi
        elif y < q:
            break
        else:
            dp = y // q; p2 = sqp + dp
            out += (l * dp) // (sqp * p2); y -= dp * q; sqp = p2
    return out, sqp, y

def qgross(grid, Qs, p):
    tot = 0
    for i in range(len(grid) - 1):
        if p <= grid[i]:
            break
        tot += Qs[i] * (min(p, grid[i + 1]) - grid[i])
    return tot

# equalized-split model, zone form — VERBATIM mirror of the splt template's
# y1-for (same muldiv floors): p* solves qA(p*)+qB(p*)=y over MERGED
# breakpoints 2e9 (A's tick) / 3e9 (B's tick); y1 = qA(p*).
_w1 = GA[1] - GA[0]                    # zone 1: [1e9, 2e9), both cell 1
_w2 = GB[1] - GA[1]                    # zone 2: [2e9, 3e9), A cell 2 + B cell 1
_RS0, _RS1, _RS2 = QA[0] + QB[0], QA[1] + QB[0], QA[1] + QB[1]
_CUM1 = _RS0 * _w1
_YA1 = QA[0] * _w1
_CUM2 = _CUM1 + _RS1 * _w2
_YA2 = _YA1 + QA[1] * _w2

def y1_model(y):
    if y < _CUM1:
        return QA[0] * y // _RS0
    if y < _CUM2:
        return _YA1 + QA[1] * (y - _CUM1) // _RS1
    return _YA2 + QA[1] * (y - _CUM2) // _RS2

YS = {"exact": 1313 * 10**27, "offgrid": 13 * 10**29, "cross": 3 * 10**32}
INSIDE_SQP, INSIDE_Y = 25 * 10**8, 6 * 10**31

def oracle():
    pins = {"GA": GA, "GB": GB, "GJ": GJ, "LA": LA, "LB": LB, "LJ": LJ,
            "QA": QA, "QB": QB, "QJ": QJ, "inside_sqp": INSIDE_SQP}

    # ── heterogeneous-fee join pins (pool_fee_join of pa + pc) ─────────
    from fractions import Fraction
    YJOIN = 5 * 10**27
    jn = {"grid": GA, "LC": LC, "QC": QC, "Y": YJOIN}
    print(f"\n# fee_union (pa {NUM}/{DEN} + pc {CNUM}/{CDEN})")
    Fh, Fd = [], []
    for i in range(len(GA) - 1):
        num, den = union_fee_chain(LA[i], LC[i], NUM, DEN, CNUM, CDEN)
        Fh.append(num); Fd.append(den)
        F = Fraction(LA[i] * NUM * (CDEN - CNUM) +
                     LC[i] * CNUM * (DEN - NUM),
                     LA[i] * (CDEN - CNUM) * DEN +
                     LC[i] * (DEN - NUM) * CDEN)
        err = abs(F - Fraction(num, den)) * DEN
        print(f"cell{i}: Fhat={num}/{den} |Fhat-F|*1e18={float(err):.3f}")
        assert err <= 2000, f"cell {i}: union-fee error {err} > 2000e-18"
        assert num < den, f"cell {i}: Fhat >= 1"
    jn["Fh"], jn["Fd"] = Fh, Fd
    jn["QBK"] = [QA[i] + QC[i] for i in range(len(GA) - 1)]
    jn["LK"] = [LA[i] + LC[i] for i in range(len(LA))]
    print("# QBK:", jn["QBK"], " LK:", jn["LK"])

    print("\n# join decomposition (quote_gross_join / quote_net_join shape)")
    dq = {}
    for i in range(len(GA) - 1):
        cell = GA[i + 1] - GA[i]
        gA, gC = QA[i] * cell, QC[i] * cell
        gK = (QA[i] + QC[i]) * cell
        qh = Qceil(LA[i] + LC[i], Fh[i], Fd[i])
        gQ = qh * cell
        assert gK == gA + gC, f"cell {i}: gross additivity broken"
        assert gQ <= gK, f"cell {i}: Qhat book {gQ} > QBK {gK}"
        oK, _, _ = swap(GA, [LA[i] + LC[i]], [QA[i] + QC[i]], GA[0], YJOIN)
        oA, _, _ = swap(GA, [LA[i]], [QA[i]], GA[0], YJOIN)
        oC, _, _ = swap(GA, [LC[i]], [QC[i]], GA[0], YJOIN)
        YoK, _, _ = swap(GA, [LA[i] + LC[i]], [qh], GA[0], YJOIN)
        YoA, _, _ = swap(GA, [LA[i]], [QA[i]], GA[0], YJOIN)
        YoC, _, _ = swap(GA, [LC[i]], [QC[i]], GA[0], YJOIN)
        assert YoK <= YoA + YoC, f"cell {i}: net decomposition violated"
        print(f"cell{i}: gross K={gK}=A+C ✓ Qhat-book={gQ} ✓ | "
              f"net joined={oK} sum={oA + oC} slack={oK - (oA + oC)} | "
              f"ynet joined={YoK} sum={YoA + YoC} slack={YoK - (YoA + YoC)}")
        dq[f"c{i}"] = {"gross": gK, "grossAC": gA + gC, "qhat": qh,
                       "net": oK, "netAC": oA + oC,
                       "ynet": YoK, "ynetAC": YoA + YoC}
    jn["decomp"] = dq
    pins["join"] = jn

    # full-pool join swap + in-scenario alt split (≤ joined, decomposition)
    oJ, sqpJ, _ = swap(GA, jn["LK"], jn["QBK"], GA[0], YJOIN)
    y1j = YJOIN // 2
    sA = swap(GA, LA, QA, GA[0], y1j)[0]
    sC = swap(GA, LC, QC, GA[0], YJOIN - y1j)[0]
    assert sA + sC <= oJ, "alt split beats joined pool"
    jn["out"], jn["sqp_end"] = oJ, sqpJ
    jn["alt"] = {"y1": y1j, "outA": sA, "outC": sC}
    print(f"join full: y={YJOIN} out={oJ} sqp_end={sqpJ} | "
          f"alt y1={y1j}: A+C={sA + sC} <= {oJ} ✓")
    print(f"# pins (phi={NUM}/{DEN}, different grids: A@2e9, B@3e9, join@{{2,3}}e9)")
    base = {}
    for tag, y in YS.items():
        outC, _, _ = swap(GJ, LJ, QJ, GJ[0], y)
        y1 = y1_model(y)
        outA, _, lA = swap(GA, LA, QA, GA[0], y1)
        outB, _, lB = swap(GB, LB, QB, GB[0], y - y1)
        base[tag] = {"y": y, "comb": outC, "y1": y1,
                     "outA": outA, "outB": outB, "sum": outA + outB,
                     "leftA": lA, "leftB": lB}
        print(f"{tag}: y={y} y1={y1} comb={outC} sum={outA+outB} "
              f"slack={outA+outB-outC} leftB={lB}")
    # cross must land strictly inside zone 2: A past its 2e9 tick, B past
    # what it would hold at a 2e9 equalization — i.e. p* in (2e9, 3e9)
    _yb_cross = YS["cross"] - y1_model(YS["cross"])
    assert y1_model(YS["cross"]) > _YA1 and _yb_cross > QB[0] * _w1, \
        "cross must split strictly inside zone 2"
    pins["base"] = base

    # refine/slice identity: interior entry pins (direct swaps, no split)
    si = {}
    print(f"\n# inside entry sqp={INSIDE_SQP} (inside ALL THREE grids)")
    for name, g, l, q in [("pa", GA, LA, QA), ("pb", GB, LB, QB), ("pd", GJ, LJ, QJ)]:
        o, sqpe, _ = swap(g, l, q, INSIDE_SQP, INSIDE_Y)
        si[name] = {"out": o, "sqp_end": sqpe}
        print(f"inside {name}: out={o} sqp_end={sqpe}")
    si["sum"] = si["pa"]["out"] + si["pb"]["out"]
    si["slack"] = si["sum"] - si["pd"]["out"]
    print(f"inside: comb={si['pd']['out']} sum={si['sum']} slack={si['slack']}")
    pins["inside"] = si

    # opt-sweep under fees, different grids: no alt split beats y1
    print("\n# opt-sweep (quote_swap_opt_above, phi=0.3%, different grids)")
    opts = {}
    for tag, y in YS.items():
        b = base[tag]
        y1, opt_total = b["y1"], b["sum"]
        pts = {0, y1, _CUM1, _CUM2, y - 1}
        stride = max(1, y1 // 400)
        pts.update(range(0, y1 + 1, stride))
        pts.update(range(max(0, y1 - 600), y1 + 600))
        bad, mx, mx_y2, n = 0, None, None, 0
        for y2 in sorted(p for p in pts if 0 <= p < y1):
            oA = swap(GA, LA, QA, GA[0], y2)[0]
            oB = swap(GB, LB, QB, GB[0], y - y2)[0]
            tot = oA + oB
            n += 1
            if tot > opt_total:
                bad += 1
            if mx is None or tot > mx:
                mx, mx_y2 = tot, y2
        print(f"{tag}: y1={y1} opt_total={opt_total} n_alts={n} violations={bad} "
              f"max_alt=({mx} @ y2={mx_y2}) ok={bad == 0}")
        assert bad == 0, f"{tag}: optimality violated at y2={mx_y2}"
        # on-chain samples: 2 alt splits, cumulative leg outputs
        alts = []
        for y2 in (1, y1 // 2):
            oA = swap(GA, LA, QA, GA[0], y2)[0]
            oB = swap(GB, LB, QB, GB[0], y - y2)[0]
            assert oA + oB <= opt_total
            alts.append({"y2": y2, "outA": oA, "outB": oB})
        opts[tag] = {"y1": y1, "opt_total": opt_total, "alts": alts}
    pins["opt"] = opts

    with open(os.path.join(ROOT, "pins.json"), "w") as f:
        json.dump(pins, f, indent=1)
    print("\nwrote pins.json")

if __name__ == "__main__":
    main()
