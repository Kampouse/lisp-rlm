;; Test filter with lambda

(define (run input)
  (let ((nums (list 1 2 3 4 5)))
    (filter (lambda (x) (> x 3)) nums)))

;; Expected: (4 5)