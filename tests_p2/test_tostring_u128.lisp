;; Test: to-string with u128 value from attached_deposit_u128
;; Use let* to keep everything in __toplevel scope
;; near-mock: ./target/release/near-mock tests_p2/test_tostring_u128.wasm _run '{}' --deposit 2000000000000000000

(let* ((dep (near/attached_deposit_u128))
       (s (to-string dep)))
  (near/log s)
  s)
