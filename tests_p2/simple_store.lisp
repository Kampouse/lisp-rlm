(define (test)
  (near/store_u128 "key" (near/attached_deposit_u128)))
(export "test" test)
