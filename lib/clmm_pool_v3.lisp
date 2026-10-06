;; lib/clmm_pool_v3.lisp — v2 + LP shares (A-side) over the canonical leg.
;; New storage: "TOKA" → token-A account (init), "PA" → LP total A claim,
;; "SHT" → total shares, "SH:<a>" → per-LP shares.
;; Economics: every A entering via swap (used = amt - rem, fees included in
;; used) accrues to PA; shares mint only on add_liq → A-per-share rises with
;; volume. Withdraw burns shares, pays A out via detached promise on TOKA.
;; Invariants: pool token-A balance ≥ PA always (swaps never remove A;
;; withdrawals remove exactly what PA decrements). Share price = PA/SHT.

(define (bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (sh-key a) (str-cat "SH:" a))

(define (pool-init3)
  ;; op=init {"toka":..,"tokb":..} → v2 init + bind token-A + zero LP state
  (begin (pool-init)
         (near/store-bytes "TOKB" (json-get-str "tokb" (near/input)))
         (near/store-bytes "TOKA" (json-get-str "toka" (near/input)))
         (near/store-bytes "PA" "0")
         (near/store-bytes "SHT" "0")
         "ok"))

(define (mint-shares who amt)
  ;; sh = SHT==0 ? amt : amt*SHT//PA ; floor, zero-share deposit rejected
  (let ((sht (bz "SHT")) (pa (bz "PA")))
    (let ((sh (if (str= sht "0")
                  amt
                  (u128-muldiv amt sht pa))))
      (if (str= sh "0")
          "zero-shares"
          (begin
            (near/store-bytes "SHT" (li-add sht sh))
            (near/store-bytes (sh-key who) (li-add (bz (sh-key who)) sh))
            (near/store-bytes "PA" (li-add pa amt))
            (str-cat "sh:" sh))))))

(define (burn-shares who sh)
  ;; out = sh*PA//SHT ; burn shares, decrement PA, return payout amount
  (let ((pa (bz "PA")) (sht (bz "SHT")))
    (let ((out (u128-muldiv sh pa sht)))
      (near/store-bytes "SHT" (li-sub sht sh))
      (near/store-bytes (sh-key who) (li-sub (bz (sh-key who)) sh))
      (near/store-bytes "PA" (li-sub pa out))
      out)))

(define (ft-on-transfer)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((msg (json-get-str "msg" inp)))
        (let ((amt (json-get-str "amount" inp)))
          (if (str= msg "add_liq")
              (begin (mint-shares sender amt)
                     (near/return "0"))
              (begin
                (pool-swap amt)
                (let ((dy (bz "DY")) (rem (bz "REM")))
                  ;; swap income accrues to LPs: used = amt - rem
                  (near/store-bytes "PA" (li-add (bz "PA") (li-sub amt rem)))
                  (if (str= dy "0")
                      (near/return rem)
                      (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                        (begin
                          (near/promise_batch_action_function_call p "ft_transfer"
                            (json-set (json-set "{}" "receiver_id" sender) "amount" dy)
                            "0" 40000000000000)
                          (near/promise_return
                            (near/promise_then p (near/current_account_id) "pay_out"
                              (json-set "{}" "r" rem) "0" 5000000000000)))))))))))))

(define (pay-out)
  (near/return (json-get-str "r" (near/input))))

(define (withdraw)
  ;; op=withdraw {"sh":N} — caller = predecessor; pays A via detached promise
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((sh (json-get-str "sh" inp)))
        (if (< (li-cmp (bz (sh-key who)) sh) 0)
            (str-cat "insufficient-shares-have:" (bz (sh-key who)))
            (let ((out (burn-shares who sh)))
              (let ((p (near/promise_batch_create (near/load-bytes "TOKA"))))
                (begin
                  (near/promise_batch_action_function_call p "ft_transfer"
                    (json-set (json-set "{}" "receiver_id" who) "amount" out)
                    "0" 40000000000000)
                  (str-cat "out:" out)))))))))

(export "ft_on_transfer" ft-on-transfer)
(export "pay_out" pay-out)
