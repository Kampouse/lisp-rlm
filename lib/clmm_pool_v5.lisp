;; ═══════════════════════════════════════════════════════════════════════
;; lib/clmm_pool_v5.lisp — dynamic CLMM pool, PROOF-FIT port (arc C1)
;;
;; Spec: afp-clmm/isabelle/CLMM_Dynamic.thy + CLMM_Reachable.thy (the boss).
;; Every op is a literal instance of a checked relation; violations are
;; REJECT + zero state change (identity discipline, same as WdFail):
;;
;;   state  = (P, s)   pool = (grd, lq, fee);  fee cells NEVER move here
;;             (CLMM_Reachable: fee mass is never grown by trading; the
;;              layer-2 accrual ledger is the separate, already-ported
;;              Aggregate_Accrual contract).
;;   swap_rel v P s P' s'  =  s' = s+v ∧ P' = refine P s' ∧ 0 < s'
;;             ∧ support ≠ {} ∧ grd_min P' ≤ s' ≤ grd_max P'.
;;             Swapping moves NO liquidity — pointwise book unchanged.
;;   mint_rel d a b   =  d ≥ 0 ∧ a ≤ b ∧ s' = s ∧ grd/fee unchanged
;;             ∧ ∀i. lq' i = (a≤i≤b ? lq i + d : lq i).   POINTWISE.
;;   burn_rel d a b   =  mint backwards ∧ ∀i. 0 ≤ lq' i ∧ support ≠ {}
;;             ∧ span holds after.
;;   donate_rel v i   =  v ≥ 0 ∧ s' = s ∧ only lq i moves: lq i + v.
;;
;; Grid: the fixed lattice grd(i) = S(i) = isqrt(1.0001^i · 2^128) (Q64.64,
;; u128), strictly monotone — a valid instantiation of the AFP grid locale
;; (the thy's examples use e^i; any smono lattice instantiates). Because the
;; lattice is fixed, `refine P s` degenerates to bookkeeping: the wedge cell
;; copies lq(lower_tick s) (refine_lq), so the ACTIVE liquidity is always
;; LQ:<P> with P = lower_tick(s) — exactly Uni-v3, exactly gen_quote_diff's
;; closed form (clmm_gen_quote_diff_eq: partial + fulls + partial).
;;
;; Quote rounding: per-cell floor — cell contribution = L·coverage >> 64
;; (y-side) / L·(ΔG)//(G·G') (x-side). Fail-closed (pool-favoring), the
;; same discipline proved for accruef's div_floor64 in Aggregate_Accrual.
;; The kernel adapters are the M2 parity-proven implementations.
;;
;; Span/support: IMIN/IMAX maintained exactly (bounded probe walk on
;; burn of an edge cell). Guards use thy bounds grd_min = G(IMIN),
;; grd_max = G(IMAX+1). Swap clamped to the span = maximal swap_rel
;; instance; remainder refunded (a smaller v is still a valid swap_rel).
;;
;; C1 SCOPE: gross `_run swap` is PROD-GATED on 6|SIMSWAP (sim-only arm;
;; near-mock state import seeds it, init never does) — books 6|X/6|Y move
;; against real tokens only via ft_on_transfer.
;; C2a (committed eeeb50eb): swap custody on the v4/NEP-141 pattern.
;; ft_on_transfer accepts TOKX/TOKY deposits: msg "seed" credits the pool
;; float (ESCX/ESCY — out-leg funding; escrow inflow mirrors the intake
;; side of each swap), msg "sw" = swap: escrow intake, walk, then promise
;; payout of `out` to sender. Payout failure (promise_result empty)
;; reverses the full leg: books/P/S restored, escrows unwound, ft_resolve
;; refunds the trader. POS:<who>:<a>:<b> = per-owner cumulative mint
;; (burn authority; the User_Ledger-flavored book).
;; C2b (this commit): mint/burn custody — REQUIRED-AMOUNT escrow. op "req"
;; {d,a,b} exposes rx/ry, the S-independent closed form both mint legs
;; and burnc owe. Mint = TWO legs (X/Y arrive in separate receipts):
;; msg "m:d:a:b" — leg 1 attaches EXACTLY its side's req (no refund path
;; exists for leg-1 excess), records 6|PM:<who> = "S:a:b:d:req" and
;; escrows req QUARANTINED in 6|ESCMX/6|ESCMY (outside the shared float,
;; so burns/claims/swaps can never pay out of pending-mint custody); leg
;; 2 attaches ≥ req, matches side+a+b+d, escrows its req in the float,
;; moves leg 1's slice into the float as position backing, completes
;; (exact op-mint core) and refunds excess via used.
;; msg "c" cancels: plain ft_transfer of the RECORDED slice, PM cleared +
;; ESCM drained only in the p5-pay "mk" success branch (failure leaves
;; pending+escrow intact for retry). Burn = op "burnc" (authority +
;; per-cell LQ guard + exact op-burn core; owed ledgers = req capped at
;; the escrow float) + op "claim" {t} per token: owed zeroed + escrow
;; debited, single payout leg; p5-pay "cx"/"cy" restores owed+escrow when
;; the receipt fails (single-leg reversal — two legs cannot be atomic).
;; Parity ops mint/burn untouched (driver oracle).
;; C2b+views (Oct 16): read-only surface for auditors/gauntlet — op
;; "escrow" { } → "ESCX:n:ESCY:n" (the custody pair in one call), op
;; "state" { } → view readout + :LAST: + :ESCX:/ESCY: + :TOKX:/TOKY:
;; (token contracts in charge). Both ZERO-write; pinned in
;; tests/test_pool_v5_views.rs + funded-flow parity in c2b.
;; Params + client views (Oct 17): init gains fee_shift (divisor 2^shift,
;; default/legacy = 8 ⇒ ÷256) and s0 (boot price, default 2^64); stored in
;; 6|SHIFT / 6|S, exposed via op "state" (:SHIFT: — effective value).
;; op "owed" {who} → "owed:<x>:<y>" (OW keys, one call); op "positions"
;; {who} → "pos:<a>:<b>:<d>;…" (poskey scan, d≠0; gas-bounded, see fn
;; comment). Views ZERO-write; pinned in tests/test_pool_v5_params.rs.
;; Fee generation (Oct 16): custody-path swaps skim F = intake >> SHIFT
;; (shift is the POOL PARAMETER above; legacy/default 8 = 1/256 floor).
;; The walker runs on v_eff = intake − F; F stays in the
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
;; The grid bid-ask spread (market sim) is SEPARATE and still pools.
;;
;; Call discipline: one value-producing call per let binding (compiler
;; sibling-call clobber — accrual contract note). `begin` wraps only
;; effect-only sequences. Kernel adapters are string-in/string-out.
;; ═══════════════════════════════════════════════════════════════════════

