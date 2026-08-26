;; Test calling lambda stored in local variable
;; This tests the fix for: (let ((f (lambda (x) (+ x 1)))) (f 5))

(define (run input)
  (let ((f (lambda (x) (+ x 1))))
    (f 5)))

;; Expected: 6