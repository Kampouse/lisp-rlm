;; Simple test for near/transfer - amount must be a u128 pointer
;; First store a u128, then transfer it
(define (test_transfer)
  (let ((acct "test.near"))
    (near/store_u128 "test_deposit" (near/attached_deposit_u128))
    (near/transfer acct (near/load_u128 "test_deposit"))))

(export "test_transfer" test_transfer)