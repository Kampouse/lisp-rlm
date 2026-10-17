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
;; C1 SCOPE: _run-only. Books 6|X/6|Y move ONLY in swaps
;; (consumed in / quoted out, exact).
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
;; escrows req; leg 2 attaches ≥ req, matches side+a+b+d, escrows its
;; req, completes (exact op-mint core) and refunds excess via used.
;; msg "c" cancels: plain ft_transfer of the RECORDED slice, PM cleared +
;; escrow drained only in the p5-pay "mk" success branch (failure leaves
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

(define (p5-lq-nz? i) (not (= (li-cmp (p5-bz (p5-lqkey i)) "0") 0)))

(define (p5-imin) (str->num (p5-bz "6|IMIN")))

(define (p5-imax) (str->num (p5-bz "6|IMAX")))

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
         (d (json-get-str "d" inp))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
    ;; thy burn_rel: guards = range, per-cell 0 ≤ lq' i, nonzero support,
    ;; span bounds. No authority predicate exists in the relation — the
    ;; pos-key check the v4 skeleton carried was beyond the proof and is
    ;; dropped (spec-parity; positions remain mint-side bookkeeping).
    (if (or (< b a) (> (- b a) 64))
        (p5-fail "ERR_RANGE")
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
        ;; support: shrink edges exactly — the thy span is IMIN/IMAX
        ;; themselves; walk inward to the first/last nonzero cell, 0/0
        ;; when the span emptied (oracle: support None ≡ bounds 0/0).
        (p5-respan!)
        (str-cat "burn-ok:" d)))))))

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
(define (p5-swarm-dn s p xbook ybook v)
  (let* ((imin (p5-imin))
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
                   (if (< (li-cmp dyi rm) 1)
                       (recur (- pp 1) gl (li-add acc (if (= (li-cmp dyi "0") 0) "0" dxo))
                              (li-sub rm dyi)
                              v0)
                       (let ((s2 (p5-sfdyi L ss rm)))
                         (begin
                           ;; s2 lands strictly inside the cell BELOW pp
                           ;; ([G(gl), G(pp))) — thy P' = lower_tick(s').
                           (p5-put "6|P" (to-string (- pp 1)))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dxo L ss s2))
                                    (str-cat "|" "0")))))))))))
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
          msg)))))

;; walk arm for neg=0 (up): returns "acc|rm" packed, puts P/S.
(define (p5-swarm-up s p xbook ybook v)
  (let* ((imax (p5-imax))
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
                   (if (< (li-cmp dx rm) 1)
                       (recur (+ pp 1) gh (li-add acc (if (= (li-cmp dx "0") 0) "0" dy))
                              (li-sub rm dx)
                              v0)
                       (let ((s2 (p5-sfdx L ss rm)))
                         (begin
                           (p5-put "6|P" (to-string pp))
                           (p5-put "6|S" s2)
                           (str-cat (li-add acc (p5-dy L ss s2))
                                    (str-cat "|" "0")))))))))))
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
          msg)))))

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

(define (p5-on-transfer)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (str= (p5-num-ok? amt) "0")
            (near/return amt)
            (if (str= (near/predecessor_account_id) (p5-bz "6|TOKX"))
                (if (str= (json-get-str "msg" inp) "sw")
                    (p5-swap-x sender amt)
                    (if (str= (json-get-str "msg" inp) "seed")
                        (begin
                          ;; pool-owned float, no position (bootstrap for
                          ;; swap out-legs; mint custody escrows its own)
                          (p5-put "6|ESCX" (li-add (p5-bz "6|ESCX") amt))
                          (near/return "0"))
                        (p5-mint-leg sender amt)))
                (if (str= (near/predecessor_account_id) (p5-bz "6|TOKY"))
                    (if (str= (json-get-str "msg" inp) "sw")
                        (p5-swap-y sender amt)
                        (if (str= (json-get-str "msg" inp) "seed")
                            (begin
                              (p5-put "6|ESCY" (li-add (p5-bz "6|ESCY") amt))
                              (near/return "0"))
                            (p5-mint-leg sender amt)))
                    (near/return amt))))))))

