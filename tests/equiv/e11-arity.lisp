;; e11 — arity errors (round-3 fix 3) — both should error
(define (f2 a b) (+ a b))
(define (main)
  (println (try (f2 1) (catch e "err-missing")))
  (println (try (f2 1 2 3) (catch e "err-extra"))))
