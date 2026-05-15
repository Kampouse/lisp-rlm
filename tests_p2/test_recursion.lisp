;; Test 6: Recursion (factorial)
(define (fact n)
  (if (= n 0) 1 (* n (fact (- n 1)))))
(define (main)
  (print (fact 10)))
;; Expected: 3628800
