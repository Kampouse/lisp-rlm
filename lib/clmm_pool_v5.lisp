;; ═══════════════════════════════════════════════════════════════════════
;; lib/clmm_pool_v5.lisp — CLMM v5 concentrated-liquidity pool (M3)
;;
;; Build: concat limb_math + clmm_v5_kernel + clmm_v5_dispatch_tail +
;; THIS file (generated — json chains machine-built, see M3 notes).
;; Pool reuses the tail's string adapters (v5-op-spow, v5-op-dy/dx,
;; v5-op-sfdyi, v5-op-fee) and kernel decimal helpers (v5-li-*,
;; u128-muldiv). The pool itself is string-only.
;;
;; Design: docs/v5-m3-pool-design.md. Key semantics:
;;   • L(segment g) = Σ_{q ≤ g} NET[q]; up-cross into b: L += NET[b];
;;     down-cross leaving segment g: L -= NET[g]. Walks jump between
;;     NONZERO NET points (path-independent at constant L) and terminate
;;     because L > 0 ⟹ boundaries exist on both sides.
;;   • Fees: charged up front, one growth step at entry L0
;;     (FG += fee·2^128 // L0). L = 0 swaps are full-refund no-ops.
;;   • Up-walk partial: S2max = floor((L·S − 1)/(L − (rem+1)·S)) capped
;;     2^127 (largest S2 with dx_in ≤ rem; the ref's S_for_dx is
;;     degenerate — design notes). Down-walk partial: v5-op-sfdyi.
;;   • Impossible states TRAP (p5-trap); expected failures refund.
;;   • Storage prefix 5| (no v4 collisions). Values are decimal strings.
;; ═══════════════════════════════════════════════════════════════════════

(define (p5-c64) "18446744073709551616")
(define (p5-c128) "340282366920938463463374607431768211456")
(define (p5-c127) "170141183460469231731687303715884105728")
(define (p5-pmax) 800000)

(define (p5-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (p5-sv k v) (near/store-bytes k v))
(define (p5-trap) (v5-li-sub "0" "1"))

;; ── parsers ────────────────────────────────────────────────────────────

;; k-th field of s split on char code c (':' = 58, '|' = 124)
(define (p5-part s c k)
  (loop ((i 0) (kk 0) (start 0))
    (if (>= i (str-length s))
        (if (= kk k) (str-substring s start i) "")
        (if (= (byte-at s i) c)
            (if (= kk k) (str-substring s start i) (recur (+ i 1) (+ kk 1) (+ i 1)))
            (recur (+ i 1) kk start)))))

(define (p5-digits-ok s)
  (if (< (str-length s) 1)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9)) "0" (recur (+ i 1))))))))

(define (p5-num-ok s)
  (if (> (str-length s) 40) "0" (p5-digits-ok s)))

(define (p5-int-ok s)
  (if (< (str-length s) 1)
      "0"
      (if (str= (str-substring s 0 1) "-")
          (if (< (str-length s) 2) "0"
              (if (> (str-length s) 11) "0"
                  (p5-digits-ok (str-substring s 1 (str-length s)))))
          (if (> (str-length s) 10) "0" (p5-digits-ok s)))))

(define (p5-parse-int s)
  (if (str= (str-substring s 0 1) "-")
      (- 0 (str->num (str-substring s 1 (str-length s))))
      (str->num s)))

;; ── signed string arithmetic (NET entries may be negative) ────────────

(define (p5-neg? s) (str= (str-substring s 0 1) "-"))

(define (p5-ssub a b)
  (if (p5-neg? b)
      (v5-li-add a (str-substring b 1 (str-length b)))
      (if (>= (v5-li-cmp a b) 0)
          (v5-li-sub a b)
          (str-cat "-" (v5-li-sub b a)))))

(define (p5-sadd a b)
  (if (p5-neg? b)
      (p5-ssub a (str-substring b 1 (str-length b)))
      (v5-li-add a b)))

;; ── storage accessors ──────────────────────────────────────────────────

