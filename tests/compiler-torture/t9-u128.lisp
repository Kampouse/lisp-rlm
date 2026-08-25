;; T9 — u128 builtins (string-based decimal values, NEAR yocto scale)
;; u128 values are decimal STRINGS in lisp land (matches NEAR's JSON API).

;; add
(println (u128/add "0" "0"))                                     ; "0"
(println (u128/add "5" "6"))                                     ; "11"
(println (u128/add "340282366920938463463374607431768211455" "0")) ; u128::MAX

;; mul — NOTE: 2^64 squared = 2^128 = u128::MAX + 1, which OVERFLOWS u128
;; (that case is an error test in t9b-u128-err-overflow-mul.lisp).
;; Positive cases: 2^32 squared = 2^64, and 1e18 * 1e6 = 1e24 (yocto scale).
(println (u128/mul "4294967296" "4294967296"))                     ; "18446744073709551616" (2^64)
(println (u128/mul "1000000000000000000" "1000000"))               ; "1000000000000000000000000" (1e24)

;; sub: 1e24 - 1 (yocto scale)
(println (u128/sub "1000000000000000000000000000" "1"))           ; "999999999999999999999999999"

;; div: u128::MAX / 2 = 2^127 - 1
(println (u128/div "340282366920938463463374607431768211455" "2")) ; "170141183460469231731687303715884105727"

;; mod
(println (u128/mod "10" "3"))                                     ; "1"

;; comparisons
(println (u128/lt "1" "2"))                                       ; true
(println (u128/gt "2" "1"))                                       ; true
(println (u128/eq "42" "42"))                                     ; true
(println (u128/eq "42" "43"))                                     ; false

;; i64 conversion
(println (u128/from-i64 9223372036854775807))                     ; "9223372036854775807"
(println (u128/to-i64 "9223372036854775807"))                     ; 9223372036854775807

;; is-zero
(println (u128/is-zero "0"))                                      ; true
(println (u128/is-zero "1"))                                      ; false

;; ── mini contract pattern: balance accounting at yocto scale ──
;; deposit 3e24 twice, withdraw 1e24, remaining must be 5e24.
(define (deposit bal amt) (u128/add bal amt))
(define (withdraw bal amt) (u128/sub bal amt))
(define bal-after-deposit-1 (deposit "0" "3000000000000000000000000000"))
(define bal-after-deposit-2 (deposit bal-after-deposit-1 "3000000000000000000000000000"))
(define bal-final (withdraw bal-after-deposit-2 "1000000000000000000000000000"))
(println bal-final)                                               ; "5000000000000000000000000000"
(println (u128/eq bal-final "5000000000000000000000000000"))      ; true
(println (u128/gt bal-final "4999999999999999999999999999"))      ; true
(println (u128/is-zero bal-final))                                ; false
