;; T4 — capture semantics: closures over mutable state, independence
;; (flipped round-3 fix 2, 2026-08-26: per-invocation cells)
(define (make-counter)
  (let ((n 0))
    (lambda () (set! n (+ n 1)) n)))
(define c1 (make-counter))
(define c2 (make-counter))
(println (c1))  ; 1
(println (c1))  ; 2
(println (c2))  ; 1 — must NOT see c1's state
(println (c1))  ; 3

;; Siblings from ONE invocation share the same binding (correct let semantics)
(define (make-pair)
  (let ((n 0))
    (list (lambda () (set! n (+ n 1)) n)
          (lambda () n))))
(define p (make-pair))
(define inc (car p))
(define get (car (cdr p)))
(println (inc)) ; 1
(println (inc)) ; 2
(println (get)) ; 2 — sibling sees inc's writes (same cell, one invocation)
(define p2 (make-pair))
(println ((car (cdr p2)))) ; 0 — fresh invocation, fresh binding
