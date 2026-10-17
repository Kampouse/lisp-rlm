#!/usr/bin/env python3
"""fee-gen surgery, single pass, paren-safe.
Every replacement is asserted paren-neutral. All anchors from clean HEAD
(2404680c). Compile follows as a separate gate."""

P = "lib/clmm_pool_v5.lisp"
src = open(P).read()
orig = src
fails = []

def net(t):
    # per-line: ';' comments run to end of LINE (not end of text);
    # strings don't span lines. Whole-text 'break' was wrong and made
    # every fragment containing ';;' comments measure garbage.
    total = 0
    for line in t.split("\n"):
        d = 0; s = False
        i = 0
        while i < len(line):
            c = line[i]
            if s:
                if c == "\\":
                    i += 2; continue
                if c == '"':
                    s = False
            else:
                if c == '"':
                    s = True
                elif c == ";":
                    break
                elif c == "(":
                    d += 1
                elif c == ")":
                    d -= 1
            i += 1
        total += d
    return total

def sub(old, new, tag, count=1):
    global src
    if src.count(old) != count:
        fails.append(f"{tag}: anchor x{src.count(old)} (want {count})")
        return
    delta = net(old) - net(new)
    if delta != 0:
        fails.append(f"{tag}: paren delta {delta} (old net {net(old)}, new net {net(new)}) — fragment must be net-preserving")
        return
    print(f"ok: {tag}")
    src = src.replace(old, new)

# ═══ 1. header doc ═══
sub(""";; Market sim (Oct 16, test_pool_v5_market.rs): NO fee parameter exists —
;; books take full in/out (fee cells never move, invariant at top), and
;; the grid curve quotes a per-cell BID-ASK SPREAD instead: a boot round
;; trip (sell X → Y, reverse the exact Y) restores S/P to the atom and
;; returns 1e9−186950004 X; the spread lands in the pool's token balance
;; (LP-side surplus). Order-book-style pricing, not AMM skim. Traders ride
;; the real NEP-141 path (ft_transfer_call msg "sw" → escrow → walk →
;; payout); range edges clamp: past-lip sells refund whole until reverse
;; pressure walks P back inside.""",
""";; Fee generation (Oct 16): custody-path swaps skim F = intake >> 8
;; (1/256, floor). The walker runs on v_eff = intake − F; F stays in the
;; intake escrow (trader pays X_eff + F; escrow retains exactly F) and
;; per CROSSED cell i the arm credits FG<side>:i += (F_i << 64) // LQ_i
;; (equal split, last cell takes the remainder; LQ=0 cells forfeit —
;; under-credit only, the div_floor64 slack proved in Fee_Slack).
;; Positions snapshot growth at mint (FS<side>:<who>:<a>:<b>:<i> + FSP
;; marker; parity mints get no marker ⇒ they accrue zero — clean epoch).
;; burnc folds owed_fee = Σ (FG−FS)×d >> 64 (accruef, Aggregate_Accrual
;; verbatim; bridge theorem = claims ≤ charged mass), caps each side at
;; the float (p5-req-cap), folds into OWX/OWY (claim pays), resets FS.
;; Parity swap/quote stay GROSS (differential oracle). 6|FEE<side>
;; totals parked mass (op "state" carries it; op "fg" {i} reads cells).
;; The grid bid-ask spread (market sim) is SEPARATE and still pools.""",
    "header-doc")

