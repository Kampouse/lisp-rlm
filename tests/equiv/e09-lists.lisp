;; e09 — lists + higher-order fns
(define (main)
  (println (car (list 1 2 3)))
  (println (cdr (list 1 2 3)))
  (println (len (list 1 2 3)))
  (println (map (lambda (x) (* x x)) (list 1 2 3))))
