;; Just return false
(define (test-false-only)
  false)
(export "test-false-only" test-false-only)

;; Just return true
(define (test-true-only)
  true)
(export "test-true-only" test-true-only)

;; Return string directly
(define (test-no-string)
  "no")
(export "test-no-string" test-no-string)