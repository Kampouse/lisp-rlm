;; error: u128/mul overflow — 2^64 squared = 2^128 = u128::MAX + 1
;; (the TASK's original positive expectation was arithmetically impossible:
;;  u128 tops out at 2^128 - 1, so checked_mul must reject this)
(println (u128/mul "18446744073709551616" "18446744073709551616"))
