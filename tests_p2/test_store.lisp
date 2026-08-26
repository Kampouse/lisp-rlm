;; Test near/store_u128 only
(define (test_store_u128)
  (near/store_u128 "test_key" (near/attached_deposit_u128)))

(export "test_store_u128" test_store_u128)