(define (p5-net-key p) (str-cat "5|NET:" (to-string p)))
(define (p5-net-get p) (p5-bz (p5-net-key p)))
(define (p5-net-nz p) (not (str= (p5-net-get p) "0")))
(define (p5-net-set p v)
  (if (str= v "0") (near/storage_remove (p5-net-key p)) (p5-sv (p5-net-key p) v)))

(define (p5-pos-key w pl pr)
  (str-cat (str-cat "5|POS:" w) (str-cat ":" (to-string pl)) (str-cat ":" (to-string pr))))
(define (p5-pos-get w pl pr)
  (let ((v (near/load-bytes (p5-pos-key w pl pr))))
    (if (nil? v) "0|0|0|0|0" v)))
(define (p5-pos-field s k) (p5-part s 124 k))
(define (p5-pend-key w) (str-cat "5|PEND:" w))

;; a + "|" + b — record join building block
(define (p5-cat2 a b) (str-cat a (str-cat "|" b)))

;; collect accrued fees into owed, then grow liquidity by ladd
(define (p5-pos-add w pl pr ladd)
  (let ((old (p5-pos-get w pl pr)))
    (let ((l0 (p5-pos-field old 0)))
      (let ((ox (p5-pos-field old 1)))
        (let ((oy (p5-pos-field old 2)))
          (let ((gx (p5-pos-field old 3)))
            (let ((gy (p5-pos-field old 4)))
              (let ((fgx (p5-bz "5|FGX")))
                (let ((fgy (p5-bz "5|FGY")))
                  (let ((cx (if (> (v5-li-cmp l0 "0") 0)
                                (u128-muldiv l0 (v5-li-sub fgx gx) (p5-c128))
                                "0")))
                    (let ((cy (if (> (v5-li-cmp l0 "0") 0)
                                  (u128-muldiv l0 (v5-li-sub fgy gy) (p5-c128))
                                  "0")))
                      (p5-sv (p5-pos-key w pl pr)
                        (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2
                          (v5-li-add l0 ladd) (v5-li-add ox cx))
                          (v5-li-add oy cy)) fgx) fgy)))))))))))))

;; NET bookkeeping + active-L bump when the range straddles the segment
(define (p5-commit-range w pl pr l)
  (let ((P (p5-parse-int (p5-bz "5|P"))))
    (begin
      (p5-net-set pl (p5-sadd (p5-net-get pl) l))
      (p5-net-set pr (p5-ssub (p5-net-get pr) l))
      (if (and (>= P pl) (< P pr))
          (let ((L2 (v5-li-add (p5-bz "5|L") l)))
            (if (p5-neg? L2) (p5-trap) (p5-sv "5|L" L2)))
          0)
      (p5-pos-add w pl pr l))))

;; next nonzero NET strictly above p (≤ 4096 probes); 999999 = none
(define (p5-next-above p d)
  (loop ((q (+ p d)) (n 0))
    (if (or (> n 4096) (> q (p5-pmax)))
        999999
        (if (p5-net-nz q) q (recur (+ q d) (+ n 1))))))

;; next nonzero NET at or below p; -999999 = none
(define (p5-next-below p d)
  (if (p5-net-nz p)
      p
      (loop ((q (- p d)) (n 0))
        (if (or (> n 4096) (< q (- 0 (p5-pmax))))
            -999999
            (if (p5-net-nz q) q (recur (- q d) (+ n 1)))))))

;; largest S2 with dx_in(L,S,S2) ≤ rem, capped 2^127. rem ≥ 1.
(define (p5-s2max-dx L S rem)
  (let ((T (u128-muldiv (v5-li-add rem "1") S "1")))
    (if (<= (v5-li-cmp L T) 0)
        (p5-c127)
        (let ((LS (u128-muldiv L S "1")))
          (let ((r (u128-muldiv (v5-li-sub LS "1") "1" (v5-li-sub L T))))
            (if (> (v5-li-cmp r (p5-c127)) 0) (p5-c127) r))))))

;; ── init / state ───────────────────────────────────────────────────────

