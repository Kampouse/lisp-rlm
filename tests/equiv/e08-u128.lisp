;; e08 — u128 string numerics
(define (main)
  (println (u128/add "5" "6"))
  (println (u128/mul "123" "456"))
  (println (u128/lt "5" "6"))
  (println (u128/is-zero "0")))
