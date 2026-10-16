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
;; C1 SCOPE (this commit): _run-only. Books 6|X/6|Y move ONLY in swaps
;; (consumed in / quoted out, exact). Token custody legs (ft_on_transfer,
;; payout promises, fault reversal) are arc C2 on the proven v4/NEP-141
;; pattern. POS:<who>:<a>:<b> = per-owner cumulative mint (burn authority;
;; the User_Ledger-flavored book). Driver oracle = thy formulas directly.
;;
;; Call discipline: one value-producing call per let binding (compiler
;; sibling-call clobber — accrual contract note). `begin` wraps only
;; effect-only sequences. Kernel adapters are string-in/string-out.
;; ═══════════════════════════════════════════════════════════════════════

(define (p5-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (p5-put k v) (near/store-bytes k v))
(define (p5-fail m) (begin (near/log m) "ERR"))
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

;; S_for_dx: smallest S2 ≥ S with dx_in(L,S,S2) ≤ dx.
;; Closed form (kernel-proven): L=0 → 2^127 ; else
;;   D = L + (dx+1)·S ; S2 = ceil((L·S+1)/D) ; min with 2^127, max with S.
(define (p5-sfdx L S dx)
  (let ((C127 (p5-str6 "340282366920938463463374607431768211456"))
        (Lb (p5-str6 L)) (Sb (p5-str6 S)) (dxb (p5-str6 dx))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (if (<= (v5-strip Lb 12) 0)
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
            ;; tB += L
            (v5-zero! tA 12)
            (v5-copy! Lb tA 6)
            (limb-add tB tA tB 12 12)
            ;; tC = L·S + 1
            (v5-zero! tC 20)
            (limb-mul Lb Sb tC 6 6)
            (limb-set! one 0 1)
            (limb-add tC one tC 12 1)
            ;; q = ceil(tC / tB)
            (v5-zero! q 10) (v5-zero! r 10)
            (v5-divmod! tC tB q r nbs s one)
            (if (> (v5-strip r 10) 0)
                (begin (limb-set! one 0 1) (limb-add q one q 10 1))
                0)
            ;; tE = min(2^127, max(S, q))
            (v5-zero! tE 12)
            (v5-copy! q tE 10)
            (if (> (limb-cmp tE C127 10 10) 0)
                (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))
                0)
            (if (< (limb-cmp tE Sb 10 6) 0)
                (begin (v5-zero! tE 12) (v5-copy! Sb tE 6))
                0))))
      (p5-buf-str tE)))

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

;; ── support bounds: exact, bounded probes ──────────────────────────────
;; IMIN/IMAX cached; burn of an edge cell walks inward (≤ 4096 probes).

(define (p5-lq-nz? i) (> (str->num (p5-bz (p5-lqkey i))) 0))

(define (p5-imin) (str->num (p5-bz "6|IMIN")))

(define (p5-imax) (str->num (p5-bz "6|IMAX")))

;; ── ops ────────────────────────────────────────────────────────────────

(define (p5-init)
  (if (nil? (near/load-bytes "6|TOKX"))
      (let ((tokx (json-get-str "toka" (near/input)))
            (toky (json-get-str "tokb" (near/input))))
        (begin
          (p5-put "6|TOKX" tokx)
          (p5-put "6|TOKY" toky)
          ;; s0 = 2^64 (price 1.0), P = 0, empty book
          (p5-put "6|S" "18446744073709551616")
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
        (p5-put (p5-poskey who a b)
                (li-sub (p5-bz (p5-poskey who a b)) d))
        ;; support: shrink edges exactly — the thy span is IMIN/IMAX
        ;; themselves; burn only shrinks to cells still carrying LQ, and
        ;; a full-empty burn leaves the bounds at the burned span (oracle
        ;; treats all-zero span as empty either way).
        (if (p5-lq-nz? (str->num (p5-bz "6|IMIN"))) 0 0)
        (str-cat "burn-ok:" d))))))))