(define (p5-init)
  (if (not (str= (p5-bz "5|INIT") "0"))
      (near/return "already")
      (let ((inp (near/input)))
        (let ((fee (json-get-str "fee" inp)))
          (let ((pd (json-get-str "pd" inp)))
            (let ((p0 (json-get-str "p0" inp)))
              (if (or (str= (p5-num-ok fee) "0") (str= (p5-int-ok pd) "0")
                      (str= (p5-int-ok p0) "0"))
                  (near/return "bad-args")
                  (let ((fv (p5-parse-int fee)))
                    (let ((dv (p5-parse-int pd)))
                      (let ((pv (p5-parse-int p0)))
                        (if (or (not (or (= fv 0) (= fv 100) (= fv 400)
                                         (= fv 2000) (= fv 10000)))
                                (not (or (= dv 1) (= dv 8) (= dv 40) (= dv 200)))
                                (< pv (- 0 (p5-pmax))) (> pv (p5-pmax)))
                            (near/return "bad-tier")
                            (begin
                              (p5-sv "5|INIT" "1")
                              (p5-sv "5|TOKX" (json-get-str "tokx" inp))
                              (p5-sv "5|TOKY" (json-get-str "toky" inp))
                              (p5-sv "5|FEE" fee)
                              (p5-sv "5|PD" pd)
                              (p5-sv "5|P" p0)
                              (p5-sv "5|S" (v5-op-spow pv))
                              (p5-sv "5|L" "0")
                              (p5-sv "5|X" "0")
                              (p5-sv "5|Y" "0")
                              (p5-sv "5|FGX" "0")
                              (p5-sv "5|FGY" "0")
                              (near/return "ok")))))))))))))

(define (p5-state)
  (near/return (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "P" (p5-bz "5|P")) "S" (p5-bz "5|S")) "L" (p5-bz "5|L")) "X" (p5-bz "5|X")) "Y" (p5-bz "5|Y")) "FGX" (p5-bz "5|FGX")) "FGY" (p5-bz "5|FGY")) "PD" (p5-bz "5|PD")) "FEE" (p5-bz "5|FEE"))))

;; ── swap walks (pure locals) → (P S L rem out) ─────────────────────────

(define (p5-walk-up d pi si li ri)
  (loop ((P pi) (S si) (L li) (Rem ri) (Out "0"))
    (if (or (str= Rem "0") (str= L "0"))
        (list P S L Rem Out)
        (let ((q (p5-next-above P d)))
          (if (= q 999999)
              (list P S L Rem Out)
              (let ((Sb (v5-op-spow q)))
                (let ((s2m (p5-s2max-dx L S Rem)))
                  (if (>= (v5-li-cmp s2m Sb) 0)
                      (let ((take (v5-op-dx L S Sb)))
                        (let ((out2 (v5-li-add Out (v5-op-dy L S Sb))))
                          (let ((rem2 (v5-li-sub Rem take)))
                            (let ((L2 (p5-sadd L (p5-net-get q))))
                              (if (p5-neg? L2) (p5-trap)
                                  (recur q Sb L2 rem2 out2))))))
                      (let ((take (v5-op-dx L S s2m)))
                        (list P s2m L (v5-li-sub Rem take)
                              (v5-li-add Out (v5-op-dy L S s2m))))))))))))

(define (p5-walk-down d pi si li ri)
  (loop ((P pi) (S si) (L li) (Rem ri) (Out "0"))
    (if (or (str= Rem "0") (str= L "0"))
        (list P S L Rem Out)
        (let ((q (p5-next-below P d)))
          (if (= q -999999)
              (list P S L Rem Out)
              (let ((Sb (v5-op-spow q)))
                (let ((target (v5-op-sfdyi L S Rem)))
                  (if (<= (v5-li-cmp target Sb) 0)
                      (let ((out2 (v5-li-add Out (v5-op-dx L Sb S))))
                        (let ((rem2 (v5-li-sub Rem (v5-op-dy L Sb S))))
                          (let ((L2 (p5-ssub L (p5-net-get P))))
                            (let ((nP (if (= q P) (- P d) q)))
                              (if (p5-neg? L2) (p5-trap)
                                  (recur nP Sb L2 rem2 out2))))))
                      (list P target L (v5-li-sub Rem (v5-op-dy L target S))
                            (v5-li-add Out (v5-op-dx L target S)))))))))))

