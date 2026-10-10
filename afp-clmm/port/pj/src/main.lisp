;; pool_fee_join P1 P2 (AFP CLMM_Transformation) — joint pool = pool_fee_join(pa, pc)
;; P1 = pa (phi 3e15/1e18) + P2 = pc (5e15/1e18), joint on grid GA.
;; gross_fct additivity holds EXACTLY for heterogeneous fees (join_gross_fct):
;;   QBK{i} = QA{i} + QC{i}  (integer add, no rounding)
;; fee P i = fee_union l1 l2 f1 f2 = (l1 f1 (1-f2) + l2 f2 (1-f1)) /
;;   (l1 (1-f2) + l2 (1-f1)) — fee-scale (FS=1e15) nested floors, the ONLY
;;   rounded quantity here; get-fee exports the per-cell Fhat.
(define G0 "1000000000")
(define G1 "2000000000")
(define G2 "4000000000")
(define FS "1000000000000000")
(define LK0 "140000000000000000000000")
(define LK1 "150000000000000000000000")
(define QBK0 "140501907733250001260063")
(define QBK1 "150632802931407287188198")
(define Fh0 "3572248529200")
(define Fh1 "4200963661915")
(define Fd0 "1000000000000000")
(define Fd1 "1000000000000000")
(define (fee0) (mdiv Fh0 FS Fd0))
(define (fee1) (mdiv Fh1 FS Fd1))

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

(define (get-fee)
  (near/return (fee0)))

(export "get-fee" get-fee)

(define (walk)
  (let* ((sqp (sget "s:sqp" "0"))
         (y (sget "s:y" "0"))
         (out (sget "s:out" "0")))
    (if (u128/eq y "0")
        "done"
        (if (ult sqp G2)
            (if (ult sqp G1) (if (ult y QBK0) "done" (begin (step sqp y LK0 QBK0 G1 out) (walk))) (if (ult y QBK1) "done" (begin (step sqp y LK1 QBK1 G2 out) (walk))))
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