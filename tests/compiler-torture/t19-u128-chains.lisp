;; T19 — u128 arithmetic chains 10+ ops deep with NEAR storage roundtrip
;;
;; Ground rules: (2) the chain values were computed with python3 integer
;; arithmetic (exact); u128 div truncates toward zero like python3 //.
;; Chain: 10^18 → +1 → *2 → -2 → /3 → +334 → *3 → -1000 → /7 → +286 → *7
;; → -2000, then the final value goes through near/store → near/load →
;; +5 → /3 → u128/to-i64. The storage mock must round-trip the string
;; unchanged (erc20-in-miniature: balance → storage → balance → arithmetic).

(define x0 "1000000000000000000")
(define x1 (u128/add x0 "1"))                      ; 1000000000000000001
(define x2 (u128/mul x1 "2"))                      ; 2000000000000000002
(define x3 (u128/sub x2 "2"))                      ; 2000000000000000000
(define x4 (u128/div x3 "3"))                      ; 666666666666666666
(define x5 (u128/add x4 "334"))                    ; 666666666666667000
(define x6 (u128/mul x5 "3"))                      ; 2000000000000001000
(define x7 (u128/sub x6 "1000"))                   ; 2000000000000000000
(define x8 (u128/div x7 "7"))                      ; 285714285714285714
(define x9 (u128/add x8 "286"))                    ; 285714285714286000
(define x10 (u128/mul x9 "7"))                     ; 2000000000000002000
(define x11 (u128/sub x10 "2000"))                 ; 2000000000000000000

(println x1)
(println x4)
(println x8)
(println x11)                                       ; chain closes on 2*10^18 exactly

;; storage roundtrip + continued arithmetic on the loaded value
(near/store "balance" x11)
(define loaded (near/load "balance"))
(println loaded)                                    ; "2000000000000000000"
(define bumped (u128/add loaded "5"))
(println bumped)                                    ; "2000000000000000005"
(println (u128/to-i64 "9223372036854775807"))       ; exact i64::MAX fit
(println (u128/lt x11 bumped))                      ; true  — x11 < x11+5
(println (u128/gt bumped loaded))                   ; true — loaded+5 > loaded
(println (u128/eq loaded x11))                      ; true
(println (u128/is-zero "0"))                        ; true
