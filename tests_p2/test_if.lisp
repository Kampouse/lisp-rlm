;; Test 3: If/cond
(define (main)
  (print (if (> 5 3) 1 0))
  (print (if (< 5 3) 1 0))
  (print (if (= 42 42) 1 0)))
;; Expected: 1 0 1