;; ── swap settle: dir 0 = x-in (up, pay y) · dir 1 = y-in (down, pay x) ──

(define (p5-swap amt minout sender dir)
  (let ((L0 (p5-bz "5|L")))
    (if (or (str= (p5-bz "5|INIT") "0") (str= L0 "0"))
        (near/return amt)
        (let ((p0 (p5-bz "5|P")))
          (let ((s0 (p5-bz "5|S")))
            (let ((fx0 (p5-bz "5|FGX")))
              (let ((fy0 (p5-bz "5|FGY")))
                (let ((x0 (p5-bz "5|X")))
                  (let ((y0 (p5-bz "5|Y")))
                    (let ((fee (v5-op-fee amt (p5-bz "5|FEE"))))
                      (let ((rem (v5-li-sub amt fee)))
                        (if (str= rem "0")
                            (near/return amt)
                            (let ((d (p5-parse-int (p5-bz "5|PD"))))
                              (let ((r (if (= dir 0)
                                           (p5-walk-up d (p5-parse-int p0) s0 L0 rem)
                                           (p5-walk-down d (p5-parse-int p0) s0 L0 rem))))
                                (let ((out (car (cdr (cdr (cdr (cdr r)))))))
                                  (let ((nrem (car (cdr (cdr (cdr r))))))
                                    (let ((nL (car (cdr (cdr r)))))
                                      (let ((nS (car (cdr r))))
                                        (let ((nP (car r)))
                                          (if (< (v5-li-cmp out minout) 0)
                                              (near/return amt)
                                              (begin
                                                (p5-sv "5|P" (to-string nP))
                                                (p5-sv "5|S" nS)
                                                (p5-sv "5|L" nL)
                                                (if (= dir 0)
                                                    (p5-sv "5|FGX" (v5-li-add fx0
                                                              (u128-muldiv fee (p5-c128) L0)))
                                                    (p5-sv "5|FGY" (v5-li-add fy0
                                                              (u128-muldiv fee (p5-c128) L0))))
                                                (let ((consumed (v5-li-sub amt nrem)))
                                                  (begin
                                                    (if (= dir 0)
                                                        (begin
                                                          (p5-sv "5|X" (v5-li-add x0 consumed))
                                                          (p5-sv "5|Y" (v5-li-sub y0 out)))
                                                        (begin
                                                          (p5-sv "5|Y" (v5-li-add y0 consumed))
                                                          (p5-sv "5|X" (v5-li-sub x0 out))))
                                                    (if (str= out "0")
                                                        (near/return nrem)
                                                        (p5-pay-swap dir sender out amt
                                                          nrem p0 s0 L0 fx0 fy0 x0 y0)))))))))))))))))))))))))

(define (p5-pay-swap dir sender out amt rem p0 s0 l0 fx0 fy0 x0 y0)
  (let ((tok (if (= dir 0) (p5-bz "5|TOKY") (p5-bz "5|TOKX"))))
    (let ((p (near/promise_batch_create tok)))
      (let ((cb (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "a" amt) "r" rem) "p0" p0) "s0" s0) "l0" l0) "fx0" fx0) "fy0" fy0) "x0" x0) "y0" y0) "dir" (to-string dir)) "w" sender)))
        (begin
          (near/promise_batch_action_function_call p "ft_transfer"
            (json-set (json-set "{}" "receiver_id" sender) "amount" out)
            "0" 40000000000000)
          (near/promise_return
            (near/promise_then p (near/current_account_id) "pay_out" cb
              "0" 60000000000000)))))))

(define (p5-payout)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (begin
                (p5-sv "5|P" (json-get-str "p0" inp))
                (p5-sv "5|S" (json-get-str "s0" inp))
                (p5-sv "5|L" (json-get-str "l0" inp))
                (p5-sv "5|FGX" (json-get-str "fx0" inp))
                (p5-sv "5|FGY" (json-get-str "fy0" inp))
                (p5-sv "5|X" (json-get-str "x0" inp))
                (p5-sv "5|Y" (json-get-str "y0" inp))
                (near/return (json-get-str "a" inp)))
              (near/return (json-get-str "r" inp)))))
      (near/return "forbidden")))