(define (p5-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (p5-put k v) (near/store-bytes k v))
(define (p5-fail m) (near/abort m))
(define (p5-ok m) m)

;; "acc|rm" packing: split at the FIRST | (acc never contains |)
(define (p5-fst res)
  (loop ((i 0))
    (if (str= (str-slice res i (+ i 1)) "|")
        (str-slice res 0 i)
        (recur (+ i 1)))))
(define (p5-snd res)
  (let ((k (str-len res)))
    (loop ((i (- k 1)))
      (if (str= (str-slice res i (+ i 1)) "|")
          (str-slice res (+ i 1) k)
          (recur (- i 1))))))

;; ── kernel adapters (M2 parity-proven; string in/out) ──────────────────

(define (p5-str6 s)
  (let ((b (buf-alloc 48)) (A (str->limbs s)))
    (begin (v5-zero! b 12) (v5-copy! (car A) b (car (cdr A))) b)))

(define (p5-buf-str b)
  (limbs->str b (if (> (v5-strip b 12) 0) (v5-strip b 12) 1)))

;; ── fee math (limb primitives; string-in/string-out) ──────────────────
;; fee skim: v >> SHIFT. SHIFT is a pool PARAMETER (key 6|SHIFT, set at
;; init, default 8 ⇒ 1/256 ≈ 0.39%; init 10 ⇒ 1/1024 ≈ 0.098%).
;; v5-divmod! is binary: the loop computes 2^SHIFT exactly (SHIFT ≤ 40),
;; so the knob never touches the bit-level limb math.
(define (p5-shf) (let ((sh (p5-bz "6|SHIFT")))
                   (if (= (str->num sh) 0) 8 (str->num sh))))
(define (p5-pow2 n)
  (loop ((i n) (acc 1))
    (if (= i 0) acc (recur (- i 1) (* 2 acc)))))
(define (p5-shr8 v)
  (let ((A (str->limbs v)) (tA (buf-alloc 48)) (tB (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 12) (v5-copy! (car A) tA (car (cdr A)))
      (v5-zero! tB 12) (limb-set! tB 0 (p5-pow2 (p5-shf)))
      (v5-zero! q 12) (v5-zero! r 10)
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
            (tF (buf-alloc 48)) (tB (buf-alloc 48)) (tC (buf-alloc 48)) (C64 (buf-alloc 16))
            (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
            (nbs (buf-alloc 64)) (s (buf-alloc 400)))
        (begin
          (v5-zero! tF 12) (v5-copy! (car FA) tF (car (cdr FA)))
          (v5-zero! tB 12) (v5-copy! (car LA) tB (car (cdr LA)))
          (p5-C64! C64)
          (v5-zero! tC 12)
          (limb-mul tF C64 tC 8 4)
          (v5-zero! q 12) (v5-zero! r 10)
          (v5-divmod! tC tB q r nbs s one)
          (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1))))))

(define (p5-mulshr64 x y)
  (let ((XA (str->limbs x)) (YA (str->limbs y))
        (tB (buf-alloc 48)) (tC (buf-alloc 64)) (C64 (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tC 16)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (p5-C64! C64)
      (v5-zero! tB 12) (v5-copy! C64 tB 4)
      (v5-zero! q 12) (v5-zero! r 10)
      (v5-divmod! tC tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))

;; S(i) = isqrt(1.0001^i · 2^128), |i| ≤ 800000 (kernel gates beyond)
(define (p5-G i)
  (let ((acc (buf-alloc 88)) (R (buf-alloc 88)) (P (buf-alloc 176))
        (t24 (buf-alloc 120)) (N (buf-alloc 64)) (nbs2 (buf-alloc 64))
        (s2 (buf-alloc 256)) (rem (buf-alloc 48)) (root (buf-alloc 48))
        (t0 (buf-alloc 48)) (t1 (buf-alloc 48)) (D (buf-alloc 88))
        (Bp (buf-alloc 88)) (Bn (buf-alloc 88)))
    (begin
      (v5-zero! D 22) (limb-set! D 12 1)
      (v5-zero! Bp 22)
      (let ((BP (str->limbs "1000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")))
        (v5-copy! (car BP) Bp (car (cdr BP))))
      (v5-zero! Bn 22)
      (let ((BA (str->limbs "10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")) (BO (buf-alloc 64)))
        (begin (v5-sdiv! (car BA) 10001 BO 13) (v5-copy! BO Bn 13)))
      (v5-copy! D acc 16)
      (v5-copy! Bp R 16)
      (if (< i 0) (v5-copy! Bn R 16) 0)
      (v5-pow-run! (if (< i 0) (- 0 i) i) acc R P)
      (v5-pow-final! acc t24 N)
      (v5-isqrt-dd! N 12 nbs2 s2 rem root t0 t1)
      (p5-buf-str root))))

;; dy = L·(S2−S) >> 64   (up-move output; down-move input as dyi)
(define (p5-dy L S S2)
  (let ((Lb (p5-str6 L)) (Sb (p5-str6 S)) (S2b (p5-str6 S2))
        (d (buf-alloc 32)) (P (buf-alloc 64)) (out (buf-alloc 64)))
    (begin (v5-dy! Lb Sb S2b d P out) (p5-buf-str out))))

;; dyi = L·(S−S2) >> 64
(define (p5-dyi L S S2)
  (p5-dy L S2 S))

;; dx = L·(S2−S) // (S·S2)   (up-move input)
(define (p5-dx L S S2)
  (let ((Lb (p5-str6 L)) (Sb (p5-str6 S)) (S2b (p5-str6 S2))
        (d (buf-alloc 32)) (P (buf-alloc 64)) (Q (buf-alloc 64))
        (out (buf-alloc 64)) (q (buf-alloc 64)) (r (buf-alloc 64))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)) (one (buf-alloc 8)))
    (begin (v5-dx! Lb Sb S2b d P Q out q r nbs s one) (p5-buf-str out))))

;; dxo = L·(S−S2) // (S·S2)  (down-move output)
(define (p5-dxo L S S2)
  (p5-dx L S2 S))

;; t* = (2^64·(dy+1) − 1) // L  into q  (exact dy-solver core)
(define (p5-tstar! dyB L tA tB tC q r nbs s one)
  (begin
    (v5-zero! tA 12)
    (v5-copy! dyB tA 4)
    (limb-set! one 0 1)
    (limb-add tA one tA 12 1)
    (v5-zero! tB 12) (limb-set! tB 0 1)
    (loop ((j 0))
      (if (>= j 64) 0 (begin (v5-double! tB 10) (recur (+ j 1)))))
    (v5-zero! tC 20)
    (limb-mul tA tB tC 6 10)
    (limb-set! one 0 1)
    (limb-sub tC one tC 10 1)
    (v5-zero! q 10) (v5-zero! r 10)
    (v5-divmod! tC L q r nbs s one)))

;; S_for_dx: largest S2 ≥ S with dx_in(L,S,S2) ≤ dx  [RENORMALIZED form]
;; dx_in = L·(S2−S)·2^64 // (S·S2) ≤ dx  ⟺  S2·D < N with
;;   D = L·2^64 − (dx+1)·S ,  N = L·S·2^64
;; ⇒ S2 = (N−1) div D  (mirrors sfdyi's −1 form); L=0 or D≤0 (dx ≥ cell
;;   total, unreachable) → 2^127 ; clamp ≥ S.
(define (p5-sfdx L S dx)
  (let ((C127 (p5-str6 "340282366920938463463374607431768211456"))
        (Lb (p5-str6 L)) (Sb (p5-str6 S)) (dxb (p5-str6 dx))
        (tA (buf-alloc 80)) (tB (buf-alloc 88)) (tC (buf-alloc 88))
        (tD (buf-alloc 88)) (tE (buf-alloc 80)) (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (if (or (<= (v5-strip Lb 12) 0) (<= (v5-strip Sb 12) 0))
          (begin (v5-zero! tE 12) (v5-copy! C127 tE 6) 0)
          (begin
            ;; tA = dx+1 (12-limb)
            (v5-zero! tA 12)
            (v5-copy! dxb tA 4)
            (limb-set! one 0 1)
            (limb-add tA one tA 12 1)
            ;; tB = (dx+1)·S  (6×6 → 12 limbs)
            (v5-zero! tB 20)
            (limb-mul tA Sb tB 6 12)
            ;; tC = L·2^64  (64 doublings)
            (v5-zero! tC 20)
            (v5-copy! Lb tC 6)
            (loop ((j 0))
              (if (>= j 64) 0 (begin (v5-double! tC 20) (recur (+ j 1)))))
            ;; D = L·2^64 − (dx+1)·S ; D ≤ 0 ⇒ unreachable → 2^127
            (limb-sub tC tB tC 20 12)
            (if (<= (v5-strip tC 20) 0)
                (begin (v5-zero! tE 12) (v5-copy! C127 tE 6) 0)
                (begin
                  ;; D = tC (normalize to 12 limbs top-down is fine: value
                  ;; fits — dx < cell total ⇒ D ≈ L·2^64)
                  (v5-zero! tB 20)
                  (v5-copy! tC tB 20)
                  ;; tD = L·S·2^64
                  (v5-zero! tD 20)
                  (limb-mul Lb Sb tD 6 6)
                  (loop ((j 0))
                    (if (>= j 64) 0 (begin (v5-double! tD 20) (recur (+ j 1)))))
                  ;; q = (tD − 1) div D
                  (limb-set! one 0 1)
                  (limb-sub tD one tD 20 1)
                  (v5-zero! q 10) (v5-zero! r 10)
                  (v5-divmod! tD tB q r nbs s one)
                  ;; tE = max(S, q)
                  (v5-zero! tE 12)
                  (v5-copy! q tE 10)
                  (if (< (limb-cmp tE Sb 10 6) 0)
                      (begin (v5-zero! tE 12) (v5-copy! Sb tE 6))
                      0)))))
      (p5-buf-str tE))))

;; S_for_dy_in: smallest S2 ≤ S with dy_in(L,S,S2) ≤ dy
;;   = L=0 → 1 ; else max(1, S − t*)
(define (p5-sfdyi L S dy)
  (let ((Lb (p5-str6 L)) (Sb (p5-str6 S)) (dyb (p5-str6 dy))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (if (<= (v5-strip Lb 12) 0)
          (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0)
          (begin
            (p5-tstar! dyb Lb tA tB tC q r nbs s one)
            (v5-zero! tD 12) (v5-copy! Sb tD 6)
            (if (> (limb-cmp tD q 10 10) 0)
                (begin (v5-zero! tE 12) (limb-sub tD q tE 10 10) 0)
                (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0))))
      (p5-buf-str tE))))

(define (li-add a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-add (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (li-sub a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-sub (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (li-cmp a b)
  (let ((A (str->limbs a)) (B (str->limbs b)))
    (limb-cmp (car A) (car B) (car (cdr A)) (car (cdr B)))))


;; ── li helpers (limb_math string ops) ──────────────────────────────────
;; li-add/li-sub/li-cmp: arbitrary-precision decimal strings.

(define (p5-lqkey i) (str-cat "6|LQ:" (to-string i)))
(define (p5-poskey w a b)
  (str-cat "6|POS:" (str-cat w (str-cat ":" (str-cat (to-string a)
    (str-cat ":" (to-string b)))))))

;; fee-snapshot keys: per (side,who,a,b,cell) growth checkpoint.
(define (p5-fskey w a b i) (p5-fsk "X" w a b i))
(define (p5-fskeyy w a b i) (p5-fsk "Y" w a b i))
(define (p5-fsk s w a b i)
  (str-cat "6|FS" (str-cat s (str-cat ":" (str-cat w (str-cat ":"
    (str-cat (to-string a) (str-cat ":" (str-cat (to-string b)
      (str-cat ":" (to-string i)))))))))))

;; ── support bounds: exact, bounded probes ──────────────────────────────
;; IMIN/IMAX cached; burn of an edge cell walks inward (≤ 4096 probes).

(define (p5-lq-nz? i) (not (= (li-cmp (p5-bz (p5-lqkey i)) "0") 0)))

(define (p5-imin) (str->num (p5-bz "6|IMIN")))

(define (p5-imax) (str->num (p5-bz "6|IMAX")))

;; F // k for small k (≤ 65)
(define (p5-quotk F k)
  (let ((FA (str->limbs F)) (tA (buf-alloc 48)) (tB (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 12) (v5-copy! (car FA) tA (car (cdr FA)))
      (v5-zero! tB 12) (limb-set! tB 0 k)
      (v5-zero! q 12) (v5-zero! r 10)
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
                (recur (+ i 1)))))))))

;; shrink IMIN upward to the first nonzero cell (≤4096 probes); 0 if none
(define (p5-shrink-imin lo hi)
  (loop ((i lo) (n 0))
    (if (> n 4096) 0
    (if (> i hi) 0
        (if (p5-lq-nz? i)
            i
            (recur (+ i 1) (+ n 1)))))))

;; shrink IMAX downward to the last nonzero cell (≤4096 probes); 0 if none
(define (p5-shrink-imax lo hi)
  (loop ((i hi) (n 0))
    (if (> n 4096) 0
    (if (< i lo) 0
        (if (p5-lq-nz? i)
            i
            (recur (- i 1) (+ n 1)))))))

;; respan after burn: IMIN/IMAX = exact nonzero support, 0/0 when empty
(define (p5-respan!)
  (let ((lo (p5-imin)) (hi (p5-imax)))
    (let ((nlo (p5-shrink-imin lo hi)) (nhi (p5-shrink-imax lo hi)))
      (begin
        (p5-put "6|IMIN" (to-string nlo))
        (p5-put "6|IMAX" (to-string nhi))
        0))))

;; ── ops ────────────────────────────────────────────────────────────────

(define (p5-init)
  (if (nil? (near/load-bytes "6|TOKX"))
      (let ((tokx (json-get-str "toka" (near/input)))
            (toky (json-get-str "tokb" (near/input)))
            ;; bonding-curve graduation account (empty = provision dead)
            (cv (json-get-str "curve" (near/input)))
            ;; pool PARAMS: fee divisor = 2^fee_shift, boot price = s0.
            ;; Defaults: shift 8 (1/256 ≈ 0.39%), s0 = 2^64 (price 1.0).
            (sh (json-get-str "fee_shift" (near/input)))
            (s0 (json-get-str "s0" (near/input))))
        (begin
          (p5-put "6|TOKX" tokx)
          (p5-put "6|TOKY" toky)
          (if (not (str= cv ""))
              (p5-put "6|CURVE" cv)
              0)
          ;; SHIFT: empty/0 input falls back to 8 via p5-shf — pre-param
          ;; pools (and legacy init callers) keep the ÷256 skim exactly.
          (p5-put "6|SHIFT"
                  (if (= (str->num sh) 0) "" (if (< (str->num sh) 41) sh "")))
          ;; s0 = boot price; empty ⇒ 2^64 (price 1.0)
          (p5-put "6|S" (if (= (str-len s0) 0) "18446744073709551616" s0))
          ;; LP lock: lock_secs (duration) ⇒ burn-until = init_now + secs.
          ;; Empty/0 = unlocked. Set ONCE at init — immutable afterwards;
          ;; a launch pool's LP cannot pull liquidity before the lock even
          ;; if every key it owns is compromised (enforcement is HERE, in
          ;; the pool, not in a factory promise).
          (p5-put "6|LOCK"
                  (let ((secs (str->num (json-get-str "lock_secs" (near/input)))))
                    (if (> secs 0)
                        (li-add (near/block_timestamp) (to-string secs))
                        "0")))
          (p5-put "6|P" "0")
          (p5-put "6|X" "0")
          (p5-put "6|Y" "0")
          (p5-put "6|IMIN" "0")
          (p5-put "6|IMAX" "0")
          "init-ok"))
      "already-initialized"))

;; mint_rel d a b — pointwise; width ≤ 64 cells (gas bound, documented)
(define (p5-mint)
  (let* ((inp (near/input))
         (who (near/predecessor_account_id))
         (d (json-get-str "d" inp))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
    (if (or (< b a) (> (- b a) 64) (not (= (li-cmp d "0") 1)))
        (p5-fail "ERR_RANGE")
        (let ((pos (p5-bz (p5-poskey who a b))))
          (begin
            ;; pointwise: LQ i += d for a ≤ i ≤ b
            (loop ((i a))
              (if (> i b) 0
                  (let ((k (p5-lqkey i)))
                    (begin
                      (p5-put k (li-add (p5-bz k) d))
                      (recur (+ i 1))))))
            (p5-put (p5-poskey who a b) (li-add pos d))
            ;; support bounds widen only
            (if (< a (str->num (p5-bz "6|IMIN")))
                (p5-put "6|IMIN" (to-string a))
                0)
            (if (> b (str->num (p5-bz "6|IMAX")))
                (p5-put "6|IMAX" (to-string b))
                0)
            (str-cat "mint-ok:" d))))))

;; burn_rel d a b — authority + per-cell non-negativity + support + span
(define (p5-burn)
  (let* ((inp (near/input))
         (who (near/predecessor_account_id))
         (d (json-get-str "d" inp))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
    ;; thy burn_rel: guards = range, per-cell 0 ≤ lq' i, nonzero support,
    ;; span bounds. Authority lives ABOVE the relation (access control, not
    ;; pool math — cf. auth_burn_rel in CLMM_Dynamic.thy): the caller must
    ;; own ≥ d at [a,b] per the POS custody keys, mirroring p5-burnc.
    ;; v4 carried this check; it was dropped as "beyond the proof" and the
    ;; 2026-10-17 audit flagged the result as a griefing vector (Finding A).
    (if (or (< b a) (> (- b a) 64))
        (p5-fail "ERR_RANGE")
    (if (< (li-cmp (p5-bz (p5-poskey who a b)) d) 0)
        (p5-fail "ERR_AUTH")
    ;; per-cell guard ∀i∈[a,b]. lq i ≥ d  (relation: 0 ≤ lq' i)
    (if (= (loop ((i a))
                 (if (> i b) 1
                     (if (< (li-cmp (p5-bz (p5-lqkey i)) d) 0)
                         0
                         (recur (+ i 1)))))
           0)
        (p5-fail "ERR_LIQ")
    (let ((pre-imin (p5-imin)) (pre-imax (p5-imax)))
      (begin
        ;; pointwise subtract
        (loop ((i a))
          (if (> i b) 0
              (let ((k (p5-lqkey i)))
                (begin
                  (p5-put k (li-sub (p5-bz k) d))
                  (recur (+ i 1))))))
        ;; custody: the caller's POS position shrinks by d (burnc parity —
        ;; burn consumes the caller's own liquidity, not the cell ledger's)
        (p5-put (p5-poskey who a b)
                (li-sub (p5-bz (p5-poskey who a b)) d))
        ;; support: shrink edges exactly — the thy span is IMIN/IMAX
        ;; themselves; walk inward to the first/last nonzero cell, 0/0
        ;; when the span emptied (oracle: support None ≡ bounds 0/0).
        (p5-respan!)
        (str-cat "burn-ok:" d))))))))

;; ── swap: swap_rel v P s P' s', clamped to the span ────────────────────
;; direction up: v > 0, sell x (input X ≤ v) → out Y.
;; direction down: v < 0, sell y → out X.
;; The walk: current cell P (active L = LQ:<P>), cell covers
;; [G(P), G(P+1)) going up / [G(P-1), G(P)) going down. Per-cell output
;; floors (fail-closed). Each leg moves s; the book is untouched.
;; ── swap: swap_rel v P s P' s', clamped to the span ────────────────────
;; direction up: v > 0, sell x (input X ≤ v) → out Y.
;; direction down: v < 0, sell y → out X.
;; The walk: current cell P (active L = LQ:<P>), cell covers
;; [G(P), G(P+1)) going up / [G(P-1), G(P)) going down. Per-cell output
;; floors (fail-closed). Each leg moves s; the book is untouched.
;; Emitter note (ft2 arc, revised 2026-10-15): the "let* slot collision"
;; was a misdiagnosis — the repro files were mis-parenthesized sources the
;; compiler faithfully compiled. The detectable corruption form (4+
;; positional if arms) is now a hard error (arity cap, cf7f60e3); the
;; 2-form-if / 2-body-let* shape is still legal lisp — paren discipline
;; still matters. let* itself is safe (p5-burn uses it, and the walk arms
;; were converted to let* in 88e3ec17 once the phantom repro was buried).
;; Loops return ONE packed string.
;;
;; walk arm for neg=1 (down): returns "acc|rm" packed, puts P/S.
;; cap = X out-float (ESCX) for custody swaps, "-1" = uncapped (calc/sim).
;; NOTE the sentinel: a REAL float of zero ("0") must cap the walk at
;; nothing — an x1 position has no ESCY at all and p5-bz answers "0", so
;; "0" must MEAN zero, never "uncapped" (g_x1_unfillable, 2026-10-20).
;; The walker refuses any cell whose out-leg (dxo) would exceed the
;; remaining float: it stops at the cell edge and returns the unconsumed
;; input as rem. Without this, a sweep against insufficient float paid out
;; more than the pool held — accounting walked, the token transfer bounced
;; "insufficient", and books went dirty (g_two_sweepup, 2026-10-20).
(define (p5-swarm-dn s p xbook ybook v feep? cap)
  (let* ((F (if feep? (p5-shr8 v) "0"))
         (v (if feep? (li-sub v F) v))
         (imin (p5-imin))
         (gmin (p5-G imin))
         (res (loop ((pp p) (ss s) (acc "0") (rm v) (v0 v))
                 (if (= (li-cmp rm "0") 0)
                     (if (and (= (li-cmp acc "0") 0) (= (li-cmp rm v0) 0))
                         (str-cat acc (str-cat "|" rm))
                         (begin (p5-put "6|P" (to-string pp))
                                (p5-put "6|S" ss)
                                (str-cat acc (str-cat "|" rm))))
                 (if (or (< (li-cmp ss gmin) 1) (<= pp imin))
                     (if (and (= (li-cmp acc "0") 0) (= (li-cmp rm v0) 0))
                         (str-cat acc (str-cat "|" rm))
                         (begin (p5-put "6|P" (to-string pp))
                                (p5-put "6|S" ss)
                                (str-cat acc (str-cat "|" rm))))
                 (let* ((L (p5-bz (p5-lqkey pp)))
                 (gl (p5-G (- pp 1)))
                 (dxo (p5-dxo L ss gl))
                 (dyi (p5-dyi L ss gl)))
                   (if (and (not (str= cap "-1"))
                            (> (li-cmp (li-add acc dxo) cap) 0))
                       (begin (p5-put "6|P" (to-string pp))
                              (p5-put "6|S" ss)
                              (str-cat acc (str-cat "|" rm)))
                   (if (< (li-cmp dyi rm) 1)
                       (recur (- pp 1) gl (li-add acc (if (= (li-cmp dyi "0") 0) "0" dxo))
                              (li-sub rm dyi)
                              v0)
                       (let ((s2 (p5-sfdyi L ss rm)))
                         (begin
                           ;; s2 lands strictly INSIDE cell pp: the full-cell
                           ;; dyi exceeded rm, else the branch above hopped —
                           ;; so gl < s2 ≤ ss < G(pp). The active cell is the
                           ;; one CONTAINING the new price ⇒ P stays pp.
                           ;; (Old pp-1 stranded P one cell below S and froze
                           ;; later up-walks — seeds 16/34, 2026-10-17.)
                           (p5-put "6|P" (to-string pp))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dxo L ss s2))
                                    (str-cat "|" "0"))))))))))))
  (let* ((out (p5-fst res))
         (rmf (p5-snd res))
         (xnew (li-add xbook out))
         (yin (li-sub v rmf))
         (msg (str-cat "swap-out:" (str-cat out (str-cat ":rem:" rmf)))))
    (if (= (li-cmp rmf v) 0)
        msg
        (begin
          (p5-put "6|OUT" out)
          (p5-put "6|REM" rmf)
          (p5-put "6|X" xnew)
          (p5-put "6|Y" (li-add ybook yin))
          (p5-put "6|LAST" msg)
          (if feep?
              (begin
                (p5-credit! "Y" F (str->num (p5-bz "6|P")) p)
                (p5-put "6|FEEY" (li-add (p5-bz "6|FEEY") F))
                0)
              0)
          msg)))))

;; walk arm for neg=0 (up): returns "acc|rm" packed, puts P/S.
;; cap = Y out-float (ESCY) for custody swaps, "-1" = uncapped (calc/sim).
;; Same float guard as p5-swarm-dn (see the sentinel note there).
(define (p5-swarm-up s p xbook ybook v feep? cap)
  (let* ((F (if feep? (p5-shr8 v) "0"))
         (v (if feep? (li-sub v F) v))
         (imax (p5-imax))
         (gmax (p5-G (+ imax 1)))
         (res (loop ((pp p) (ss s) (acc "0") (rm v) (v0 v))
                 (if (= (li-cmp rm "0") 0)
                     (if (and (= (li-cmp acc "0") 0) (= (li-cmp rm v0) 0))
                         (str-cat acc (str-cat "|" rm))
                         (begin (p5-put "6|P" (to-string pp))
                                (p5-put "6|S" ss)
                                (str-cat acc (str-cat "|" rm))))
                 (if (or (>= (li-cmp ss gmax) 0) (>= pp (+ imax 1)))
                     (if (and (= (li-cmp acc "0") 0) (= (li-cmp rm v0) 0))
                         (str-cat acc (str-cat "|" rm))
                         (begin (p5-put "6|P" (to-string pp))
                                (p5-put "6|S" ss)
                                (str-cat acc (str-cat "|" rm))))
                 (let* ((L (p5-bz (p5-lqkey pp)))
                 (gh (p5-G (+ pp 1)))
                 (dy (p5-dy L ss gh))
                 (dx (p5-dx L ss gh)))
                   (if (and (not (str= cap "-1"))
                            (> (li-cmp (li-add acc dy) cap) 0))
                       (begin (p5-put "6|P" (to-string pp))
                              (p5-put "6|S" ss)
                              (str-cat acc (str-cat "|" rm)))
                   (if (< (li-cmp dx rm) 1)
                       (recur (+ pp 1) gh (li-add acc (if (= (li-cmp dx "0") 0) "0" dy))
                              (li-sub rm dx)
                              v0)
                       (let ((s2 (p5-sfdx L ss rm)))
                         (begin
                           (p5-put "6|P" (to-string pp))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dy L ss s2))
                                    (str-cat "|" "0"(str-cat "|" "0")))))))))))))
  (let* ((out (p5-fst res))
         (rmf (p5-snd res))
         (ynew (li-add ybook out))
         (xin (li-sub v rmf))
         (msg (str-cat "swap-out:" (str-cat out (str-cat ":rem:" rmf)))))
    (if (= (li-cmp rmf v) 0)
        msg
        (begin
          (p5-put "6|OUT" out)
          (p5-put "6|REM" rmf)
          (p5-put "6|Y" ynew)
          (p5-put "6|X" (li-add xbook xin))
          (p5-put "6|LAST" msg)
          (if feep?
              (begin
                (p5-credit! "X" F p (str->num (p5-bz "6|P")))
                (p5-put "6|FEEX" (li-add (p5-bz "6|FEEX") F))
                0)
              0)
          msg)))))

