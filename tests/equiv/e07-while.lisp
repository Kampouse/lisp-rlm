;; e07 — while + accumulator on LET vars (wasm contract model has no
;; top-level program globals; let+set! is the portable form)
(define (main)
  (let ((i 0) (acc 0))
    (while (< i 5)
      (set! acc (+ acc i))
      (set! i (+ i 1)))
    (println acc)))
