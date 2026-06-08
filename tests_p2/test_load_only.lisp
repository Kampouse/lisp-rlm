;; Test near/load_u128 only (needs store first)
(define (test_load_u128_only)
  ;; Assume key exists - just call load
  (near/load_u128 "test_key"))

(export "test_load_u128_only" test_load_u128_only)