;; ── add liquidity ──────────────────────────────────────────────────────
;; Two-leg, user-driven (the pool cannot pull from an LP: ft_transfer_call
;; moves caller funds — design notes). Straddle adds: LP sends side A with
;; msg "add:<pl>:<pr>" (books A immediately, PEND records the liability),
;; then side B with the same msg; leg 2 commits NET/position/active-L.
;; addcancel returns the held side. Pure ranges stay single-leg.

(define (p5-add amt pl pr who side)
  ;; an existing matching PEND makes this leg 2
  (let ((pv (near/load-bytes (p5-pend-key who))))
    (if (and (not (nil? pv))
             (str= (p5-part pv 124 0) (to-string pl))
             (str= (p5-part pv 124 1) (to-string pr)))
        (p5-add-leg2 amt pl pr who side pv)
        (p5-add-leg1 amt pl pr who side))))

(define (p5-add-leg1 amt pl pr who side)
  (let ((P (p5-parse-int (p5-bz "5|P"))))
    (let ((S (p5-bz "5|S")))
      (let ((Spl (v5-op-spow pl)))
        (let ((Spr (v5-op-spow pr)))
          (let ((diff (v5-li-sub Spr Spl)))
            (if (> pl P)
                (if (not (str= side "x"))
                    (near/return amt)
                    (let ((A (u128-muldiv Spl Spr "1")))
                      (let ((L (u128-muldiv amt A diff)))
                        (if (str= L "0")
                            (near/return amt)
                            (let ((xu (u128-muldiv (u128-muldiv L diff "1") "1" A)))
                              (begin
                                (p5-commit-range who pl pr L)
                                (p5-sv "5|X" (v5-li-add (p5-bz "5|X") xu))
                                (near/return (v5-li-sub amt xu))))))))
                (if (<= pr P)
                    (if (not (str= side "y"))
                        (near/return amt)
                        (let ((L (u128-muldiv amt (p5-c64) diff)))
                      (if (str= L "0")
                          (near/return amt)
                          (let ((yu (u128-muldiv L diff (p5-c64))))
                            (begin
                              (p5-commit-range who pl pr L)
                              (p5-sv "5|Y" (v5-li-add (p5-bz "5|Y") yu))
                              (near/return (v5-li-sub amt yu)))))))
                    ;; straddle: book this side, PEND the other
                    (let ((held (p5-add-straddle amt pl pr who side P S Spl Spr)))
                      held)))))))))

(define (p5-add-straddle amt pl pr who side P S Spl Spr)
  (if (str= side "x")
      (p5-add-strx amt pl pr who P S Spl Spr)
      (p5-add-stry amt pl pr who P S Spl Spr)))

(define (p5-add-strx amt pl pr who P S Spl Spr)
  (let ((A (u128-muldiv S Spr "1")))
    (let ((SprS (v5-li-sub Spr S)))
      (let ((L (u128-muldiv amt A SprS)))
        (if (str= L "0")
            amt
            (let ((xu (u128-muldiv (u128-muldiv L SprS "1") "1" A)))
              (let ((Sspl (v5-li-sub S Spl)))
                (let ((yn (u128-muldiv L Sspl (p5-c64))))
                  (begin
                    (p5-sv "5|X" (v5-li-add (p5-bz "5|X") xu))
                    (p5-sv (p5-pend-key who)
                      (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2
                        (to-string pl) (to-string pr)) L) yn) xu)
                        (p5-cat2 (p5-cat2 S "1") (v5-li-sub amt xu))))
                    (v5-li-sub amt xu))))))))))

(define (p5-add-stry amt pl pr who P S Spl Spr)
  (let ((Sspl (v5-li-sub S Spl)))
    (if (str= Sspl "0")
        amt
        (let ((L (u128-muldiv amt (p5-c64) Sspl)))
          (if (str= L "0")
              amt
              (let ((yu (u128-muldiv L Sspl (p5-c64))))
                (let ((SprS (v5-li-sub Spr S)))
                  (let ((xn (u128-muldiv (u128-muldiv L SprS "1") "1"
                                         (u128-muldiv S Spr "1"))))
                    (begin
                      (p5-sv "5|Y" (v5-li-add (p5-bz "5|Y") yu))
                      (p5-sv (p5-pend-key who)
                        (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2
                          (to-string pl) (to-string pr)) L) xn) yu)
                          (p5-cat2 (p5-cat2 S "0") (v5-li-sub amt yu))))
                      (v5-li-sub amt yu))))))))))

