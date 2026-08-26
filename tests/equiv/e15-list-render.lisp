;; e15 — list PRINT rendering parity: quoted strings, nested arrays,
;; bools, zeros (the zero fast-path bug class), separators
(define (main)
  (println (list 1 2 3))
  (println (list 0 0))
  (println (list -5 7))
  (println (list "x" "" "z"))
  (println (list true false))
  (println (list (list 1) (list 2 3))))
