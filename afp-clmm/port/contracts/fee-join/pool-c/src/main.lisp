;; CLMM grid pool (AFP CLMM_Operations port) — pool C: fee-union leg P2, grid = A's, fee 0.5%
;; grid 1000000000 / 2000000000 / 4000000000   net liq [40000000000000000000000, 90000000000000000000000]
;; fee phi = 5000000000000000/1000000000000000000: gross book Q=ceil(L*den/(den-num)) per cell
;; (AFP gross_fct L/(1-phi), fixed-point ceil, precomputed by generator).
;; swap: deposit=y, optional "start" (default G0; interior entry = the
;; refine/slice identity pin); out += l*dp/(sqp*p2) via u128/muldiv.
;; ledger: paid:<caller> accumulates out; left:<caller> = refunded dust.
(define G0 "1000000000")
(define G1 "2000000000")
(define G2 "4000000000")
(define L0 "40000000000000000000000")
(define L1 "90000000000000000000000")
(define Q0 "40201005025125628140704")
(define Q1 "90452261306532663316583")

(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (fail m) (near/log m) (near/abort m))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (mdiv a b d) (u128/muldiv a b d))
(define (ult a b) (u128/lt a b))

;; one step across the cell starting at sqp: net liq l, gross book q, bound
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
      (sput "s:sqp" p2))))

(define (walk)
  (let* ((sqp (sget "s:sqp" "0"))
         (y (sget "s:y" "0"))
         (out (sget "s:out" "0")))
    (if (u128/eq y "0")
        "done"
        (if (ult sqp G2)
            (if (ult sqp G1) (if (ult y Q0) "done" (begin (step sqp y L0 Q0 G1 out) (walk))) (if (ult y Q1) "done" (begin (step sqp y L1 Q1 G2 out) (walk))))
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
(export "get-paid" get-paid)