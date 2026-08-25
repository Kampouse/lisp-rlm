;; T5 — numeric tower edges
(println (+ 9223372036854775807 0))    ; i64 max unchanged
(println (* 3037000499 3037000499))    ; 9223372030926249001 — largest safe square
(println (+ 1.5 1))                    ; 2.5 — f64/i64 mix
(println (/ 7 2))                      ; integer div? 3 — document behavior
(println (/ 7.0 2))                    ; 3.5
(println (- 0 ( - 0 5)))               ; 5