;; leg 2: pv = pl|pr|L|need|held|S|side(0=y-held,1=x-held)|refd
(define (p5-add-leg2 amt pl pr who side pv)
  (let ((L (p5-part pv 124 2)))
    (let ((need (p5-part pv 124 3)))
      (let ((held (p5-part pv 124 4)))
        (let ((pendS (p5-part pv 124 5)))
          (let ((hside (p5-part pv 124 6)))
            ;; delivering side must be the opposite of the held side
            (if (not (str= hside (if (str= side "x") "0" "1")))
                (near/return amt)
                (if (not (str= (p5-bz "5|S") pendS))
                    (near/return amt)   ;; price moved: retry after addcancel
                    (if (< (v5-li-cmp amt need) 0)
                        (near/return amt)   ;; underfunded leg 2: refund, PEND stays
                        (begin
                          (p5-commit-range who pl pr L)
                          (if (str= side "x")
                              (p5-sv "5|X" (v5-li-add (p5-bz "5|X") need))
                              (p5-sv "5|Y" (v5-li-add (p5-bz "5|Y") need)))
                          (near/storage_remove (p5-pend-key who))
                          (v5-li-sub amt need)))))))))))

;; addcancel: signer refunds their held straddle side, PEND cleared
(define (p5-addcancel)
  (if (str= (p5-bz "5|INIT") "0")
      (near/return "uninit")
      (let ((who (near/predecessor_account_id)))
        (let ((pv (near/load-bytes (p5-pend-key who))))
          (if (nil? pv)
              (near/return "no-pend")
              (let ((held (p5-part pv 124 4)))
                (let ((hside (p5-part pv 124 6)))
                  (begin
                    (if (str= hside "1")
                        (p5-sv "5|X" (v5-li-sub (p5-bz "5|X") held))
                        (p5-sv "5|Y" (v5-li-sub (p5-bz "5|Y") held)))
                    (near/storage_remove (p5-pend-key who))
                    (let ((p (near/promise_batch_create
                               (if (str= hside "1") (p5-bz "5|TOKX") (p5-bz "5|TOKY")))))
                      (begin
                        (near/promise_batch_action_function_call p "ft_transfer"
                          (json-set (json-set "{}" "receiver_id" who) "amount" held)
                          "0" 40000000000000)
                        (near/promise_return
                          (near/promise_then p (near/current_account_id) "pay_out"
                            (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "a" held) "r" "0") "p0" (p5-bz "5|P")) "s0" (p5-bz "5|S")) "l0" (p5-bz "5|L")) "fx0" (p5-bz "5|FGX")) "fy0" (p5-bz "5|FGY")) "x0" (if (str= hside "1") (v5-li-add (p5-bz "5|X") held) (p5-bz "5|X"))) "y0" (if (str= hside "1") (p5-bz "5|Y") (v5-li-add (p5-bz "5|Y") held))) "dir" "0") "w" who)
                            "0" 60000000000000))))))))))))

;; ── remove liquidity ───────────────────────────────────────────────────

(define (p5-remove)
  (let ((inp (near/input)))
    (if (str= (p5-bz "5|INIT") "0")
        (near/return "uninit")
        (let ((who (near/predecessor_account_id)))
          (let ((plstr (json-get-str "pl" inp)))
            (let ((prstr (json-get-str "pr" inp)))
              (let ((lb (json-get-str "liq" inp)))
                (if (or (str= (p5-int-ok plstr) "0")
                        (str= (p5-int-ok prstr) "0")
                        (str= (p5-num-ok lb) "0"))
                    (near/return "bad-args")
                    (let ((pl (p5-parse-int plstr)))
                      (let ((pr (p5-parse-int prstr)))
                        (let ((pos (p5-pos-get who pl pr)))
                          (let ((l0pos (p5-pos-field pos 0)))
                            (if (< (v5-li-cmp l0pos lb) 0)
                                (near/return "insufficient")
                                (p5-remove2 who pl pr lb pos l0pos))))))))))))))

