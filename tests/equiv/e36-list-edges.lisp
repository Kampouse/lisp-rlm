;; e36 — empty-list accessors (wasm-fuzz find, 2026-08-27)
;; wasm_emit car/cdr read past the count word of an allocated-but-empty
;; list header: cdr's count-1 underflow ran the copy loop past the heap
;; (OOB fault at exactly-full heap, silent garbage with headroom); car
;; read arr+8 (silent garbage). Both now guard count==0 → nil, matching
;; the interpreter.
(define (main)
  (println (car (list)))
  (println (cdr (list)))
  (println (car (list 1)))
  (println (cdr (list 1)))
  (println (car (list "a" "b")))
  (println (cdr (list "a" "b")))
  (println (cdr (cdr (list 1 2))))
  (println (car (cdr (list 1 2))))
  ;; len of empty (count word path, no read)
  (println (len (list)))
  ;; let-form regression (fuzzer find #1): canonical pair-list let works
  ;; on both surfaces — wasm also accepts flat/vec single-binding, interp
  ;; does not (surface asymmetry, tracked separately)
  (println (let ((x 1) (y 2)) (+ x y)))
)