# ═══ 2. helpers (paren-neutral: complete defines) ═══
sub("""(define (p5-buf-str b)
  (limbs->str b (if (> (v5-strip b 12) 0) (v5-strip b 12) 1)))""",
"""(define (p5-buf-str b)
  (limbs->str b (if (> (v5-strip b 12) 0) (v5-strip b 12) 1)))

;; ── fee math (limb primitives; string-in/string-out) ──────────────────
(define (p5-shr8 v)
  (let ((A (str->limbs v)) (tA (buf-alloc 32)) (tB (buf-alloc 8))
        (q (buf-alloc 32)) (r (buf-alloc 8)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 8) (v5-copy! (car A) tA (car (cdr A)))
      (v5-zero! tB 2) (limb-set! tB 0 256)
      (v5-zero! q 8) (v5-zero! r 2)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))

(define (p5-C64! b)
  (begin
    (v5-zero! b 4) (limb-set! b 0 1)
    (loop ((j 0))
      (if (>= j 64) 0 (begin (v5-double! b 4) (recur (+ j 1)))))))

(define (p5-feegrow F L)
  (if (= (li-cmp L "0") 0)
      "0"
      (let ((FA (str->limbs F)) (LA (str->limbs L))
            (tF (buf-alloc 32)) (tC (buf-alloc 48)) (C64 (buf-alloc 16))
            (q (buf-alloc 32)) (r (buf-alloc 16)) (one (buf-alloc 8))
            (nbs (buf-alloc 64)) (s (buf-alloc 400)))
        (begin
          (v5-zero! tF 8) (v5-copy! (car FA) tF (car (cdr FA)))
          (p5-C64! C64)
          (v5-zero! tC 12)
          (limb-mul tF C64 tC 8 4)
          (v5-zero! q 8) (v5-zero! r 4)
          (v5-divmod! tC (car LA) q r nbs s one)
          (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1))))))

(define (p5-mulshr64 x y)
  (let ((XA (str->limbs x)) (YA (str->limbs y))
        (tC (buf-alloc 48)) (C64 (buf-alloc 16))
        (q (buf-alloc 32)) (r (buf-alloc 16)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tC 12)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (p5-C64! C64)
      (v5-zero! q 8) (v5-zero! r 4)
      (v5-divmod! tC C64 q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
    "fee-math-helpers")

# ═══ 3. small int quot + li fold + credit walker (after p5-imax) ═══
sub("""(define (p5-imax) (str->num (p5-bz "6|IMAX")))""",
"""(define (p5-imax) (str->num (p5-bz "6|IMAX")))

;; F // k for small k (≤ 65)
(define (p5-quotk F k)
  (let ((FA (str->limbs F)) (tA (buf-alloc 32)) (tB (buf-alloc 8))
        (q (buf-alloc 32)) (r (buf-alloc 8)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 8) (v5-copy! (car FA) tA (car (cdr FA)))
      (v5-zero! tB 2) (limb-set! tB 0 k)
      (v5-zero! q 8) (v5-zero! r 2)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))

;; F × (k−1) for small k
(define (p5-mulk F k)
  (loop ((i 1) (acc F))
    (if (>= i k) acc (recur (+ i 1) (li-add acc F)))))

;; equal-split credit: F over cells [p0..p1] (last takes remainder),
;; grow FG<side>:i += (F_i << 64) // LQ_i. LQ=0 cells forfeit (pool
;; keeps F in escrow — under-credit only). p0..p1 inclusive, ≤ span.
(define (p5-credit! side F p0 p1)
  (let ((k (+ (- p1 p0) 1)))
    (let ((Fi (p5-quotk F k)))
      (loop ((i p0))
        (if (> i p1)
            0
            (let ((fi (if (= i p1)
                          (li-sub F (p5-mulk Fi (- k 1)))
                          Fi)))
              (begin
                (p5-put (str-cat "6|FG" (str-cat side (str-cat ":" (to-string i))))
                        (li-add (p5-bz (str-cat "6|FG" (str-cat side (str-cat ":" (to-string i)))))
                                (p5-feegrow fi (p5-bz (p5-lqkey i)))))
                (recur (+ i 1)))))))))""",
    "credit-walker")

# ═══ 4. swarm-up: fee entry + credit exit ═══
sub("""(define (p5-swarm-up s p xbook ybook v)
  (let* ((imax (p5-imax))""",
"""(define (p5-swarm-up s p xbook ybook v feep?)
  (let* ((v (if feep? (li-sub v (p5-shr8 v)) v))
         (imax (p5-imax))""",
    "swarm-up-entry")

sub("""          (p5-put "6|Y" ynew)
          (p5-put "6|X" (li-add xbook xin))
          (p5-put "6|LAST" msg)
          msg)))))""",
"""          (p5-put "6|Y" ynew)
          (p5-put "6|X" (li-add xbook xin))
          (p5-put "6|LAST" msg)
          (if feep?
              (begin
                (p5-credit! "X" (p5-shr8 v) p (str->num (p5-bz "6|P")))
                (p5-put "6|FEEX" (li-add (p5-bz "6|FEEX") (p5-shr8 v)))
                0)
              0)
          msg)))))""",
    "swarm-up-exit")

# ═══ 5. swarm-dn mirror ═══
sub("""(define (p5-swarm-dn s p xbook ybook v)
  (let* ((imin (p5-imin))""",
"""(define (p5-swarm-dn s p xbook ybook v feep?)
  (let* ((v (if feep? (li-sub v (p5-shr8 v)) v))
         (imin (p5-imin))""",
    "swarm-dn-entry")

sub("""          (p5-put "6|X" xnew)
          (p5-put "6|Y" (li-add ybook yin))
          (p5-put "6|LAST" msg)
          msg)))))""",
"""          (p5-put "6|X" xnew)
          (p5-put "6|Y" (li-add ybook yin))
          (p5-put "6|LAST" msg)
          (if feep?
              (begin
                (p5-credit! "Y" (p5-shr8 v) (str->num (p5-bz "6|P")) p)
                (p5-put "6|FEEY" (li-add (p5-bz "6|FEEY") (p5-shr8 v)))
                0)
              0)
          msg)))))""",
    "swarm-dn-exit")

# ═══ 6. callers ═══
sub("""    (if (= neg 1)
        (p5-swarm-dn s p xbook ybook v)
        (p5-swarm-up s p xbook ybook v)))))))))))))""",
"""    (if (= neg 1)
        (p5-swarm-dn s p xbook ybook v 0)
        (p5-swarm-up s p xbook ybook v 0)))))))))))))""",
    "op-swap-gross")

sub("""          ;; up sells X for Y (p5-swap: v>0 = sell x)
          (p5-swarm-up s p xbook ybook amt)""",
"""          ;; up sells X for Y (fee path)
          (p5-swarm-up s p xbook ybook amt 1)""",
    "custody-x-fee")

sub("""          ;; down sells Y for X (p5-swap: v<0 = sell y)
          (p5-swarm-dn s p xbook ybook amt)""",
"""          ;; down sells Y for X (fee path)
          (p5-swarm-dn s p xbook ybook amt 1)""",
    "custody-y-fee")

# ═══ 7. leg-decide: intake drain keeps the skim in escrow ═══
sub("""            (let ((c (li-sub amt (p5-bz "6|REM"))))
              (begin
                (p5-put ekey (li-sub (p5-bz ekey) c))
                (p5-payout sender out (p5-bz "6|REM") c ps0 pl0 ekey okey tokkey)))))))""",
"""            ;; fee path: chain result refunds rm only — the skim F =
            ;; amt>>8 STAYS in escrow (LP-claimable; FG cells ledger it)
            (let ((c (li-sub (li-sub amt (p5-shr8 amt)) (p5-bz "6|REM"))))
              (begin
                (p5-put ekey (li-sub (p5-bz ekey) c))
                (p5-payout sender out (p5-bz "6|REM") c ps0 pl0 ekey okey tokkey)))))))""",
    "leg-decide-fee")

# ═══ 8. state + fg view + dispatch ═══
sub("""      (str-cat ":TOKX:" (p5-bz "6|TOKX"))
      (str-cat ":TOKY:" (p5-bz "6|TOKY")))))""",
"""      (str-cat ":TOKX:" (p5-bz "6|TOKX"))
      (str-cat ":TOKY:" (p5-bz "6|TOKY"))
      (str-cat ":FEEX:" (p5-bz "6|FEEX"))
      (str-cat ":FEEY:" (p5-bz "6|FEEY")))))