(define (p5-remove2 who pl pr lb pos l0pos)
  (let ((P (p5-parse-int (p5-bz "5|P"))))
    (let ((S (p5-bz "5|S")))
      (let ((Spl (v5-op-spow pl)))
        (let ((Spr (v5-op-spow pr)))
          (let ((ox (p5-pos-field pos 1)))
            (let ((oy (p5-pos-field pos 2)))
              (let ((gx (p5-pos-field pos 3)))
                (let ((gy (p5-pos-field pos 4)))
                  (let ((fgx (p5-bz "5|FGX")))
                    (let ((fgy (p5-bz "5|FGY")))
                      (let ((cx (u128-muldiv l0pos (v5-li-sub fgx gx) (p5-c128))))
                        (let ((cy (u128-muldiv l0pos (v5-li-sub fgy gy) (p5-c128))))
                          (let ((tox (v5-li-add ox cx)))
                            (let ((toy (v5-li-add oy cy)))
                              (let ((pox (u128-muldiv tox lb l0pos)))
                                (let ((poy (u128-muldiv toy lb l0pos)))
                                  (let ((npl (p5-net-get pl)))
                                    (let ((npr (p5-net-get pr)))
                                      (let ((l0act (p5-bz "5|L")))
                                        (let ((x0 (p5-bz "5|X")))
                                          (let ((y0 (p5-bz "5|Y")))
                                            (p5-remove3 who pl pr lb pos l0pos
                                              P S Spl Spr pox poy tox toy
                                              npl npr l0act x0 y0 fgx fgy
                                              ox oy)))))))))))))))))))))))

(define (p5-remove3 who pl pr lb pos l0pos P S Spl Spr pox poy tox toy
                    npl npr l0act x0 y0 fgx fgy ox oy)
  (let ((diff (v5-li-sub Spr Spl)))
    (let ((res
           (if (and (>= P pl) (< P pr))
               (list (u128-muldiv (u128-muldiv lb (v5-li-sub Spr S) "1") "1"
                                  (u128-muldiv S Spr "1"))
                     (u128-muldiv lb (v5-li-sub S Spl) (p5-c64)))
               (if (<= pr P)
                   (list "0" (u128-muldiv lb diff (p5-c64)))
                   (list (u128-muldiv (u128-muldiv lb diff "1") "1"
                                      (u128-muldiv Spl Spr "1")) "0")))))
      (let ((tx (v5-li-add (car res) pox)))
        (let ((ty (v5-li-add (car (cdr res)) poy)))
          (begin
            (p5-net-set pl (p5-ssub npl lb))
            (p5-net-set pr (p5-sadd npr lb))
            (let ((l2 (v5-li-sub l0pos lb)))
              (if (str= l2 "0")
                  (near/storage_remove (p5-pos-key who pl pr))
                  (p5-sv (p5-pos-key who pl pr)
                    (p5-cat2 (p5-cat2 (p5-cat2 (p5-cat2 l2
                      (v5-li-sub tox pox)) (v5-li-sub toy poy)) fgx) fgy)))))
            (if (and (>= P pl) (< P pr))
                (p5-sv "5|L" (v5-li-sub l0act lb))
                0)
            (p5-sv "5|X" (v5-li-sub x0 tx))
            (p5-sv "5|Y" (v5-li-sub y0 ty))
            (if (and (str= tx "0") (str= ty "0"))
                (near/return "rm-ok")
                (let ((leg1tok (if (not (str= tx "0")) (p5-bz "5|TOKX") (p5-bz "5|TOKY"))))
                  (let ((leg1amt (if (not (str= tx "0")) tx ty)))
                    (let ((p (near/promise_batch_create leg1tok)))
                      (let ((cb (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "w" who) "pl" (to-string pl)) "pr" (to-string pr)) "lb" lb) "npl" npl) "npr" npr) "pos0" pos) "l0" l0act) "x0" x0) "y0" y0) "tx" tx) "ty" ty)))
                        (begin
                          (near/promise_batch_action_function_call p "ft_transfer"
                            (json-set (json-set "{}" "receiver_id" who) "amount" leg1amt)
                            "0" 40000000000000)
                          (near/promise_return
                            (near/promise_then p (near/current_account_id) "pay_rm"
                              cb "0" 60000000000000)))))))))))))

