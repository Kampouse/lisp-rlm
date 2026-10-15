;; accrual-mount harness — the proven fee-ledger core, exercised at pool scale.
;; Mount design: port/accrual-mount.md (PS section).
;;
;; ONE global per-share fee accumulator in 2^64 fixed point:
;;   PS += f * 2^64 // SHT          (fee wave, H1)
;;   CK:<lp> := PS                  (deposit, H2)
;;   owed = (PS - CK:<lp>) * sh // 2^64   (claim, H3)
;; This IS accruef (Aggregate_Accrual.thy) with constant cap Lf = sh:
;; theorem accrue_within_charged_book => total owed <= total fees,
;; floored, fail-closed. Late LPs checkpoint at entry => no skim.
;;
;; Harness shorthands vs the real pool (deliberate, under test is the
;; LEDGER): m-deposit takes lp + amt directly (v4 mints PB-proportionally
;; at ft_on_transfer); scenario numbers are generator-computed
;; (gen_scenario.py) — zero hand-typed expectations.
;;
;; u128-muldiv(a, b, d) = a*b//d: muldiv(f, TWO64, SHT) and
;; muldiv(dPS, sh, TWO64) are exactly the two divs above. Sibling-call
;; discipline: one call per let* binding (compiler clobber rule).

(define (sput k v) (near/storage_set k v))
(define (sget k) (default (near/storage_get k) "0"))
(define (fail m) (near/log m) (near/abort m))
(define (num-ok? s)
  (if (> (str-length s) 40) "0"
      (loop ((i 0))
        (if (>= i (str-length s)) "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9)) "0" (recur (+ i 1))))))))

(define TWO64 "18446744073709551616")

;; H1: fee wave f -> PS += f*TWO64//SHT
(define (m-fee)
  (let* ((inp (near/input))
         (fs (json-get-str "f" inp))
         (ok (num-ok? fs))
         (g (if (str= ok "0") (fail "ERR_NUM") "ok"))
         (f fs)
         (sht (sget "SHT"))
         (z (if (str= sht "0") (fail "ERR_NO_LPS") "ok"))
         (inc (u128-muldiv f TWO64 sht))
         (ps (sget "PS"))
         (ps2 (li-add ps inc)))
    (begin (sput "PS" ps2) (near/return (str-cat "PS:" ps2)))))

;; H2: deposit mints amt shares, checkpoint at current PS
(define (m-deposit)
  (let* ((inp (near/input))
         (lp (json-get-str "lp" inp))
         (amts (json-get-str "amt" inp))
         (ok (num-ok? amts))
         (g0 (if (str= ok "0") (fail "ERR_NUM") "ok"))
         (amt amts)
         (ten (li-cmp amt "10"))
         (g1 (if (< ten 0) (fail "ERR_MIN") "ok"))
         (ps (sget "PS"))
         (ckk (str-cat "CK:" lp))
         (shk (str-cat "SH:" lp))
         (sh0 (sget shk))
         (sh1 (li-add sh0 amt))
         (sht (sget "SHT"))
         (sht1 (li-add sht amt)))
    (begin
      (sput shk sh1)
      (sput "SHT" sht1)
      (sput ckk ps)
      (near/return (str-cat "SHT:" sht1)))))

;; H3: settle owed = (PS - CK)*sh//TWO64, credit BAL, checkpoint := PS
(define (m-claim)
  (let* ((inp (near/input))
         (lp (json-get-str "lp" inp))
         (ckk (str-cat "CK:" lp))
         (shk (str-cat "SH:" lp))
         (blk (str-cat "BAL:" lp))
         (ck (sget ckk))
         (ps (sget "PS"))
         (dps (li-sub ps ck))
         (sh (sget shk))
         (owed (u128-muldiv dps sh TWO64))
         (b0 (sget blk))
         (b1 (li-add b0 owed)))
    (begin
      (sput blk b1)
      (sput ckk ps)
      (near/return (str-cat "BAL:" lp "=" b1)))))

(define (m-view)
  (let* ((sht (sget "SHT"))
         (ps (sget "PS"))
         (b1 (sget "BAL:lp1.test.near"))
         (b2 (sget "BAL:lp2.test.near"))
         (b3 (sget "BAL:lp3.test.near"))
         (t (li-add b1 (li-add b2 b3))))
    (near/return (str-cat "SHT:" sht " PS:" ps " OWEDSUM:" t))))

(define (m-init)
  (begin
    (sput "SHT" "0")
    (sput "PS" "0")
    (near/return "SHT:0")))

(export "m-init" m-init)
(export "m-fee" m-fee)
(export "m-deposit" m-deposit)
(export "m-claim" m-claim)
(export "m-view" m-view)
