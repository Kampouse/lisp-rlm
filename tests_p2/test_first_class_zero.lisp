;; Test calling lambda stored in local variable with zero args

(define (run input)
  (let ((f (lambda () 42)))
    (f)))

;; Expected: 42