;; pay_rm: leg-1 fail → full restore; success → leg 2 (unchecked, v4 gap)
(define (p5-payrm)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (let ((pl (p5-parse-int (json-get-str "pl" inp))))
                (let ((pr (p5-parse-int (json-get-str "pr" inp))))
                  (begin
                    (p5-net-set pl (json-get-str "npl" inp))
                    (p5-net-set pr (json-get-str "npr" inp))
                    (p5-sv (p5-pos-key (json-get-str "w" inp) pl pr)
                           (json-get-str "pos0" inp))
                    (p5-sv "5|L" (json-get-str "l0" inp))
                    (p5-sv "5|X" (json-get-str "x0" inp))
                    (p5-sv "5|Y" (json-get-str "y0" inp))
                    (near/return "rm-failed"))))
              (let ((tx (json-get-str "tx" inp)))
                (let ((ty (json-get-str "ty" inp)))
                  (if (and (not (str= tx "0")) (not (str= ty "0")))
                      (let ((p (near/promise_batch_create (p5-bz "5|TOKY"))))
                        (begin
                          (near/promise_batch_action_function_call p "ft_transfer"
                            (json-set (json-set "{}" "receiver_id"
                                        (json-get-str "w" inp))
                              "amount" ty)
                            "0" 40000000000000)
                          (near/promise_return p)))
                      (near/return "rm-ok")))))))
      (near/return "forbidden")))

;; ── ft_on_transfer dispatcher ──────────────────────────────────────────

(define (p5-ft)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (str= (p5-num-ok amt) "0")
            (near/return amt)
            (if (str= (p5-bz "5|INIT") "0")
                (near/return amt)
                (let ((msg (json-get-str "msg" inp)))
                  (let ((cmd (p5-part msg 58 0)))
                    (cond
                      ((str= cmd "swap")
                       (let ((mn (p5-part msg 58 1)))
                         (if (str= mn "")
                             (near/return amt)
                         (if (str= (p5-num-ok mn) "0")
                             (near/return amt)
                             (if (str= (near/predecessor_account_id) (p5-bz "5|TOKX"))
                                 (p5-swap amt mn sender 0)
                                 (if (str= (near/predecessor_account_id) (p5-bz "5|TOKY"))
                                     (p5-swap amt mn sender 1)
                                     (near/return amt)))))))
                      ((str= cmd "add")
                       (let ((plstr (p5-part msg 58 1)))
                         (let ((prstr (p5-part msg 58 2)))
                           (if (or (str= (p5-int-ok plstr) "0")
                                   (str= (p5-int-ok prstr) "0"))
                               (near/return amt)
                               (let ((pl (p5-parse-int plstr)))
                                 (let ((pr (p5-parse-int prstr)))
                                   (let ((d (p5-parse-int (p5-bz "5|PD"))))
                                     (if (or (>= pl pr)
                                             (not (= (mod pl d) 0))
                                             (not (= (mod pr d) 0))
                                             (< pl (- 0 (p5-pmax)))
                                             (> pr (p5-pmax)))
                                         (near/return amt)
                                         (if (str= (near/predecessor_account_id)
                                                   (p5-bz "5|TOKX"))
                                             (p5-add amt pl pr sender "x")
                                             (if (str= (near/predecessor_account_id)
                                                      (p5-bz "5|TOKY"))
                                                 (p5-add amt pl pr sender "y")
                                                 (near/return amt)))))))))))
                                            (else (near/return amt)))))))))))

(export "ft_on_transfer" p5-ft)
(export "init" p5-init)
(export "remove_liq" p5-remove)
(export "addcancel" p5-addcancel)
(export "pay_out" p5-payout)
(export "pay_rm" p5-payrm)
(export "state" p5-state true)
