;; e13 — nested data + equality
(define (main)
  (println (= (list 1 (list 2 3)) (list 1 (list 2 3))))
  (println (= (list 1 2) (list 1 2 3)))
  (println nil))
