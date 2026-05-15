;; Test 5: Multiple functions
(define (square x) (* x x))
(define (sum-of-squares a b) (+ (square a) (square b)))
(define (main)
  (print (sum-of-squares 3 4)))
;; Expected: 25
