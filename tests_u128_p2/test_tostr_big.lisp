; Test u128 to_str with large value (hi > 0)
; 10^21 = 1000000000000000000000
(define (test_big)
  (u128/new 1456811 2810552320 64)  ; 10^21
  (u128/to_str 64 200))

(define (test_small)
  (u128/new 0 42 64)
  (u128/to_str 64 200))

(export "test_big" test_big)
(export "test_small" test_small)
