;; e05 — let + set!
(define (main)
  (let ((x 1) (y 2))
    (println (+ x y))
    (set! x 10)
    (println (+ x y))))