;; donate_rel v i — lq i += v, nothing else
(define (p5-donate)
  (let* ((inp (near/input))
         (v (json-get-str "v" inp))
         (i (str->num (json-get-str "i" inp)))
         (k (p5-lqkey i)))
    (begin
      (p5-put k (li-add (p5-bz k) v))
      (if (< i (str->num (p5-bz "6|IMIN")))
          (p5-put "6|IMIN" (to-string i))
          0)
      (if (> i (str->num (p5-bz "6|IMAX")))
          (p5-put "6|IMAX" (to-string i))
          0)
      (str-cat "donate-ok:" v))))

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
;; Emitter discipline (ft2 arc): big multi-binding let* blocks in one
;; function collide local slots — every function below uses sequential
;; single-binding lets only, and loops return ONE packed string.
;;
;; walk arm for neg=1 (down): returns "acc|rm" packed, puts P/S.
(define (p5-swarm-dn s p xbook ybook v)
  (let ((imin (p5-imin)))
  (let ((gmin (p5-G imin)))
  (let ((res (loop ((pp p) (ss s) (acc "0") (rm v) (v0 v))
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
                 (let ((L (p5-bz (p5-lqkey pp))))
                 (let ((gl (p5-G (- pp 1))))
                 (let ((dxo (p5-dxo L ss gl)))
                 (let ((dyi (p5-dyi L ss gl)))
                   (if (< (li-cmp dyi rm) 1)
                       (recur (- pp 1) gl (li-add acc (if (= (li-cmp dyi "0") 0) "0" dxo))
                              (li-sub rm dyi)
                              v0)
                       (let ((s2 (p5-sfdyi L ss rm)))
                         (begin
                           (p5-put "6|P" (to-string pp))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dxo L ss s2))
                                    (str-cat "|" "0"))))))))))))))
  (let ((out (p5-fst res)))
  (let ((rmf (p5-snd res)))
  (let ((xnew (li-add xbook out)))
  (let ((yin (li-sub v rmf))
        (msg (str-cat "swap-out:" (str-cat out (str-cat ":rem:" rmf)))))
    (if (= (li-cmp rmf v) 0)
        msg
        (begin
          (p5-put "6|X" xnew)
          (p5-put "6|Y" (li-add ybook yin))
          (p5-put "6|LAST" msg)
          msg))))))))))

;; walk arm for neg=0 (up): returns "acc|rm" packed, puts P/S.
(define (p5-swarm-up s p xbook ybook v)
  (let ((imax (p5-imax)))
  (let ((gmax (p5-G (+ imax 1))))
  (let ((res (loop ((pp p) (ss s) (acc "0") (rm v) (v0 v))
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
                 (let ((L (p5-bz (p5-lqkey pp))))
                 (let ((gh (p5-G (+ pp 1))))
                 (let ((dy (p5-dy L ss gh)))
                 (let ((dx (p5-dx L ss gh)))
                   (if (< (li-cmp dx rm) 1)
                       (recur (+ pp 1) gh (li-add acc (if (= (li-cmp dx "0") 0) "0" dy))
                              (li-sub rm dx)
                              v0)
                       (let ((s2 (p5-sfdx L ss rm)))
                         (begin
                           (p5-put "6|P" (to-string pp))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dy L ss s2))
                                    (str-cat "|" "0"))))))))))))))
  (let ((out (p5-fst res)))
  (let ((rmf (p5-snd res)))
  (let ((ynew (li-add ybook out)))
  (let ((xin (li-sub v rmf))
        (msg (str-cat "swap-out:" (str-cat out (str-cat ":rem:" rmf)))))
    (if (= (li-cmp rmf v) 0)
        msg
        (begin
          (p5-put "6|Y" ynew)
          (p5-put "6|X" (li-add xbook xin))
          (p5-put "6|LAST" msg)
          msg))))))))))

(define (p5-swap)
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
        (p5-swarm-dn s p xbook ybook v)
        (p5-swarm-up s p xbook ybook v)))))))))))))
;; ── views + dispatch ───────────────────────────────────────────────────

(define (p5-get)
  (let ((inp (near/input)))
    (let ((k (json-get-str "k" inp)))
      (p5-bz (str-cat "6|" k)))))

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
    (str-cat t5 (str-cat ":IMAX:" imx))))))))))))))

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

(define (p5-run)
  (let ((op (json-get-str "op" (near/input))))
    (if (str= op "init") (p5-init)
    (if (str= op "mint") (p5-mint)
    (if (str= op "burn") (p5-burn)
    (if (str= op "donate") (p5-donate)
    (if (str= op "swap") (p5-swap)
    (if (str= op "quote") (p5-quote)
    (if (str= op "get") (p5-get)
    (if (str= op "view") (p5-view)
    (if (str= op "g") (p5-G (str->num (json-get-str "i" (near/input))))
        "unknown-op")))))))))))

(export "_run" p5-run)