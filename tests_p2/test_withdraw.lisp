;; Simplified withdraw test
(define (test-withdraw)
  ;; Check deposit threshold
  (if (near/deposit-gte 1)
      "enough deposit"
    "need more deposit"))

(export "test-withdraw" test-withdraw)