;; sell X for Y: escrow intake, walk via the same core as op "swap", then
;; promise the out-leg from 6|ESCY. Chain result (p5-pay's return) is what
;; ft2's resolve refunds.
(define (p5-swap-x sender amt)
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
          ;; up sells X for Y (p5-swap: v>0 = sell x)
          (p5-swarm-up s p xbook ybook amt)
          (p5-leg-decide sender amt "6|X" "6|ESCX" "6|ESCY" ps0 pl0 "6|TOKY")))))))))))

;; sell Y for X — mirror of p5-swap-x (out-leg from 6|ESCX on 6|TOKX)
(define (p5-swap-y sender amt)
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
          ;; down sells Y for X (p5-swap: v<0 = sell y)
          (p5-swarm-dn s p xbook ybook amt)
          (p5-leg-decide sender amt "6|Y" "6|ESCY" "6|ESCX" ps0 pl0 "6|TOKX")))))))))))

;; shared leg decision — reads 6|OUT/6|REM via bz (v4 pattern: NO
;; return-string parsing inside receipts):
;;   sentinel ⇒ walker moved nothing: undo escrow, refund all;
;;   out == 0 ⇒ degenerate (intake consumed, output floored): undo the
;;     intake-book delta + P/S + escrow, refund all;
;;   else ⇒ consume intake escrow (-c), promise the out-leg; p5-pay owns
;;     full reversal (X/Y/P/S + escrow -rm) on payout failure.
(define (p5-leg-decide sender amt ibook ekey okey ps0 pl0 tokkey)
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
            (let ((c (li-sub amt (p5-bz "6|REM"))))
              (begin
                (p5-put ekey (li-sub (p5-bz ekey) c))
                (p5-payout sender out (p5-bz "6|REM") c ps0 pl0 ekey okey tokkey)))))))

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
                      (li-sub (p5-bz (json-get-str "e" inp)) (json-get-str "m" inp)))
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
                        (near/return "0")))))))
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
;;   leg 1 records 6|PM:<sender> = "<X|Y>:a:b:d" and escrows d on its
;;   side; leg 2 matches a/b and the opposite side, requires ≥ req,
;;   completes the position (exact op-mint core). Both sides' REQUIRED
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
(define (p5-reqx i b s d acc)
  (if (> i b) acc
      (p5-reqx (+ i 1) b (p5-G (+ i 1)) d
               (li-add acc (p5-dx d s (p5-G (+ i 1)))))))

;; required Y: sum of dy(d, G(i), G(i+1)) — the Y-consumption form
;; (down-leg input, >> 64), rising (dyi over a rising walk would walk
;; the pair DOWN into limb-sub underflow — the trap this fixed).
(define (p5-reqy i b s d acc)
  (if (> i b) acc
      (p5-reqy (+ i 1) b (p5-G (+ i 1)) d
               (li-add acc (p5-dy d s (p5-G (+ i 1)))))))

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
            (begin (p5-put k (li-add (p5-bz k) d)) (recur (+ i 1))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))
    (if (< a (str->num (p5-bz "6|IMIN")))
        (p5-put "6|IMIN" (to-string a))
        0)
    (if (> b (str->num (p5-bz "6|IMAX")))
        (p5-put "6|IMAX" (to-string b))
        0)
    (near/return amt)))

;; ft_on_transfer mint/cancel leg (predecessor = a pool token, amount
;; validated). msg "c" cancels a pending mint: refund the recorded
;; slice via an escrow-backed ft_transfer_call (failure ⇒ p5-pay "mk"
;; re-credits the escrow).
(define (p5-mint-leg sender amt)
  (let ((msg (json-get-str "msg" (near/input))))
    (if (str= msg "c")
        (let ((rec (p5-bz (str-cat "6|PM:" sender))))
          (if (or (str= rec "") (str= rec "0"))
              (near/return amt)
              (let ((esc (p5-fld rec 4 58)))   ;; recorded escrow slice
              (let ((ek1 (str-cat "6|ESC" (str-slice rec 0 1))))
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
        (if (not (= (byte-at msg 0) 109))
            (near/return amt)
            (let ((d (p5-fld msg 1 58)))
            (let ((a (str->num (p5-fld msg 2 58))))
            (let ((b (str->num (p5-fld msg 3 58))))
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
                    ;; this side. EXACT attach only — leg 1's excess could
                    ;; never ride a refund (only leg 2's chain result goes
                    ;; through resolve), so sloppy legs are rejected whole
                    ;; (op "req" gives the caller the exact figure).
                    (let ((ga (p5-G a)))
                    (let ((req (if toky?
                                   (p5-reqy a b ga d "0")
                                   (p5-reqx a b ga d "0"))))
                      (if (not (= (li-cmp amt req) 0))
                          (p5-fail "ERR_EXACT")
                          (let ((ek (if toky? "6|ESCY" "6|ESCX")))
                            (begin
                              (p5-put (str-cat "6|PM:" sender)
                                (str-cat (if toky? "Y" "X")
                                  (str-cat ":" (str-cat (to-string a)
                                    (str-cat ":" (str-cat (to-string b)
                                      (str-cat ":" (str-cat d (str-cat ":" req)))))))))
                              (p5-put ek (li-add (p5-bz ek) req))
                              (near/return "0"))))))
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
                                (p5-mint-finish sender d a b
                                  (li-sub amt req))))))))))))))))))

