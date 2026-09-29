;; P2 component test: BIP-340 sign + verify through the stitched Rust lib.
;; Input JSON: {} (uses fixed official vector inputs)
;; Output: "MATCH=YES VERIFY=VALID" on stdout (P2 has no near/log).
(define (run input)
  (let* ((sk (hex-decode "0000000000000000000000000000000000000000000000000000000000000003"))
         (msg (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (sig (schnorr-sign sk msg aux))
         (sighex (hex-encode sig))
         (pk (hex-decode "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9"))
         (ok (schnorr-verify pk sig msg))
         (expected "e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0"))
    (str-cat (if (= sighex expected) "MATCH=YES" "MATCH=NO")
             (if (= ok 1) " VERIFY=VALID" " VERIFY=INVALID"))))
