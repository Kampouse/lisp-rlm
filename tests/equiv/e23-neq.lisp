;; e23 — != structural inequality (was silent always-false on wasm)
(define (main)
  (println (!= 1 2))                                    ; true  (num)
  (println (!= 1 1))                                    ; false
  (println (!= "a" "b"))                                ; true  (str)
  (println (!= "a" "a"))                                ; false
  (println (!= (list 1 2) (list 1 2)))                  ; false (structural!)
  (println (!= (list 1 2) (list 1 3)))                  ; true
  (println (not (= "a" "b")))                           ; true  (not/= composition)
  (println (= (str-concat "a" "b") (str-concat "a" "b")))    ; true  (dynamic str)
  (println (!= (str-concat "a" "b") (str-concat "a" "c"))))  ; true
