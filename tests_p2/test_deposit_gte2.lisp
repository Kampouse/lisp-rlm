(define (check)
  ;; Test deposit-gte, store result, return it
  (let ((result (near/deposit-gte 1000000000000000000)))
    result))
(export "check" check)
