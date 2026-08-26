;; e12 — arith type errors (round-3 fix 4) — both should error
(define (main)
  (println (try (+ "a" 1) (catch e "err-type")))
  (println (try (+ nil 5) (catch e "err-type-2"))))
