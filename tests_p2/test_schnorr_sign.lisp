;; End-to-end Rust-layer BIP-340 sign+verify through the stitched wasm lib.
;; Official BIP-340 vector 0: sk=3, msg=0^32, aux=0^32.
;; Expected sig: e907831f...0536c0, pk: F9308A01...CE036F9 (3G).
(define (run input)
  (let* ((sk (hex-decode "0000000000000000000000000000000000000000000000000000000000000003"))
         (msg (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (sig (schnorr-sign sk msg aux))
         (sighex (hex-encode sig))
         (pk (hex-decode "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9"))
         (ok (schnorr-verify pk sig msg))
         (expected "e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0"))
    (begin
      (near/log (str-cat "SIG=" sighex))
      (near/log (str-cat "MATCH=" (if (= sighex expected) "YES" "NO")))
      (near/log (str-cat "VERIFY=" (if (= ok 1) "VALID" "INVALID")))
      (if (= ok 1) (if (= sighex expected) "ALL-GOOD" "SIG-MISMATCH") "VERIFY-FAILED"))))
(export "run" run)
