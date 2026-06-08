(define (test-false)
  (if (near/deposit-gte 1)
      "yes"
    "no"))
(export "test-false" test-false)

(define (test-true)
  ;; Set attached_deposit to > 0 to test true case
  ;; But mock always returns 0, so this will still be false
  (if (near/deposit-gte 0)
      "yes"
    "no"))
(export "test-true" test-true)