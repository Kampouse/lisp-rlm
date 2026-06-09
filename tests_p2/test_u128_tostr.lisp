; Test u128 to string conversion
(define (test_small)
  (u128/new 0 42 64)
  (u128/to_str 64 200))

(define (test_large)
  ; 10^21 = 1000000000000000000000
  ; lo part: 0xA7640000 = 2810552320
  ; hi part: 0xDE0B6B = 1456811
  (u128/new 1456811 2810552320 64)
  (u128/to_str 64 200))

(export "test_small" test_small)
(export "test_large" test_large)
