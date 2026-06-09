(define (test)
  (u128/store 64 1000 0)
  (u128/to_str 64 80))
(export "test" test)