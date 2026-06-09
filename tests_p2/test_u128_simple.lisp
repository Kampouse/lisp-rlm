(define (test)
  ;; Test u128 with value 1000 = 0x3E8 = lo=1000, hi=0
  (u128/store 64 1000 0)
  (u128/to_str 64 80))
(export "test" test)