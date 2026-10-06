;; lib/clmm_pool_v2.lisp — NEP-141 receiver leg on the stateful pool.
;; New storage: "TOKB" → token-B account (set by init), "DY"/"REM" → last
;; swap results (avoids string-splitting ft_on_transfer), "DEP:<a>" LP
;; credits (add-liquidity, recorded for v3 shares).
;; Entries: run (op=init|swap|state|md as v1) · ft_on_transfer · pay_out.
;; ft_on_transfer {"sender_id","amount","msg"}:
;;   msg="swap" → ladder swap, pay B out via promise (ft_transfer on
;;   TOKB), final callback pay_out returns unused-A refund string; pool
;;   drains/decrements persisted as v1.
;;   dy=0 → plain return of the refund (no promise needed).

(define (bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (dep-key a) (str-cat "DEP:" a))

(define (pool-init2)
  ;; op=init with optional "tokb" → also bind token-B account
  (begin (pool-init)
         (near/store-bytes "TOKB" (json-get-str "tokb" (near/input)))
         "ok"))

(define (ft-on-transfer)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (if (str= (json-get-str "msg" inp) "add")
          (begin (near/store-bytes (dep-key sender)
                   (li-add (bz (dep-key sender)) (json-get-str "amount" inp)))
                 (near/return "0"))
          (begin
            (pool-swap (json-get-str "amount" inp))
            (let ((dy (bz "DY")) (rem (bz "REM")))
              (if (str= dy "0")
                  (near/return rem)
                  (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                    (begin
                      (near/promise_batch_action_function_call p "ft_transfer"
                        (json-set (json-set "{}" "receiver_id" sender) "amount" dy)
                        "0" 40000000000000)
                      (near/promise_return
                        (near/promise_then p (near/current_account_id) "pay_out"
                          (json-set "{}" "r" rem) "0" 5000000000000)))))))))))

(define (pay-out)
  (near/return (json-get-str "r" (near/input))))

(export "ft_on_transfer" ft-on-transfer)
(export "pay_out" pay-out)
