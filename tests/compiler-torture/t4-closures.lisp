;; T4 — capture semantics: closures over mutable state, independence
(define (make-counter)
  (let ((n 0))
    (lambda () (set! n (+ n 1)) n)))
(define c1 (make-counter))
(define c2 (make-counter))
(println (c1))  ; 1
(println (c1))  ; 2
(println (c2))  ; 1 — must NOT see c1's state
(println (c1))  ; 3
