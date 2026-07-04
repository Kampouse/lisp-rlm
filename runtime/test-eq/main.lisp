(define (eq-test a b)
  (= a b))

(define (run input)
  (if (eq-test input "hello")
    "matched"
    "no-match"))
