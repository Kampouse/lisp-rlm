;; Test booleans and if
(define (test-bool-false)
  (if false
      "yes"
    "no"))
(export "test-bool-false" test-bool-false)

(define (test-bool-true)
  (if true
      "yes"
    "no"))
(export "test-bool-true" test-bool-true)

(define (test-deposit-if)
  (if (near/deposit-gte 0)
      "gte"
    "lt"))
(export "test-deposit-if" test-deposit-if)