;; read-only fee view: one cell's growth pair. ZERO writes.
(define (p5-fg)
  (let ((i (json-get-str "i" (near/input))))
    (str-cat "fgx:" (str-cat (p5-bz (str-cat "6|FGX:" i))
               (str-cat ":fgy:" (p5-bz (str-cat "6|FGY:" i)))))))""",
    "state-fg")

sub("""    (if (str= op "state") (p5-state)
    (if (str= op "g") (p5-G (str->num (json-get-str "i" (near/input))))
        "unknown-op"))))))))))))))))""",
"""    (if (str= op "state") (p5-state)
    (if (str= op "fg") (p5-fg)
    (if (str= op "g") (p5-G (str->num (json-get-str "i" (near/input))))
        "unknown-op")))))))))))))))))""",
    "dispatch-fg")

# ═══ 9. fskey trio after poskey (forward-ref safe) ═══
sub("""(define (p5-poskey w a b)
  (str-cat "6|POS:" (str-cat w (str-cat ":" (str-cat (to-string a)
    (str-cat ":" (to-string b)))))))""",
"""(define (p5-poskey w a b)
  (str-cat "6|POS:" (str-cat w (str-cat ":" (str-cat (to-string a)
    (str-cat ":" (to-string b)))))))

;; fee-snapshot keys: per (side,who,a,b,cell) growth checkpoint.
(define (p5-fskey w a b i) (p5-fsk "X" w a b i))
(define (p5-fskeyy w a b i) (p5-fsk "Y" w a b i))
(define (p5-fsk s w a b i)
  (str-cat "6|FS" (str-cat s (str-cat ":" (str-cat w (str-cat ":"
    (str-cat (to-string a) (str-cat ":" (str-cat (to-string b)
      (str-cat ":" (to-string i)))))))))))""",
    "fskey-trio")

# ═══ 10. feefold after reqy (before burnc) ═══
sub("""(define (p5-reqy i b s d acc)
  (if (> i b) acc
      (p5-reqy (+ i 1) b (p5-G (+ i 1)) d
               (li-add acc (p5-dy d s (p5-G (+ i 1)))))))""",
"""(define (p5-reqy i b s d acc)
  (if (> i b) acc
      (p5-reqy (+ i 1) b (p5-G (+ i 1)) d
               (li-add acc (p5-dy d s (p5-G (+ i 1)))))))