;; PROD-GATED (Finding C, 2026-10-18): the gross arm moves books/P/S with
;; no token attached — sim-settling. Live it fires ONLY when 6|SIMSWAP is
;; set; init NEVER writes that key and no op can, so the prod arm is
;; permanently dead. near-mock drivers (differential, edge battery)
;; seed 6|SIMSWAP="1" via `state import` and keep the oracle channel.
(define (p5-swap)
  ;; PROD-GATE (Finding C, 2026-10-18): the gross arm moves books/P/S with
  ;; no token attached — sim-settle. Live it fires ONLY when 6|SIMSWAP="1";
  ;; init never writes that key and no op can, so the prod arm is dead.
  ;; near-mock drivers (differential + edge battery) seed it via state
  ;; import and keep the oracle channel.
  (if (str= (p5-bz "6|SIMSWAP") "1")
      (let ((inp (near/input)))
      (let ((v (json-get-str "v" inp)))
      (let ((neg (if (str= (str-slice v 0 1) "-") 1 0)))
      (let ((v (if (= neg 1) (str-slice v 1 (str-len v)) v)))
      (let ((s (p5-bz "6|S")))
      (let ((p (str->num (p5-bz "6|P"))))
      (let ((xbook (p5-bz "6|X")))
      (let ((ybook (p5-bz "6|Y")))
        (if (= (str-len (p5-bz "6|TOKX")) 0)
            "ERR_INIT"
        (if (= (li-cmp v "0") 0)
            "swap-zero"
        (if (= neg 1)
            (p5-swarm-dn s p xbook ybook v 0 "-1")
            (p5-swarm-up s p xbook ybook v 0 "-1"))))))))))))
       "ERR_SIM_ONLY"))
