;; Test 9: While loop
(define (main)
  (let ((i 0) (sum 0))
    (while (< i 10)
      (set! sum (+ sum i))
      (set! i (+ i 1)))
    (print sum)))
;; Expected: 45