;; accruef fold (Aggregate_Accrual verbatim): owed_fee =
;; Σ (FG_i − FS_i) × d >> 64 — per-cell floor ⇒ under-credit only.
;; Then FS_i ← FG_i: the residual position earns from current growth.
;; Known v1 under-credit (pool-favoring): a top-up mint resets the
;; whole key's snapshots, forfeiting pre-top-up accrual.
(define (p5-feefold who a b d side)
  (let ((acc (loop ((i a) (s "0"))
               (if (> i b)
                   s
                   (recur (+ i 1)
                     (li-add s (p5-mulshr64
                                 (li-sub (p5-bz (str-cat "6|FG" (str-cat side
                                            (str-cat ":" (to-string i)))))
                                         (p5-bz (p5-fsk side who a b i)))
                                 d))))))))
    (begin
      (loop ((i a))
        (if (> i b)
            0
            (begin
              (p5-put (p5-fsk side who a b i)
                      (p5-bz (str-cat "6|FG" (str-cat side
                                (str-cat ":" (to-string i))))))
              (recur (+ i 1)))))
      ))""",
    "feefold")

# ═══ 11. mint-finish snapshots + FSP marker ═══
sub("""    (loop ((i a))
      (if (> i b)
          0
          (let ((k (p5-lqkey i)))
            (begin (p5-put k (li-add (p5-bz k) d)) (recur (+ i 1))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))""",
"""    (loop ((i a))
      (if (> i b)
          0
          (let ((k (p5-lqkey i)))
            (begin
              (p5-put k (li-add (p5-bz k) d))
              (p5-put (p5-fskey sender a b i) (p5-bz (str-cat "6|FGX:" i)))
              (p5-put (p5-fskeyy sender a b i) (p5-bz (str-cat "6|FGY:" i)))
              (recur (+ i 1))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))
    (p5-put (str-cat "6|FSP:" (str-cat sender
              (str-cat ":" (str-cat (to-string a)
                (str-cat ":" (to-string b)))))) "1")""",
    "mint-finish-snapshots")

# ═══ 12. burnc fee fold ═══
sub("""        ;; owed = the required amount, capped at the escrow float
        (let ((cx (p5-req-cap rx "6|ESCX")))
        (let ((cy (p5-req-cap ry "6|ESCY")))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy))))))))))))))))""",
"""        ;; fee fold (accruef) — LP trading fees, capped at the float;
        ;; raw folds persist in 6|FWX/6|FWY for callers/tests.
        (let ((fx (p5-feefold who a b d "X")))
        (let ((fy (p5-feefold who a b d "Y")))
        (begin
          (p5-put "6|FWX" fx)
          (p5-put "6|FWY" fy)
        ;; owed = required + fees, each capped at the escrow float
        (let ((cx (p5-req-cap (li-add rx fx) "6|ESCX")))
        (let ((cy (p5-req-cap (li-add ry fy) "6|ESCY")))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy)))))))))))))))))))
""",
    "burnc-feefold")

if fails:
    print("FAILED:")
    for f in fails:
        print(" ", f)
    raise SystemExit(1)

assert net(src) == 0, f"final net {net(src)}"
open(P, "w").write(src)
print(f"all anchors ok, paren-net 0, written {len(src)} (was {len(orig)})")