;; ── custody (arc C2a): swap leg only ───────────────────────────────────
;; NEP-141 entry: predecessor must be 6|TOKX or 6|TOKY; msg side-tags the
;; operation ("sw"). 6|X/6|Y are flow ledgers (both rise on every swap),
;; NOT reserves — trader token custody lives in 6|ESCX/6|ESCY, fed only by
;; ft_on_transfer, drained only by payout ft_transfers.
;; Refund coupling (ft2 resolve semantics): ft_resolve_transfer reads the
;; ft_on_transfer receipt's CHAIN RESULT — the payout callback's own
;; near/return, not intermediate returns. So p5-pay re-returns the refund
;; string on every path: success ⇒ rm (remainder back to trader); payout
;; failure ⇒ full intake c+rm (v4 returned only rm there and leaked the
;; consumed slice — fixed here), plus ledger reversal (X-=c, Y-=o) and
;; escrow unwound (-rm). Reject/empty-walk ⇒ near/return amt directly.
(define (p5-num-ok? s)
  ;; <=40 digits, ASCII only — OOB-parse & u128-scale guard (v4 pattern)
  (if (> (str-length s) 40)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9))
                  "0"
                  (recur (+ i 1))))))))

;; index of next occurrence of `sep` byte at/after `from` (str-length if none)
(define (p5-seek s from sep)
  (loop ((j from))
    (if (>= j (str-length s))
        j
        (if (= (byte-at s j) sep) j (recur (+ j 1))))))

;; nth `sep`-field of s, 0-based. Swarm arms return OUT|RM (pipe-packed).
(define (p5-fld s i sep)
  (loop ((k 0) (start 0))
    (if (> k i)
        ""
        (let ((e (p5-seek s start sep)))
          (if (= k i)
              (str-slice s start e)
              (recur (+ k 1) (+ e 1)))))))

;; donate_rel v i — lq i += v, nothing else. CUSTODY (Finding B fix,
;; 2026-10-18): donations ride ft_transfer_call msg "don:<i>" (predecessor
;; = a pool token, amount validated) so LQ only grows against escrowed
;; tokens; the _run variant is gone. v = amt, i = parsed from msg; failure
;; to parse ⇒ full refund, zero state.
(define (p5-donate-leg amt)
  (let ((rest (json-get-str "i" (json-get-str "msg" (near/input)))))
  (let ((ok (p5-num-ok? rest)))
  (let ((i (str->num rest)))
    (if (or (= ok "0")
            (= (str-len rest) 0)
            (= (li-cmp amt "0") 0))
        (near/return amt)
    (let ((k (p5-lqkey i)))
      (begin
        (p5-put k (li-add (p5-bz k) amt))
        (if (< i (str->num (p5-bz "6|IMIN")))
            (p5-put "6|IMIN" (to-string i))
            0)
        (if (> i (str->num (p5-bz "6|IMAX")))
            (p5-put "6|IMAX" (to-string i))
            0)
        (near/return "0"))))))))
(define (p5-on-transfer)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (str= (p5-num-ok? amt) "0")
            (near/return amt)
            (if (str= (near/predecessor_account_id) (p5-bz "6|TOKX"))
                (if (str= (json-get-str "a" (json-get-str "msg" inp)) "swap")
                    (p5-swap-x sender amt (json-get-str "mo" (json-get-str "msg" inp)))
                    (if (str= (json-get-str "a" (json-get-str "msg" inp)) "seed")
                        (begin
                          ;; pool-owned float, no position (bootstrap for
                          ;; swap out-legs; mint custody escrows its own)
                          (p5-put "6|ESCX" (li-add (p5-bz "6|ESCX") amt))
                          (near/return "0"))
                        (if (str= (json-get-str "a" (json-get-str "msg" inp)) "donate")
                            (p5-donate-leg amt)
                            (p5-mint-leg sender amt))))
                (if (str= (near/predecessor_account_id) (p5-bz "6|TOKY"))
                    (if (str= (json-get-str "a" (json-get-str "msg" inp)) "swap")
                        (p5-swap-y sender amt (json-get-str "mo" (json-get-str "msg" inp)))
                        (if (str= (json-get-str "a" (json-get-str "msg" inp)) "seed")
                            (begin
                              (p5-put "6|ESCY" (li-add (p5-bz "6|ESCY") amt))
                              (near/return "0"))
                            (if (str= (json-get-str "a" (json-get-str "msg" inp)) "donate")
                                (p5-donate-leg amt)
                                (p5-mint-leg sender amt))))
                    (near/return amt))))))))

;; sell X for Y: escrow intake, walk via the same core as op "swap", then
;; promise the out-leg from 6|ESCY. Chain result (p5-pay's return) is what
;; ft2's resolve refunds.
(define (p5-swap-x sender amt mo)
  (let ((s (p5-bz "6|S")))
  (let ((p (str->num (p5-bz "6|P"))))
  (let ((xbook (p5-bz "6|X")))
  (let ((ybook (p5-bz "6|Y")))
  (let ((ps0 (p5-bz "6|S")))
  (let ((pl0 (p5-bz "6|P")))
    (if (nil? (near/load-bytes "6|TOKX"))
        (near/return amt)
    (if (= (li-cmp amt "0") 0)
        (near/return amt)
        (begin
          ;; v4 DY/REM pattern: sentinel the result keys, walk, read back.
          (p5-put "6|OUT" "D")
          (p5-put "6|REM" "D")
          (p5-put "6|ESCX" (li-add (p5-bz "6|ESCX") amt))
          ;; up sells X for Y (fee path)
          (p5-swarm-up s p xbook ybook amt 1 (p5-bz "6|ESCY"))
          (p5-leg-decide sender amt "6|X" "6|ESCX" "6|ESCY" ps0 pl0 "6|TOKY" mo)))))))))))

;; sell Y for X — mirror of p5-swap-x (out-leg from 6|ESCX on 6|TOKX)
(define (p5-swap-y sender amt mo)
  (let ((s (p5-bz "6|S")))
  (let ((p (str->num (p5-bz "6|P"))))
  (let ((xbook (p5-bz "6|X")))
  (let ((ybook (p5-bz "6|Y")))
  (let ((ps0 (p5-bz "6|S")))
  (let ((pl0 (p5-bz "6|P")))
    (if (nil? (near/load-bytes "6|TOKX"))
        (near/return amt)
    (if (= (li-cmp amt "0") 0)
        (near/return amt)
        (begin
          (p5-put "6|OUT" "D")
          (p5-put "6|REM" "D")
          (p5-put "6|ESCY" (li-add (p5-bz "6|ESCY") amt))
          ;; down sells Y for X (fee path)
          (p5-swarm-dn s p xbook ybook amt 1 (p5-bz "6|ESCX"))
          (p5-leg-decide sender amt "6|Y" "6|ESCY" "6|ESCX" ps0 pl0 "6|TOKX" mo)))))))))))

;; shared leg decision — reads 6|OUT/6|REM via bz (v4 pattern: NO
;; return-string parsing inside receipts):
;;   sentinel ⇒ walker moved nothing: undo escrow, refund all;
;;   out == 0 ⇒ degenerate (intake consumed, output floored): undo the
;;     intake-book delta + P/S + escrow, refund all;
;;   else ⇒ release the refund slice (-rm; consumed c stays escrowed),
;;     full reversal (X/Y/P/S + escrow -rm) on payout failure.
;; mo = optional min-out floor from the swap msg ("mo": "<amount>").
;; Empty/absent ⇒ no guard (byte-identical to pre-mo behavior). When set
;; and the walk's out < mo, p5-fail aborts the WHOLE receipt: every write
;; in this transaction — escrow credit, books, P/S, fee credit — reverts,
;; and ft2's resolve sees an empty chain result ⇒ full refund INCLUDING
;; the fee slice. Sentinel/degenerate paths run before the guard and keep
;; their refund-all semantics.
(define (p5-leg-decide sender amt ibook ekey okey ps0 pl0 tokkey mo)
  (let ((out (p5-bz "6|OUT")))
    (if (str= out "D")
        (begin
          (p5-put ekey (li-sub (p5-bz ekey) amt))
          (near/return amt))
        (if (= (li-cmp out "0") 0)
            (begin
              (p5-put ibook (li-sub (p5-bz ibook) (li-sub amt (p5-bz "6|REM"))))
              (p5-put "6|P" pl0)
              (p5-put "6|S" ps0)
              (p5-put ekey (li-sub (p5-bz ekey) amt))
              (near/return amt))
            ;; fee path: the walker ran on v_eff = amt − F, so REM is the
            ;; remainder OF v_eff — amt − REM retains exactly F + walked
            ;; (skim stays escrowed, LP-claimable via FG; raw==ESCX holds).
            ;; intake escrow keeps the CONSUMED slice (c = F + walked —
            ;; the pool's intake-side float, backs owed); the REFUND slice
            ;; leaves via ft2's resolve (sub-bal recv rm) — the ledger must
            ;; release it here or it overstates custody by rm - c.
            ;; NOTE: mo parsed by li-cmp only — str->num is u64 and would
            ;; trap on any real token amount; mo "0"/absent ⇒ guard inert
            ;; (out < 0 is impossible). Canonical decimal expected.
            (if (and (> (str-len mo) 0) (< (li-cmp out mo) 0))
                ;; min-out violated: abort the whole receipt — every write
                ;; in this leg (escrow credit, books, P/S, fee) reverts and
                ;; ft2's resolve sees an empty result ⇒ full refund incl. fee
                (p5-fail "ERR_MO")
                (let ((c (li-sub amt (p5-bz "6|REM"))))
                  (begin
                    (p5-put ekey (li-sub (p5-bz ekey) (p5-bz "6|REM")))
                    (p5-payout sender out (p5-bz "6|REM") c ps0 pl0 ekey okey tokkey))))))))

;; promise the out-leg, then the pay callback. Chain result = pay's return.
;; oe = the OUT-side escrow key (drained on payout success).
(define (p5-payout sender out rm c ps pl ekey oe tokkey)
  (let ((p (near/promise_batch_create (p5-bz tokkey))))
    (begin
      (near/promise_batch_action_function_call p "ft_transfer"
        (json-set (json-set "{}" "receiver_id" sender) "amount" out)
        "0" 40000000000000)
      (near/promise_return
        (near/promise_then p (near/current_account_id) "pay"
          (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}"
            "r" sender) "o" out) "m" rm) "c" c) "ps" ps) "pl" pl) "e" ekey) "w" oe)
            ;; k tags the custody case — pay dispatches on it (sw = swap
            ;; reversal/refund; missing k used to fall into the cx/cy
            ;; branch and consume the full intake).
            "k" "sw")
          "0" 5000000000000)))))