;; op "burnc" {d,a,b} — custody burn: authority + exact op-burn core,
;; then owed ledgers from the required-amount closed forms. 6|RX/6|RY
;; carry the raw sums for callers/tests.
(define (p5-burnc)
  (let* ((inp (near/input))
         (who (near/predecessor_account_id))
         (d (json-get-str "d" inp))
         (a (str->num (json-get-str "a" inp)))
         (b (str->num (json-get-str "b" inp))))
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
    (let ((rx (p5-reqx a b ga d "0")))
    (let ((ry (p5-reqy a b ga d "0")))
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
        ;; owed = the required amount, capped at the escrow float
        (let ((cx (p5-req-cap rx "6|ESCX")))
        (let ((cy (p5-req-cap ry "6|ESCY")))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy))))))))))))))))

;; required-amount custody: owed = min(req, escrow float) — insolvency
;; guard only. req is token-denominated (same closed form at mint and
;; burn), so no unit scaling.
(define (p5-req-cap req ekey)
  (let ((e (p5-bz ekey)))
    (if (< (li-cmp req e) 1) req e)))

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

;; op "claim" {"t"} — pay one owed token per receipt. Failure ⇒ p5-pay
;; "cx"/"cy" restores owed + escrow (single-leg reversal, C2a pattern).
(define (p5-claim)
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((tok (json-get-str "t" inp)))
        (let ((w (if (str= tok "y")
                     (str-cat "6|OWY:" who)
                     (str-cat "6|OWX:" who))))
        (let ((e (if (str= tok "y") "6|ESCY" "6|ESCX")))
        (let ((tkey (if (str= tok "y") "6|TOKY" "6|TOKX")))
        (let ((k (if (str= tok "y") "cy" "cx"))
              (v (p5-bz w)))
          (if (= (li-cmp v "0") 0)
              "no-owed"
              (begin
                (p5-put w "0")
                (p5-put e (li-sub (p5-bz e) v))
                (let ((p (near/promise_batch_create (p5-bz tkey))))
                  (begin
                    (near/promise_batch_action_function_call p
                      "ft_transfer"
                      (json-set (json-set "{}" "receiver_id" who)
                                "amount" v)
                      "0" 40000000000000)
                    (near/promise_return
                      (near/promise_then p (near/current_account_id)
                        "pay"
                        (json-set (json-set (json-set (json-set
                          (json-set "{}" "k" k) "r" who) "o" v)
                          "w" w) "e" e)
                        "0" 5000000000000))
                    "claim-sent"))))))))))))

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
      (str-cat ":TOKY:" (p5-bz "6|TOKY")))))

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
    (if (str= op "burnc") (p5-burnc)
    (if (str= op "claim") (p5-claim)
    (if (str= op "req") (p5-req)
    (if (str= op "donate") (p5-donate)
    (if (str= op "swap") (p5-swap)
    (if (str= op "quote") (p5-quote)
    (if (str= op "get") (p5-get)
    (if (str= op "view") (p5-view)
    (if (str= op "escrow") (p5-escrow)
    (if (str= op "state") (p5-state)
    (if (str= op "g") (p5-G (str->num (json-get-str "i" (near/input))))
        "unknown-op"))))))))))))))))

(export "_run" p5-run)
(export "ft_on_transfer" p5-on-transfer)
(export "pay" p5-pay)
