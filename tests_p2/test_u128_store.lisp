(define (test-u128-roundtrip)
  (let ((buf-addr 800))
    (u128/store 64 1000000000000000000000 0)  ; Store 10^21 at address 64
    (let ((lo (near/load_u64 64)))
      (let ((hi (near/load_u64 72)))
        (near/return_str (str-concat
          "lo=" (to-string lo)
          ", hi=" (to-string hi)))))))

(export "test_u128_roundtrip" test-u128-roundtrip)