;; payout callback (internal only — self-call gate). promise_result(0)
;; empty ⇒ the payout receipt failed: dispatch on k (sw = full C2a swap
;; reversal + full-intake refund; mk = re-credit mint-cancel escrow;
;; cx/cy = restore claim owed + escrow). Success ⇒ C2a returns rm (the
;; trader's remainder rides resolve); mk/cx/cy consume (return "0").
;; Both paths return via near/return: the chain result is this value.
;; Reversal needs the walker's terminal P/S — snapshot them BEFORE the
;; walk (p5-payout's ps/pl args) since the walker itself puts them.
(define (p5-pay)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
        (let ((r (near/promise_result 0)))
          (if (< (str-length r) 1)
              ;; payout FAILED — dispatch on the custody case k:
              ;;   sw (C2a): reverse ledger + P/S, unwind intake escrow,
              ;;     refund the FULL intake (c+m) via the chain result;
              ;;   mk: mint-cancel refund failed — re-credit the escrow;
              ;;   cx/cy: claim failed — restore owed + escrow.
              (if (str= (json-get-str "k" inp) "sw")
                  (begin
                    (p5-put "6|X" (li-sub (p5-bz "6|X") (json-get-str "c" inp)))
                    (p5-put "6|Y" (li-sub (p5-bz "6|Y") (json-get-str "o" inp)))
                    (p5-put "6|P" (json-get-str "pl" inp))
                    (p5-put "6|S" (json-get-str "ps" inp))
                    (p5-put (json-get-str "e" inp)
                      (li-sub (p5-bz (json-get-str "e" inp)) (json-get-str "c" inp)))
                    (near/return (li-add (json-get-str "c" inp) (json-get-str "m" inp))))
                  (if (str= (json-get-str "k" inp) "mk")
                      ;; mint-cancel refund failed: restore NOTHING — the
                      ;; pending record + escrow survive (retry or leg 2).
                      (near/return "cancel-failed")
                      (begin
                        (p5-put (json-get-str "w" inp)
                          (li-add (p5-bz (json-get-str "w" inp))
                                  (json-get-str "o" inp)))
                        (p5-put (json-get-str "e" inp)
                          (li-add (p5-bz (json-get-str "e" inp))
                                  (json-get-str "o" inp)))
                        (near/return "0"))))
              (begin
                ;; payout OK — dispatch on k:
                ;;   sw: drain the out-side escrow by `out`; the trader's
                ;;     rm refund flows through resolve (return rm).
                ;;   mk: cancel refund delivered — clear the pending
                ;;     record + drain the escrow slice.
                ;;   cx/cy: owed/escrow were netted at claim time — consume.
                (if (str= (json-get-str "k" inp) "sw")
                    (begin
                      (p5-put (json-get-str "w" inp)
                        (li-sub (p5-bz (json-get-str "w" inp)) (json-get-str "o" inp)))
                      (near/return (json-get-str "m" inp)))
                    (if (str= (json-get-str "k" inp) "mk")
                        (begin
                          ;; cleared marker = "0" (bz's missing value):
                          ;; storing "" traps the host store
                          (p5-put (str-cat "6|PM:" (json-get-str "r" inp)) "0")
                          (p5-put (json-get-str "w" inp)
                            (li-sub (p5-bz (json-get-str "w" inp))
                                    (json-get-str "d" inp)))
                          (near/return "0"))
                        ;; cx/cy success: owed/escrow were netted at claim
                        ;; time - drain the side's owed total so burnc's
                        ;; room guard sees the freed float room
                        (begin
                          (p5-put (if (str= (json-get-str "k" inp) "cx")
                                      "6|OWTX" "6|OWTY")
                                  (li-sub (p5-bz (if (str= (json-get-str "k" inp) "cx")
                                                     "6|OWTX" "6|OWTY"))
                                          (json-get-str "o" inp)))
                          (near/return "0"))))))))
      (near/return "forbidden")))

;; ── custody (arc C2b): mint/burn legs ──────────────────────────────────
;; Positions are two-token liabilities: X and Y arrive in SEPARATE
;; ft_transfer_call receipts, so mint custody is a two-leg protocol and
;; burn custody is accounting + per-token claims (two payout legs cannot
;; be atomically reversed — each claim leg reverses ITSELF on failure).
;;
;; MINT (msg "m:d:a:b" on either token; REQUIRED-amount custody —
;; op "req" {d,a,b} gives rx/ry, the S-independent closed form both
;; legs charge and burnc refunds; leg 1 attaches EXACTLY its side's req
;; (its excess could never ride a refund); leg 2 attaches ≥ req,
;; excess refunded via its chain result; cancel msg "c" attaches 0):
;;   leg 1 records 6|PM:<sender> = "<X|Y>:a:b:d" and escrows req on its
;;   side QUARANTINED in 6|ESCM<side> (never swappable/burnable float);
;;   leg 2 matches a/b and the opposite side, requires ≥ req, and
;;   completes the position (exact op-mint core), moving leg 1's slice
;;   into the float as backing. Both sides' REQUIRED
;;   slices stay escrowed — burn claims pay req back (same closed form,
;;   so custody is symmetric; swap PnL lives in the books, not custody).
;;   Mismatch / short / re-entry / bad shape ⇒ p5-fail
;;   (resolve refunds that leg whole).
;;   Cancel refunds the recorded slice via plain ft_transfer; state is
;;   touched ONLY in the callback (success clears PM + drains escrow;
;;   failure leaves the pending mint intact for a retry).
;; BURN:
;;   op "burnc" {d,a,b} — authority (POS ≥ d) + the EXACT op-burn core,
;;   then owed ledgers 6|OWX/<who>·6|OWY/<who> += the required amounts
;;   (SAME closed form as the mint charge — S-independent, cell-edge
;;   chaining from G(a); helpers persist 6|RX/6|RY — receipts never
;;   parse returns), capped at the escrow float (insolvency guard).
;;   op "claim" {"t"} — pays one owed token per receipt from its escrow
;;   side; promise failure ⇒ p5-pay "cx"/"cy" restores owed + escrow
;;   exactly (C2a-proven single-leg reversal).
;; Parity ops "mint"/"burn" are UNTOUCHED (driver oracle).

;; required X to add d over [a,b]: sum of dx(d, G(i), G(i+1)) — the
;; X-consumption form (up-leg input, ÷ S·S2), rising so S2 > S holds.
;; Tail-recursive at fixed depth (≤ 65 = mint span cap).
;; ONE-SIDED GATE (2026-10-20): a cell strictly ABOVE the current cell
;; needs X only; cells below need Y only; the containing cell needs both.
;; Uniswap amounts-at-tick — the old code charged BOTH sides for every
;; range, so an x-only range demanded the quote side too (live bug on
;; pool4: range [11,25] @ P=9 returned ry ≈ rx).
(define (p5-reqx i b s d acc)
  (if (> i b) acc
      (p5-reqx (+ i 1) b (p5-G (+ i 1)) d
               (if (>= i (str->num (p5-bz "6|P")))
                   (li-add acc (p5-dx d s (p5-G (+ i 1))))
                   acc))))

;; required Y: sum of dy(d, G(i), G(i+1)) — the Y-consumption form
;; (down-leg input, >> 64), rising (dyi over a rising walk would walk
;; the pair DOWN into limb-sub underflow — the trap this fixed).
;; Mirror gate: cells ≤ the current cell contribute Y.
(define (p5-reqy i b s d acc)
  (if (> i b) acc
      (p5-reqy (+ i 1) b (p5-G (+ i 1)) d
               (if (<= i (str->num (p5-bz "6|P")))
                   (li-add acc (p5-dy d s (p5-G (+ i 1))))
                   acc))))

;; FULL-RANGE folds (no price gate) — the BURN side of the ledger. The
;; gated forms above answer "what would a FRESH position cost right
;; now"; an EXISTING position's honest owed is its full-range form
;; capped at the float:
;;   · no sweep ⇒ float == the gated mint charge ≤ full form ⇒ the cap
;;     pays back exactly what went in.
;;   · swept (price crossed the range) ⇒ the swept side's float left
;;     for the swapper, but the OTHER float holds the conversion
;;     proceeds (the swapper's intake) — the full-range form on that
;;     side reaches them, so the position exits whole. With the gated
;;     form here the proceeds strand in escrow (observed: 8.0e22 Y
;;     unclaimable after the lf_lifecycle sweep).
;; Conservation by construction: owed ≤ float, and the float only ever
;; holds mint charges + swap intakes − payouts − claims. Multi-LP: the
;; float is shared, so concurrent positions exit pro-rata-ish (first
;; claimer bounded by the shared float) — same class as any shared
;; escrow; the custody/launchpad pool has a single position.
(define (p5-rxa i b s d acc)
  (if (> i b) acc
      (p5-rxa (+ i 1) b (p5-G (+ i 1)) d
              (li-add acc (p5-dx d s (p5-G (+ i 1)))))))

(define (p5-rya i b s d acc)
  (if (> i b) acc
      (p5-rya (+ i 1) b (p5-G (+ i 1)) d
              (li-add acc (p5-dy d s (p5-G (+ i 1)))))))

;; accruef fold (Aggregate_Accrual verbatim): owed_fee =
;; Σ (FG_i − FS_i) × d >> 64 — per-cell floor ⇒ under-credit only.
;; Then FS_i ← FG_i: the residual position earns from current growth.
;; Known v1 under-credit (pool-favoring): a top-up mint resets the
;; whole key's snapshots, forfeiting pre-top-up accrual.
;;
;; NAME-SCOPE TRAP (cost a real bug, Oct 2026; reproduced + characterized
;; 2026-10-17, probe recipe: /tmp/trap_v{3,4}.lisp): there are exactly two
;; ways to "naturally" return a fold result here, and BOTH are compile
;; errors — (1) referencing the loop's binding after the loop:
;;   (loop ((i a) (acc "0")) ...) ... acc
;;   → "type error: undefined variable 'acc'" (loop bindings do not
;;   escape the loop); (2) hoisting with an internal define:
;;   (define acc (loop ...)) → "undefined variable 'define'" (defines
;;   are top-level only; a let-bound result in a trailing begin DOES
;;   compile fine — that shape is not the trap). Therefore the fold
;;   value can only escape AS the loop's own return value: the FS
;;   snapshot resets MUST ride inside the body (read-before-write), as
;;   below. The original bug: the loop's reset put ran, but its fold
;;   value was discarded, so FWX/FWY stayed empty and LP fee claims
;;   paid principal only.
(define (p5-feefold who a b d side)
  (loop ((i a) (s "0"))
    (if (> i b)
        s
        (let ((fg (p5-bz (str-cat "6|FG" (str-cat side
                     (str-cat ":" (to-string i))))))
              (fs (p5-bz (p5-fsk side who a b i))))
          (begin
            ;; FS_i ← FG_i NOW: the residual position earns from current
            ;; growth. Delta uses the PRE-reset fs (read-before-write).
            (p5-put (p5-fsk side who a b i) fg)
            (recur (+ i 1)
              (li-add s (p5-mulshr64 (li-sub fg fs) d))))))))

;; complete a matched two-leg mint: BOTH sides' d stays escrowed (the
;; deposited tokens back the position — burn claims pay from them);
;; write the position (exact op-mint core), consume exactly d, refund
;; any excess of this leg via the chain result.
(define (p5-mint-finish sender d a b amt)
  (begin
    (p5-put (str-cat "6|PM:" sender) "0")
    ;; position books — pointwise LQ += d, POS += d, widen support
    (loop ((i a))
      (if (> i b)
          0
          (let ((k (p5-lqkey i)))
            (begin
              (p5-put k (li-add (p5-bz k) d))
              (p5-put (p5-fskey sender a b i)
                      (p5-bz (str-cat "6|FGX:" (to-string i))))
              (p5-put (p5-fskeyy sender a b i)
                      (p5-bz (str-cat "6|FGY:" (to-string i))))
              (recur (+ i 1))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))
    (p5-put (str-cat "6|FSP:" (str-cat sender
              (str-cat ":" (str-cat (to-string a)
                (str-cat ":" (to-string b)))))) "1")
    (if (< a (str->num (p5-bz "6|IMIN")))
        (p5-put "6|IMIN" (to-string a))
        0)
    (if (> b (str->num (p5-bz "6|IMAX")))
        (p5-put "6|IMAX" (to-string b))
        0)
    (near/return amt)))

;; op "provision" {lo, hi, lock_secs} — bonding-curve graduation mint.
;; Caller MUST be 6|CURVE (set by the deployer; empty = op is dead, legacy
;; pools byte-identical). The curve seeded the escrows beforehand (float
;; X = unsold curve tokens via a:"seed", float Y = real QT reserve);
;; provision mints LQ d directly FROM that global escrow custody — the
;; same custody burnc pays out of — sized so req(d) fits per side:
;;   d = min(ESCX/kx, ESCY/ky), kx/ky = req(1e18 probe) via muldiv
;; (reqx/reqy are linear in d; floors make d an underestimate ⇒ escrow
;; remainder stays float for swap out-legs). Bookkeeping = mint-finish
;; verbatim (LQ += d per cell, FS snapshots from FG, POS += d, FSP = 1,
;; widen support). Lock: if 6|LOCK is 0/empty and lock_secs > 0, sets
;; burn-until = now + secs ONCE — a graduated launch's LP cannot be
;; pulled early even by whoever holds the curve's keys.
(define (p5-provision)
  (let ((inp (near/input)))
  (let ((who (near/predecessor_account_id)))
    (if (not (str= who (p5-bz "6|CURVE")))
        (p5-fail "ERR_CURVE")
        (let ((lo (str->num (json-get-str "lo" inp)))
              (hi (str->num (json-get-str "hi" inp)))
              (lk (str->num (json-get-str "lock_secs" inp))))
          (if (or (< hi lo) (> (- hi lo) 64))
              (p5-fail "ERR_RANGE")
              (let ((ga (p5-G lo)))
              (let ((gb (p5-G hi)))
              (let ((dp "1000000000000000000000000000000"))
              (let ((kx (u128-muldiv dp (li-sub gb ga) (u128-muldiv ga gb "1"))))
              (let ((ky (u128-muldiv dp (li-sub gb ga) "18446744073709551616")))
                (if (or (= (li-cmp kx "0") 0) (= (li-cmp ky "0") 0))
                    (p5-fail "ERR_SMALL")
                    (let ((d1 (u128-muldiv (p5-bz "6|ESCX") dp kx)))
                    (let ((d2 (u128-muldiv (p5-bz "6|ESCY") dp ky)))
                    (let ((d (if (< (li-cmp d1 d2) 0) d1 d2)))
                      (if (not (= (li-cmp d "0") 1))
                          (p5-fail "ERR_SMALL")
                          (begin
                            (loop ((i lo))
                              (if (> i hi)
                                  0
                                  (let ((k (p5-lqkey i)))
                                    (begin
                                      (p5-put k (li-add (p5-bz k) d))
                                      (p5-put (p5-fskey who lo hi i)
                                              (p5-bz (str-cat "6|FGX:" (to-string i))))
                                      (p5-put (p5-fskeyy who lo hi i)
                                              (p5-bz (str-cat "6|FGY:" (to-string i))))
                                      (recur (+ i 1))))))
                            (p5-put (p5-poskey who lo hi)
                                    (li-add (p5-bz (p5-poskey who lo hi)) d))
                            (p5-put (str-cat "6|FSP:" (str-cat who
                                      (str-cat ":" (str-cat (to-string lo)
                                        (str-cat ":" (to-string hi)))))) "1")
                            (if (< lo (str->num (p5-bz "6|IMIN")))
                                (p5-put "6|IMIN" (to-string lo))
                                0)
                            (if (> hi (str->num (p5-bz "6|IMAX")))
                                (p5-put "6|IMAX" (to-string hi))
                                0)
                            (if (and (> lk 0)
                                     (= (li-cmp (p5-bz "6|LOCK") "0") 0))
                                (p5-put "6|LOCK"
                                        (li-add (near/block_timestamp)
                                                (to-string lk)))
                                0)
                            (str-cat "provision-ok:" d))))))))))))))))))

;; ft_on_transfer mint/cancel leg (predecessor = a pool token, amount
;; validated). msg "c" cancels a pending mint: refund the recorded
;; slice via an escrow-backed ft_transfer_call (failure ⇒ p5-pay "mk"
;; re-credits the escrow).
(define (p5-mint-leg sender amt)
  (let ((act (json-get-str "a" (json-get-str "msg" (near/input)))))
    (if (str= act "cancel")
        (let ((rec (p5-bz (str-cat "6|PM:" sender))))
          (if (or (str= rec "") (str= rec "0"))
              (near/return amt)
              (let ((esc (p5-fld rec 4 58)))   ;; recorded escrow slice
              (let ((ek1 (str-cat "6|ESCM" (str-slice rec 0 1))))
                (begin
                  ;; cancel: refund the recorded slice via plain ft_transfer
                  ;; (a 0-amount ft_transfer_call moves nothing). State is
                  ;; touched ONLY in the callback: success clears PM + drains
                  ;; escrow; failure restores nothing — the pending mint and
                  ;; its escrow survive for a retry or leg 2 completion.
                  (let ((p (near/promise_batch_create (p5-bz "6|TOKX"))))
                    (begin
                      (near/promise_batch_action_function_call p
                        "ft_transfer"
                        (json-set (json-set "{}" "receiver_id" sender)
                                  "amount" esc)
                        "0" 40000000000000)
                      (near/promise_return
                        (near/promise_then p (near/current_account_id)
                          "pay"
                          (json-set (json-set (json-set (json-set
                            (json-set "{}" "k" "mk") "r" sender)
                            "d" esc) "w" ek1) "t" "6|TOKX")
                          "0" 5000000000000)))))))))
        (if (not (str= act "mint"))
            (near/return amt)
            (let ((d (json-get-str "d" (json-get-str "msg" (near/input)))))
            (let ((a (str->num (json-get-str "lo" (json-get-str "msg" (near/input))))))
            (let ((b (str->num (json-get-str "hi" (json-get-str "msg" (near/input))))))
              ;; NOTE: no pend guard here — a pending record IS the leg-2
              ;; trigger; same-side re-entry dies in ERR_MISMATCH below
              ;; (byte check), and completion clears the record.
              (if (or (< b a)
                      (> (- b a) 64)
                      (not (= (p5-num-ok? d) "1"))
                      (not (= (li-cmp d "0") 1)))
                  (p5-fail "ERR_RANGE")
              ;; attach validation is req-based per leg below (leg 1:
              ;; ERR_EXACT, leg 2: ERR_SHORT against its own side's req)
              (let ((toky? (str= (near/predecessor_account_id)
                                 (p5-bz "6|TOKY"))))
              (let ((rec (p5-bz (str-cat "6|PM:" sender))))
                ;; "0" = never written, "" = cancel-cleared: BOTH mean
                ;; first leg (bz never returns "" for a missing key)
                (if (or (str= rec "") (str= rec "0"))
                    ;; FIRST leg: record + escrow the REQUIRED amount on
                    ;; this side. C3: attach ≥ req — excess rides the same
                    ;; resolve refund leg 2 uses (the old "no refund path"
                    ;; note was wrong: ft2 resolve reads THIS call's
                    ;; return). Over-attach is how clients ride out the
                    ;; req-view drift (see the C3 note below).
                    (let ((ga (p5-G a)))
                    (let ((req (if toky?
                                   (p5-reqy a b ga d "0")
                                   (p5-reqx a b ga d "0"))))
                    ;; ONE-SIDED MINT: when the OTHER side's req = 0 this leg
                    ;; IS the whole mint — escrow + finish now. The old path
                    ;; always waited for a second leg, so a zero-skipped
                    ;; x-only/y-only deposit recorded itself and never
                    ;; finished (frontend surfaces it as a rejected leg).
                    (let ((other (if toky?
                                     (p5-reqx a b ga d "0")
                                     (p5-reqy a b ga d "0"))))
                      (if (< (li-cmp amt req) 0)
                          (p5-fail "ERR_SHORT")
                          ;; Auto-finish ONLY on an explicit one-sided flag:
                          ;; the factory's split seed legs are ALSO one-sided
                          ;; by the req gate (range split around price) and
                          ;; must keep the two-leg protocol — inferring from
                          ;; other=0 hijacked its first leg (lf-lifecycle
                          ;; regression: escrow booked, legs mispaired).
                          (if (and (str= other "0")
                                   (str= "1" (json-get-str "one" (json-get-str "msg" (near/input)))))
                              (let ((ek (if toky? "6|ESCY" "6|ESCX")))
                                (begin
                                  (p5-put ek (li-add (p5-bz ek) req))
                                  (p5-mint-finish sender d a b (li-sub amt req))))
                              (let ((ek (if toky? "6|ESCMY" "6|ESCMX")))
                                (begin
                                  (p5-put (str-cat "6|PM:" sender)
                                    (str-cat (if toky? "Y" "X")
                                      (str-cat ":" (str-cat (to-string a)
                                        (str-cat ":" (str-cat (to-string b)
                                          (str-cat ":" (str-cat d (str-cat ":" req)))))))))
                                  ;; pending-mint custody is QUARANTINED in
                                  ;; ESCM: the shared float (ESC) must never
                                  ;; hold it, so burns, claims and swaps can
                                  ;; never pay out of a pending mint's money
                                  (p5-put ek (li-add (p5-bz ek) req))
                                  (near/return (li-sub amt req)))))))))
                    ;; SECOND leg: opposite side, same a/b (d must match
                    ;; too — field 3 — else the two legs split escrow)
                    (if (not (and (= (byte-at rec 0)
                                     (if toky? 88 89))
                                  (= (str->num (p5-fld rec 1 58)) a)
                                  (= (str->num (p5-fld rec 2 58)) b)
                                  (str= (p5-fld rec 3 58) d)))
                        (p5-fail "ERR_MISMATCH")
                        (let ((ga (p5-G a)))
                        (let ((req (if toky?
                                       (p5-reqy a b ga d "0")
                                       (p5-reqx a b ga d "0"))))
                          (if (< (li-cmp amt req) 0)
                              (p5-fail "ERR_SHORT")
                              (begin
                                ;; leg 2's required slice joins the escrow
                                (p5-put (if toky? "6|ESCY" "6|ESCX")
                                  (li-add (p5-bz (if toky? "6|ESCY" "6|ESCX"))
                                          req))
                                ;; release leg 1's quarantined custody: its
                                ;; slice becomes this position's backing in
                                ;; the shared float (byte 88 = X, 89 = Y)
                                (let ((m1 (if (= (byte-at rec 0) 88)
                                              "6|ESCMX" "6|ESCMY")))
                                (let ((e1 (if (= (byte-at rec 0) 88)
                                              "6|ESCX" "6|ESCY")))
                                  (begin
                                    (p5-put m1 (li-sub (p5-bz m1)
                                                       (p5-fld rec 4 58)))
                                    (p5-put e1 (li-add (p5-bz e1)
                                                       (p5-fld rec 4 58)))))
                                (p5-mint-finish sender d a b
                                  (li-sub amt req)))))))))))))))))))

;; op "burnc" {d,a,b} — custody burn: authority + exact op-burn core,
;; then owed ledgers from the required-amount closed forms. 6|RX/6|RY
;; carry the raw sums for callers/tests.
(define (p5-burnc)
  (let* ((inp (near/input))
         (who (near/predecessor_account_id))
         (d (json-get-str "d" inp))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
    ;; LP lock: no burn, not even partial, before the unlock time. Checked
    ;; FIRST so the answer never depends on range/liquidity validity.
    (if (and (> (li-cmp (p5-bz "6|LOCK") "0") 0)
             (< (li-cmp (near/block_timestamp) (p5-bz "6|LOCK")) 0))
        (p5-fail "ERR_LOCKED")
    (if (or (< b a)
            (> (- b a) 64)
            (not (= (p5-num-ok? d) "1"))
            (not (= (li-cmp d "0") 1)))
        (p5-fail "ERR_RANGE")
    ;; authority: caller owns ≥ d at [a,b] (POS keys = custody truth)
    (if (< (li-cmp (p5-bz (p5-poskey who a b)) d) 0)
        (p5-fail "ERR_AUTH")
    ;; per-cell guard ∀i∈[a,b]. LQ i ≥ d  (same relation as op burn)
    (if (= (loop ((i a))
                 (if (> i b) 1
                     (if (< (li-cmp (p5-bz (p5-lqkey i)) d) 0)
                         0
                         (recur (+ i 1)))))
           0)
        (p5-fail "ERR_LIQ")
    (let ((ga (p5-G a)))
    (let ((rx (p5-rxa a b ga d "0")))
    (let ((ry (p5-rya a b ga d "0")))
      (begin
        (p5-put "6|RX" rx)
        (p5-put "6|RY" ry)
        ;; pointwise subtract — op-burn core
        (loop ((i a))
          (if (> i b) 0
              (let ((k (p5-lqkey i)))
                (begin
                  (p5-put k (li-sub (p5-bz k) d))
                  (recur (+ i 1))))))
        (p5-respan!)
        (p5-put (p5-poskey who a b)
                (li-sub (p5-bz (p5-poskey who a b)) d))
        ;; fee fold (accruef) — LP trading fees, capped at the float;
        ;; raw folds persist in 6|FWX/6|FWY for callers/tests.
        (let ((fx (p5-feefold who a b d "X")))
        (let ((fy (p5-feefold who a b d "Y")))
        (begin
          ;; owed = req + fees, CAPPED at the per-side escrow float.
          ;; The req forms are provide-forms at the CURRENT price: after
          ;; a sweep the swept side's backing has genuinely left the
          ;; pool (sold to the swapper), so the form can exceed the
          ;; float. The old fail-hard (ERR_FLOAT, 2026-10-18) stranded
          ;; the position — the burn trapped and NOTHING could move;
          ;; the one-sided gate (4691a29f) removed the 2x provision
          ;; slack that masked this. No book term in the backing: the
          ;; 6|X/6|Y book entries are out-legs already debited from the
          ;; escrow at initiation, so counting them double-counts (the
          ;; mock settles receipts in-DAG; an lp claim then "paid" from
          ;; money that was gone and zeroed the owed). The return
          ;; carries the PAID amounts — the caller forwards exactly
          ;; those.
          (p5-put "6|FWX" fx)
          (p5-put "6|FWY" fy)
        (let ((cx (p5-req-cap (li-add rx fx) (p5-float-net-x))))
        (let ((cy (p5-req-cap (li-add ry fy) (p5-float-net-y))))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (p5-put "6|OWTX" (li-add (p5-bz "6|OWTX") cx))
            (p5-put "6|OWTY" (li-add (p5-bz "6|OWTY") cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy))))))))))))))))))))


;; op "harvest" {a,b} — fold pending fees to owed WITHOUT burning. Same
;; accruef fold as burnc over the FULL position d (fees = Σ (FG−FS)×d >>
;; 64, FS←FG read-before-write), credited capped at the per-side free
;; float; position principal untouched: no LQ/POS writes, no req term,
;; no lock gate (nothing moves but fee ownership). If the float is short,
;; the un-credited remainder is forfeited by the FS reset (same
;; pool-favoring semantics as burnc's cap — insolvency is pro-rata by
;; claim order).
(define (p5-harvest)
  (let* ((who (near/predecessor_account_id))
         (inp (near/input))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
    (if (or (< b a) (> (- b a) 64))
        (p5-fail "ERR_RANGE")
    (let ((d (p5-bz (p5-poskey who a b))))
    (if (not (= (li-cmp d "0") 1))
        (p5-fail "ERR_AUTH")
    (let ((fx (p5-feefold who a b d "X")))
    (let ((fy (p5-feefold who a b d "Y")))
      (begin
        (let ((cx (p5-req-cap fx (p5-float-net-x))))
        (let ((cy (p5-req-cap fy (p5-float-net-y))))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (p5-put "6|OWTX" (li-add (p5-bz "6|OWTX") cx))
            (p5-put "6|OWTY" (li-add (p5-bz "6|OWTY") cy))
            (str-cat "harvest-ok:" (str-cat cx (str-cat ":" cy)))))))))))))))

;; required-amount custody: owed = min(req, free float) — insolvency
;; guard only. req is token-denominated (same closed form at mint and
;; burn), so no unit scaling. Free float = escrow minus ALREADY-RECORDED
;; owed (6|OWTX/6|OWTY are senior claims: first burn to record is first
;; paid, so Σ owed ≤ escrow is an invariant and every claim is fully
;; backed). Pending-mint custody is quarantined in 6|ESCMX/6|ESCMY
;; (never part of the shared float), so burns, claims and swap walkers
;; can never pay out of a pending mint's money.
(define (p5-float-net-x)
  (let ((e (p5-bz "6|ESCX")))
  (let ((r (p5-bz "6|OWTX")))
    (if (< (li-cmp r e) 1) (li-sub e r) "0"))))
(define (p5-float-net-y)
  (let ((e (p5-bz "6|ESCY")))
  (let ((r (p5-bz "6|OWTY")))
    (if (< (li-cmp r e) 1) (li-sub e r) "0"))))

(define (p5-req-cap req net)
  (if (< (li-cmp req net) 1) req net))

;; op "req" {d,a,b} — the required-amount closed form (S-independent:
;; pure cell-edge chaining from G(a)); mint legs attach these exactly.
(define (p5-req)
  (let ((inp (near/input)))
  (let ((d (json-get-str "d" inp)))
  (let ((a (str->num (json-get-str "a" inp))))
  (let ((b (str->num (json-get-str "b" inp))))
    (let ((ga (p5-G a)))
      (str-cat "rx:" (str-cat (p5-reqx a b ga d "0")
                 (str-cat ":ry:" (p5-reqy a b ga d "0"))))))))))

;; op "reg" { } — self-registration gate for claim payouts. NEP-141
;; transfers to an unregistered account FAIL on-chain; a failed claim
;; payout costs a full reversal round-trip and (before the pay signal
;; fix) surfaced as a silent "0". Gate before netting: not registered ⇒
;; abort "not-registered" with owed+escrow untouched. Self-only (the
;; caller pays their own storage key — no griefer-funded registry).
;; Swap/mint payouts need no gate: the sender just did ft_transfer_call,
;; which NEP-141 only accepts from registered accounts.
(define (p5-register)
  (let ((who (json-get-str "who" (near/input))))
    (if (< (str-len who) 1)
        ;; predecessor is ProhibitedInView — view-mode callers pass who
        ;; explicitly; tx callers may too (fallback to predecessor)
        (let ((w2 (near/predecessor_account_id)))
          (p5-reg! w2))
        (p5-reg! who))))

(define (p5-reg! who)
  (let ((k (str-cat "6|REG:" who)))
    (if (nil? (near/load-bytes k))
        (begin (p5-put k "1") (str-cat "reg-ok:" who))
        (str-cat "reg-ok:" who))))

;; op "claim" {"t"} — pay one owed token per receipt. Failure ⇒ p5-pay
;; "cx"/"cy" restores owed + escrow (single-leg reversal, C2a pattern).
;; Token is case-normalized ("X"/"Y" accepted — a live 2026-10-17 tx sent
;; "Y" and the old strict (str= tok "y") silently routed it to the X
;; branch) and any other value is REJECTED: the old fall-through paid the
;; X branch for arbitrary garbage input.
(define (p5-claim-norm tok)
  (if (str= tok "X") "x"
      (if (str= tok "Y") "y" tok)))

(define (p5-claim-one tok who)
  (let ((w (if (str= tok "y")
               (str-cat "6|OWY:" who)
               (str-cat "6|OWX:" who))))
  (let ((e (if (str= tok "y") "6|ESCY" "6|ESCX")))
  (let ((tkey (if (str= tok "y") "6|TOKY" "6|TOKX")))
  (let ((k (if (str= tok "y") "cy" "cx"))
        (ib (if (str= tok "y") "6|Y" "6|X"))
        (v (p5-bz w)))
    (if (nil? (near/load-bytes (str-cat "6|REG:" who)))
        "not-registered"
    (if (= (li-cmp v "0") 0)
        "no-owed"
    ;; payout stock check: pay only what the float actually backs. The
    ;; 6|X/6|Y books are FLOW LEDGERS (walker intake/PnL bookings), not
    ;; token stock — funding a payout from them (the 2026-10-18
    ;; reclassification) let claims pay more than the pool held (party
    ;; sim: balance fell below the escrow books). Shortfall is a PARTIAL
    ;; payout: the unpaid remainder stays owed for retry after refill; a
    ;; failed payout restores owed + esc symmetrically (pay cx/cy).
    (let ((v2 (if (< (li-cmp (p5-bz e) v) 1) (p5-bz e) v)))
    (if (= (li-cmp v2 "0") 0)
        "underfunded"
        (begin
          ;; leave the unpaid remainder owed on the owner's ledger
          (p5-put w (li-sub v v2))
          (let ((v v2))
          (p5-put e (li-sub (p5-bz e) v))
          (let ((p (near/promise_batch_create (p5-bz tkey))))
            (begin
              (near/promise_batch_action_function_call p
                "ft_transfer"
                (json-set (json-set "{}" "receiver_id" who)
                          "amount" v)
                "0" 5000000000000)
              (near/promise_return
                (near/promise_then p (near/current_account_id)
                  "pay"
                  (json-set (json-set (json-set (json-set
                    (json-set "{}" "k" k) "r" who) "o" v)
                    "w" w) "e" e)
                  "0" 5000000000000))
              "claim-sent")))))))))))))

(define (p5-claim)
  (let ((who (near/predecessor_account_id)))
    (let ((tok (p5-claim-norm (json-get-str "t" (near/input)))))
      (if (or (str= tok "x") (str= tok "y"))
          (p5-claim-one tok who)
          "bad-token"))))

;; ── views + dispatch ───────────────────────────────────────────────────

(define (p5-get)
  (let ((inp (near/input)))
    (let ((k (json-get-str "k" inp)))
      (p5-bz (str-cat "6|" k)))))

(define (p5-lq-chunk)
  (let ((lo (p5-imin)))
  (let ((hi (p5-imax)))
    (loop ((i lo) (acc ""))
      (if (> i hi) acc
          (let ((v (p5-bz (p5-lqkey i))))
            (recur (+ i 1)
              (if (= (li-cmp v "0") 0) acc
                  (str-cat acc
                    (str-cat ":LQ:"
                      (str-cat (to-string i) (str-cat "|" v))))))))))))

(define (p5-view)
  (let ((s (p5-bz "6|S")))
  (let ((pp (p5-bz "6|P")))
  (let ((x (p5-bz "6|X")))
  (let ((y (p5-bz "6|Y")))
  (let ((imn (p5-bz "6|IMIN")))
  (let ((imx (p5-bz "6|IMAX")))
  (let ((t1 (str-cat "S:" s)))
  (let ((t2 (str-cat t1 (str-cat ":P:" pp))))
  (let ((t3 (str-cat t2 (str-cat ":X:" x))))
  (let ((t4 (str-cat t3 (str-cat ":Y:" y))))
  (let ((t5 (str-cat t4 (str-cat ":IMIN:" imn))))
  (let ((t6 (str-cat t5 (str-cat ":IMAX:" imx))))
    (str-cat t6 (p5-lq-chunk)))))))))))))))

;; read-only custody view: the escrow ledger pair in one call — the exact
;; keys external auditors reconcile against raw ft_balance_of. ZERO writes.
(define (p5-escrow)
  (str-cat "ESCX:" (str-cat (p5-bz "6|ESCX")
             (str-cat ":ESCY:" (p5-bz "6|ESCY")))))

;; read-only full state: reserves, curve bounds, last action, custody pair,
;; and the token contracts in charge. ZERO writes.
(define (p5-state)
  (let ((b (p5-view)))
    (str-cat b
      (str-cat ":LAST:" (p5-bz "6|LAST"))
      (str-cat ":ESCX:" (p5-bz "6|ESCX"))
      (str-cat ":ESCY:" (p5-bz "6|ESCY"))
      (str-cat ":TOKX:" (p5-bz "6|TOKX"))
      (str-cat ":TOKY:" (p5-bz "6|TOKY"))
      (str-cat ":SHIFT:" (if (= (str->num (p5-bz "6|SHIFT")) 0)
                             "8"
                             (p5-bz "6|SHIFT")))
      (str-cat ":FEEX:" (p5-bz "6|FEEX"))
      (str-cat ":FEEY:" (p5-bz "6|FEEY")))))

;; read-only fee view: one cell's growth pair. ZERO writes.
(define (p5-fg)
  (let ((i (json-get-str "i" (near/input))))
    (str-cat "fgx:" (str-cat (p5-bz (str-cat "6|FGX:" i))
               (str-cat ":fgy:" (p5-bz (str-cat "6|FGY:" i)))))))

;; read-only: one owner's owed balances in one call. ZERO writes.
;; owed:<x>:<y> (0 when no receipt).
(define (p5-owed)
  (let ((who (json-get-str "who" (near/input))))
    (str-cat "owed:" (str-cat (p5-bz (str-cat "6|OWX:" who))
               (str-cat ":" (p5-bz (str-cat "6|OWY:" who)))))))

;; read-only: all of an owner's LP positions, format
;; pos:<a>:<b>:<d>;... (semicolon-joined). No index — the scan reads the
;; poskeys themselves (single source of truth; the flag-key variant was
;; dropped: same silent-orphan class as the feefold trap). a over
;; [IMIN..IMAX], b over [a..min(a+64, IMAX)] — IMIN/IMAX are the mint
;; extremes so no poskey can lie outside. Membership = str= d "0"
;; (poskey values are canonical li-add output; NEVER li-cmp per cell —
;; 129*65 bignum parses blew the mock gas cap, 2026-10-17). Empty
;; string => no positions. ZERO writes.
(define
  (p5-positions)
  (let
    ((who (json-get-str "who" (near/input))))
    (let
      ((lo (str->num (p5-bz "6|IMIN"))) (hi (str->num (p5-bz "6|IMAX"))))
      (loop
        ((a lo) (acc ""))
        (if
          (> a hi)
          acc
          (let
            ((inner
                (loop
                  ((b a) (s acc))
                  (if
                    (or (> b hi) (> b (+ a 64)))
                    s
                    (let
                      ((d (p5-bz (p5-poskey who a b))))
                      (recur
                        (+ b 1)
                        (if
                          (str= d "0")
                          s
                          (str-cat
                            s
                            (str-cat
                              (if (= (str-len s) 0) "" ";")
                              (str-cat
                                "pos:"
                                (str-cat
                                  (to-string a)
                                  (str-cat
                                    ":"
                                    (str-cat (to-string b) (str-cat ":" d)))))))))))))
)
            (recur (+ a 1) inner)))))))

;; oracle op: pure quote — same closed form, ZERO state writes (the
;; driver's differential oracle target; also swap_rel's s'-projection).
(define (p5-qarm-dn s p v)
  (let ((imin (p5-imin)))
  (let ((gmin (p5-G imin)))
  (let ((r (loop ((q p) (ss s) (acc "0") (rm v))
               (if (or (= (li-cmp rm "0") 0)
                       (< (li-cmp ss gmin) 1)
                       (<= q imin))
                   (str-cat "q:" (str-cat acc (str-cat ":rm:" rm)))
               (let ((L (p5-bz (p5-lqkey q))))
               (let ((gl (p5-G (- q 1))))
               (let ((dyi (p5-dyi L ss gl)))
                 (if (< (li-cmp dyi rm) 1)
                     (recur (- q 1) gl (li-add acc (if (= (li-cmp dyi "0") 0) "0" (p5-dxo L ss gl)))
                            (li-sub rm dyi))
                     (let ((s2 (p5-sfdyi L ss rm)))
                       (str-cat "q:" (str-cat (li-add acc (p5-dxo L ss s2))
                                (str-cat ":rm:" "0"))))))))))))
    r))))

(define (p5-qarm-up s p v)
  (let ((imax (p5-imax)))
  (let ((gmax (p5-G (+ imax 1))))
  (let ((r (loop ((q p) (ss s) (acc "0") (rm v))
               (if (or (= (li-cmp rm "0") 0)
                       (>= (li-cmp ss gmax) 0)
                       (>= q (+ imax 1)))
                   (str-cat "q:" (str-cat acc (str-cat ":rm:" rm)))
               (let ((L (p5-bz (p5-lqkey q))))
               (let ((gh (p5-G (+ q 1))))
               (let ((dx (p5-dx L ss gh)))
                 (if (< (li-cmp dx rm) 1)
                     (recur (+ q 1) gh (li-add acc (if (= (li-cmp dx "0") 0) "0" (p5-dy L ss gh)))
                            (li-sub rm dx))
                     (let ((s2 (p5-sfdx L ss rm)))
                       (str-cat "q:" (str-cat (li-add acc (p5-dy L ss s2))
                                (str-cat ":rm:" "0"))))))))))))
    r))))
(define (p5-quote)
  (let ((inp (near/input)))
  (let ((v (json-get-str "v" inp)))
  (let ((neg (if (str= (str-slice v 0 1) "-") 1 0)))
  (let ((v (if (= neg 1) (str-slice v 1 (str-len v)) v)))
  (let ((s (p5-bz "6|S")))
  (let ((p (str->num (p5-bz "6|P"))))
    (if (= (li-cmp v "0") 0)
        "q:0:0"
    (if (= neg 1)
        (p5-qarm-dn s p v)
        (p5-qarm-up s p v))))))))))

;; harness door: gross ops + raw-key reads, SIMSWAP-gated. Prod answers
;; "sim-only" -- public ABI = verb exports + ft_on_transfer msgs.
(define (grid) (p5-G (str->num (json-get-str "i" (near/input)))))
;; ── get_lock — burn-until (ns), "0" = unlocked ──────────────────────────
(define (p5-get-lock)
  (near/return (p5-bz "6|LOCK")))

(define (p5-exec)
  (let ((op (json-get-str "op" (near/input))))
    (if (str= (p5-bz "6|SIMSWAP") "1")
    (if (str= op "init") (p5-init)
    (if (str= op "mint") (p5-mint)
    (if (str= op "burn") (p5-burn)
    (if (str= op "burnc") (p5-burnc)
    (if (str= op "harvest") (p5-harvest)
    (if (str= op "claim") (p5-claim)
    (if (str= op "req") (p5-req)
    (if (str= op "swap") (p5-swap)
    (if (str= op "quote") (p5-quote)
    (if (str= op "get") (p5-get)
    (if (str= op "view") (p5-view)
    (if (str= op "escrow") (p5-escrow)
    (if (str= op "state") (p5-state)
    (if (str= op "fg") (p5-fg)
    (if (str= op "reg") (p5-register)
    (if (str= op "owed") (p5-owed)
    (if (str= op "positions") (p5-positions)
    (if (str= op "g") (p5-G (str->num (json-get-str "i" (near/input))))
        "unknown-op"))))))))))))))))))
    "sim-only")))
;; public ABI: verb exports + chain-imposed doors (ft_on_transfer
;; carries ALL inbound money; pay resolves payout promises).
(export "exec" p5-exec)
(export "init" p5-init)
(export "burn" p5-burnc)
(export "collect" p5-claim)
(export "harvest" p5-harvest)
(export "register" p5-register)
(export "position_requirements" p5-req)
(export "quote" p5-quote)
(export "get_state" p5-state)
(export "get_positions" p5-positions)
(export "get_owed" p5-owed)
(export "get_escrow" p5-escrow)
(export "get_fg" p5-fg)
(export "grid" grid)
(export "get_lock" p5-get-lock)
(export "provision" p5-provision)
(export "ft_on_transfer" p5-on-transfer)
(export "pay" p5-pay)
