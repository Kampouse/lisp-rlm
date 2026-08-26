(define (test)
  ;; Store u128 at address 64: lo=3875820019684212736, hi=54 (10^21)
  ;; u128/store takes: addr_lo, lo_val, hi_val
  (u128/store 64 3875820019684212736 54)
  ;; Convert to string
  (u128/to_str 64 80))
(export "test" test)