;; e06 — closures (T4 semantics). WASM surface gap: lambdas as values
;; unsupported (local: "unknown function f"; top-level define: reads as nil).
;; Expected: WASM_CERR until closure support lands; interp asserts T4 cells.
(define (mk) (let ((n 0)) (lambda () (set! n (+ n 1)) n)))
(define (main)
  (let ((c1 (mk)) (c2 (mk)))
    (println (c1))
    (println (c1))
    (println (c2))
    (println (c1))))
