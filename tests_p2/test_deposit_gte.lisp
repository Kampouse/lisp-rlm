(define (check)
  (if (near/deposit-gte 1000000000000000000)
    1
    0))
(export "check" check)
