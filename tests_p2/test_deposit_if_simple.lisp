(define (test-deposit-if)
  (if (near/deposit-gte 0)
      "gte"
    "lt"))
(export "test-deposit-if" test-deposit-if)