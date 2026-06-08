;; Test near/load_u128
(define (test_load_u128)
  (near/store_u128 "test_key" (near/attached_deposit_u128))
  (near/load_u128 "test_key"))

(export "test_load_u128" test_load_u128)