;; Test all u128 builtins
(define (test-all)
  ;; Store attached deposit
  (near/store_u128 "balance" (near/attached_deposit_u128))
  ;; Load it back  
  (let ((bal (near/load_u128 "balance")))
    (near/transfer "recipient.near" bal)))

(export "test-all" test-all)